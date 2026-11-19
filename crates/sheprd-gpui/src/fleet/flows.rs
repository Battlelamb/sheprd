//! What Sheprd does with another machine: syncs the log with it, invites
//! it to join, or shares it as a host. Each asks this computer's daemon
//! for its part and runs the machine's part over SSH on the background
//! executor.

use super::{Error, Fleet, Machine, Standing, remote};
use crate::remote::Supervisor;
use gpui::App;
use sheprd_remote_control::control::{
    Request, Response,
    fleet::{
        Method,
        sync::{Answer, Applied, Apply, Ask},
    },
};
use std::{sync::atomic::AtomicBool, time::Instant};

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

/// Syncs `machine` with this computer's fleet `fleet`:
///
/// 1. asks the machine for what this computer lacks, naming the fleet and
///    this computer's heads, and sending nothing: a machine in no fleet or
///    another answers nothing and learns nothing;
/// 2. hands what it answered to this computer's daemon, which says what
///    the machine lacks;
/// 3. sends the machine that.
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
            let mine = match daemon(&client, Ask::Status).await? {
                Answer::Status { view: Some(view) } if view.fleet == fleet => view.heads.clone(),
                _ => return Ok((Standing::Outside, Vec::new(), Vec::new())),
            };
            let target = machine.target.clone();
            let ask = Apply {
                fleet: Some(fleet.clone()),
                heads: mine,
                ops: Vec::new(),
            };
            let theirs = executor
                .spawn(async move { remote::apply(&target, &ask, &AtomicBool::new(false)) })
                .await;
            let theirs = match theirs {
                Ok(theirs) if theirs.fleet == fleet => theirs,
                Ok(_) | Err(Error::NoProgram) => {
                    return Ok((Standing::Outside, Vec::new(), Vec::new()));
                }
                Err(error) => return Err(error),
            };
            let ours = applied(
                &client,
                Apply {
                    fleet: Some(fleet.clone()),
                    heads: theirs.heads.clone(),
                    ops: theirs.ops,
                },
            )
            .await?;
            let mut heads = theirs.heads;
            if !ours.ops.is_empty() {
                let target = machine.target.clone();
                let send = Apply {
                    fleet: Some(fleet.clone()),
                    heads: ours.heads.clone(),
                    ops: ours.ops,
                };
                let answer = executor
                    .spawn(async move { remote::apply(&target, &send, &AtomicBool::new(false)) })
                    .await?;
                heads = answer.heads;
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
            let everything = applied(
                &client,
                Apply {
                    fleet: Some(fleet.clone()),
                    heads: Vec::new(),
                    ops: Vec::new(),
                },
            )
            .await?;
            let target = machine.target.clone();
            let send = Apply {
                fleet: None,
                heads: Vec::new(),
                ops: everything.ops,
            };
            executor
                .spawn(async move { remote::apply(&target, &send, &AtomicBool::new(false)) })
                .await?;
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
/// certificate authority, and this computer's daemon adds it (a person
/// here asked for it).
pub(crate) fn share(
    cx: &mut App,
    id: String,
    machine: Machine,
    deny: Vec<String>,
    done: impl FnOnce(Option<String>, &mut App) + 'static,
) {
    let Some(client) = Supervisor::client(cx) else {
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
