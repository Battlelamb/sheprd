//! What Sheprd does with another machine: syncs the log with it, invites
//! it to join, or shares it as a host. Each asks this computer's daemon
//! for its part and runs the machine's part over SSH on the background
//! executor.

use super::{Error, Fleet, Machine, Standing, remote};
use crate::remote::Supervisor;
use gpui::{App, BackgroundExecutor};
use sheprd_client::ConnectTarget;
use sheprd_remote_control::control::{
    Request, Response,
    fleet::{
        HostKind, Method, saved,
        sync::{self, Answer, Applied, Apply, Ask},
    },
};
use std::{collections::BTreeSet, sync::atomic::AtomicBool, time::Instant};

/// Asks this computer's daemon `ask` and waits for its fleet answer.
async fn daemon(client: &crate::remote::supervisor::Client, ask: Ask) -> Result<Answer, Error> {
    match client.request(Request::Fleet(Box::new(ask))).await {
        Ok(Response::Fleet(answer)) => Ok(*answer),
        Ok(Response::Error { message, .. }) => Err(Error::Daemon(message)),
        Ok(_) => Err(Error::Daemon(String::new())),
        Err(error) => Err(Error::Request(error)),
    }
}

async fn applied(
    client: &crate::remote::supervisor::Client,
    apply: Apply,
) -> Result<Applied, Error> {
    match daemon(client, Ask::Apply(apply)).await? {
        Answer::Applied(applied) => Ok(applied),
        _ => Err(Error::Daemon(String::new())),
    }
}

/// The most parts a log is carried in: the largest log a daemon keeps,
/// one answer at a time, and one more.
const MAX_PARTS: usize = saved::MAX_SAVED.div_ceil(sync::MAX_ANSWER_BYTES) + 1;

/// Asks `target` for what this computer lacks, from `heads`, naming no
/// fleet and with a fresh nonce: its answer, when it proves it is one of
/// `hosts` (the fleet's computers and Sheprd hosts) in `fleet`, with the
/// host it proved to be; `None` for a machine outside it, which learns
/// nothing.
async fn ask_proven(
    executor: &BackgroundExecutor,
    target: &ConnectTarget,
    heads: Vec<String>,
    fleet: &str,
    hosts: &[String],
) -> Result<Option<(Applied, String)>, Error> {
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let ask = Apply {
        fleet: None,
        heads,
        nonce: Some(nonce.clone()),
        ..Apply::default()
    };
    let target = target.clone();
    match executor
        .spawn(async move { remote::apply(&target, &ask, &AtomicBool::new(false)) })
        .await
    {
        Ok(theirs) if theirs.fleet == fleet => {
            Ok(sync::proven_host(&theirs, &nonce, hosts).map(|host| (theirs, host)))
        }
        Ok(_) | Err(Error::NoProgram) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Hands this computer's whole log to `target`, which is in no fleet yet,
/// in parts of at most one answer each, every part but the last marked
/// `more`: the machine joins with the last, as the operation that admits
/// it, or shares it as a host, comes last.
async fn hand_over(
    client: &crate::remote::supervisor::Client,
    executor: &BackgroundExecutor,
    target: &ConnectTarget,
    fleet: &str,
) -> Result<(), Error> {
    let mut sent = Vec::new();
    for _ in 0..MAX_PARTS {
        let part = applied(
            client,
            Apply {
                fleet: Some(fleet.to_owned()),
                heads: sent.clone(),
                ..Apply::default()
            },
        )
        .await?;
        sent = tips(&sent, &part.ops)?;
        let more = part.more;
        // The fleet is named so the machine keeps this handover's parts
        // apart from any other's.
        let send = Apply {
            fleet: Some(fleet.to_owned()),
            ops: part.ops,
            more,
            ..Apply::default()
        };
        let target = target.clone();
        executor
            .spawn(async move { remote::apply(&target, &send, &AtomicBool::new(false)) })
            .await?;
        if !more {
            return Ok(());
        }
    }
    Err(Error::TooLarge)
}

/// The heads of what was sent, after `ops`, base64url, as [`heads_after`].
pub(super) fn tips(before: &[String], ops: &[String]) -> Result<Vec<String>, Error> {
    let ops = sync::decode_ops(ops)?;
    Ok(heads_after(
        before,
        ops.iter().map(|op| {
            (
                op.id().to_string(),
                op.parents().iter().map(ToString::to_string).collect(),
            )
        }),
    ))
}

/// The heads of what was sent: `before`'s and the ids of `ops` (each with
/// its parents), but for those `ops` name as parents. As the daemon
/// answers from the start of the log, what was sent holds every ancestor
/// of what it holds, so asking from these heads gives the rest.
pub(super) fn heads_after(
    before: &[String],
    ops: impl IntoIterator<Item = (String, Vec<String>)>,
) -> Vec<String> {
    let mut tips: BTreeSet<String> = before.iter().cloned().collect();
    let mut parents = BTreeSet::new();
    for (id, named) in ops {
        tips.insert(id);
        parents.extend(named);
    }
    tips.retain(|id| !parents.contains(id));
    tips.into_iter().collect()
}

/// Syncs `machine` with this computer's fleet `fleet`:
///
/// 1. asks the machine for what this computer lacks, naming no fleet,
///    sending only this computer's heads and a nonce it must sign with
///    its certificate authority: a machine that is not one of the fleet's
///    hosts learns nothing and gets nothing;
/// 2. hands what it answered to this computer's daemon, which says what
///    the machine lacks, asking again while the machine has more;
/// 3. sends the machine that, one answer at a time.
pub(super) fn sync(cx: &mut App, id: String, machine: Machine, fleet: String) {
    let Some(client) = Supervisor::client(cx) else {
        return;
    };
    Fleet::set_link(cx, &id, |link| {
        link.running = true;
        link.last = Some(Instant::now());
    });
    let executor = cx.background_executor().clone();
    cx.spawn(async move |cx| {
        let outcome = async {
            let view = match daemon(&client, Ask::Status).await? {
                Answer::Status { view: Some(view) } if view.fleet == fleet => view,
                _ => return Ok((Standing::Outside, Vec::new(), Vec::new())),
            };
            // Hosts that sign with a certificate authority: computers in
            // the fleet and Sheprd hosts. Only one of them may prove it is
            // the machine answering.
            let hosts: Vec<String> = view
                .hosts
                .iter()
                .filter(|host| host.kind != HostKind::Agent)
                .map(|host| host.id.clone())
                .collect();
            let target = &machine.target;
            let Some((mut theirs, proven)) =
                ask_proven(&executor, target, view.heads.clone(), &fleet, &hosts).await?
            else {
                return Ok((Standing::Outside, Vec::new(), Vec::new()));
            };
            let mut ours;
            let mut parts = 1;
            loop {
                ours = applied(
                    &client,
                    Apply {
                        fleet: Some(fleet.clone()),
                        heads: theirs.heads.clone(),
                        ops: std::mem::take(&mut theirs.ops),
                        ..Apply::default()
                    },
                )
                .await?;
                if !theirs.more || parts >= MAX_PARTS {
                    break;
                }
                parts += 1;
                let Some((next, again)) =
                    ask_proven(&executor, target, ours.heads.clone(), &fleet, &hosts).await?
                else {
                    return Ok((Standing::Outside, Vec::new(), Vec::new()));
                };
                if again != proven {
                    return Ok((Standing::Outside, Vec::new(), Vec::new()));
                }
                theirs = next;
            }
            let mut heads = theirs.heads;
            for _ in 0..MAX_PARTS {
                if ours.ops.is_empty() {
                    break;
                }
                let more = ours.more;
                let send = Apply {
                    fleet: Some(fleet.clone()),
                    heads: ours.heads.clone(),
                    ops: ours.ops,
                    ..Apply::default()
                };
                let target = target.clone();
                heads = executor
                    .spawn(async move { remote::apply(&target, &send, &AtomicBool::new(false)) })
                    .await?
                    .heads;
                if !more {
                    break;
                }
                ours = applied(
                    &client,
                    Apply {
                        fleet: Some(fleet.clone()),
                        heads: heads.clone(),
                        ..Apply::default()
                    },
                )
                .await?;
            }
            // A shared Sheprd host is no member and cannot publish its own
            // record when its relays move: this computer carries it.
            let shared = view
                .hosts
                .iter()
                .any(|host| host.id == proven && host.kind == HostKind::Sheprd);
            if shared {
                let target = target.clone();
                let seen = executor
                    .spawn(async move { remote::intro(&target, &AtomicBool::new(false)) })
                    .await;
                let carried = match seen {
                    Ok(intro) => daemon(&client, Ask::Seen { intro }).await.map(drop),
                    Err(error) => Err(error),
                };
                // The log itself synced: a record not carried now is
                // tried again on the next sync.
                if let Err(error) = carried {
                    tracing::debug!(%error, "Could not carry a shared host's record");
                }
            }
            Ok::<_, Error>((Standing::Member, heads, ours.heads))
        }
        .await;
        cx.update(|cx| {
            match outcome {
                Ok((standing, heads, synced)) => Fleet::set_link(cx, &id, |link| {
                    link.running = false;
                    link.standing = standing;
                    link.heads = heads;
                    link.synced = synced;
                }),
                Err(error) => {
                    tracing::debug!(%error, "Could not sync the fleet with a machine");
                    Fleet::set_link(cx, &id, |link| {
                        link.running = false;
                        link.standing = Standing::Failed;
                    });
                }
            }
            Fleet::refresh(cx);
        });
    })
    .detach();
}

/// Invites the computer `machine` to join: it signs its own bundle for the
/// fleet, this computer's daemon admits it (a person here asked for it),
/// and the whole log is handed over, which the machine takes only because
/// it admits it.
pub(crate) fn invite(
    cx: &mut App,
    id: String,
    machine: Machine,
    done: impl FnOnce(Option<String>, &mut App) + 'static,
) {
    let (Some(client), Some(fleet)) = (
        Supervisor::client(cx),
        Fleet::view(cx).map(|view| view.fleet.clone()),
    ) else {
        done(Some(Error::NoDaemon.to_string()), cx);
        return;
    };
    Fleet::set_acting(cx, &id, true);
    let executor = cx.background_executor().clone();
    cx.spawn(async move |cx| {
        let outcome = async {
            let target = machine.target.clone();
            let named = fleet.clone();
            let (bundle, signature) = executor
                .spawn(async move { remote::bundle(&target, &named, &AtomicBool::new(false)) })
                .await?;
            daemon(
                &client,
                Ask::Admit {
                    bundle,
                    signature,
                    method: Method::Ssh,
                },
            )
            .await?;
            hand_over(&client, &executor, &machine.target, &fleet).await?;
            Ok::<_, Error>(())
        }
        .await;
        cx.update(|cx| {
            Fleet::set_acting(cx, &id, false);
            Fleet::refresh(cx);
            done(outcome.err().map(|error| error.to_string()), cx);
        });
    })
    .detach();
}

/// Shares the Sheprd host `machine` with the fleet, leaving out the
/// computer members in `deny`: it signs its own record with its
/// certificate authority, this computer's daemon adds it (a person here
/// asked for it), and the whole log is handed over, which the host takes
/// only because it shares it.
pub(crate) fn share(
    cx: &mut App,
    id: String,
    machine: Machine,
    deny: Vec<String>,
    done: impl FnOnce(Option<String>, &mut App) + 'static,
) {
    let (Some(client), Some(fleet)) = (
        Supervisor::client(cx),
        Fleet::view(cx).map(|view| view.fleet.clone()),
    ) else {
        done(Some(Error::NoDaemon.to_string()), cx);
        return;
    };
    Fleet::set_acting(cx, &id, true);
    let executor = cx.background_executor().clone();
    cx.spawn(async move |cx| {
        let outcome = async {
            let target = machine.target.clone();
            let intro = executor
                .spawn(async move { remote::intro(&target, &AtomicBool::new(false)) })
                .await?;
            daemon(&client, Ask::AddHost { intro, deny }).await?;
            // The host takes the log because it shares it, then lets in
            // the computers allowed there; syncing keeps it current.
            hand_over(&client, &executor, &machine.target, &fleet).await?;
            Ok::<_, Error>(())
        }
        .await;
        cx.update(|cx| {
            Fleet::set_acting(cx, &id, false);
            Fleet::refresh(cx);
            done(outcome.err().map(|error| error.to_string()), cx);
        });
    })
    .detach();
}

/// Removes every way a fleet reaches `machine`'s account, over SSH: the
/// last resort for a host no member can reach or a fleet that is gone (a
/// person here asked for it). Signs nothing: it only takes access away.
pub(crate) fn scrub(
    cx: &mut App,
    id: String,
    machine: Machine,
    done: impl FnOnce(Option<String>, &mut App) + 'static,
) {
    Fleet::set_acting(cx, &id, true);
    let target = machine.target;
    let task = cx
        .background_executor()
        .spawn(async move { remote::scrub(&target, &AtomicBool::new(false)) });
    cx.spawn(async move |cx| {
        let outcome = task.await;
        cx.update(|cx| {
            Fleet::set_acting(cx, &id, false);
            Fleet::set_link(cx, &id, |link| *link = super::Link::default());
            Fleet::refresh(cx);
            done(outcome.err().map(|error| error.to_string()), cx);
        });
    })
    .detach();
}
