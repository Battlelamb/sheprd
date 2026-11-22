//! Joining a fleet from a computer in none (`docs/relay-plan.md`, Fleet,
//! Joining): while this computer is in no fleet, each computer it reaches
//! over SSH is asked which fleet it is a member of, and Settings > Fleet
//! offers to join it. Joining runs both halves from here, as an invitation
//! from a member does from there: this computer's daemon signs its bundle
//! for that fleet, the member's daemon admits it over SSH (`fleet admit`),
//! and the member's log is carried back here one answer at a time.
//!
//! Reaching the member's account over SSH is what proves the person
//! controls it, as it is for an invitation, so only a machine whose host
//! key is checked strictly is offered: joining lets the fleet's computers
//! in here.

use super::{
    Error, Fleet, Link, Machine, Standing,
    flows::{MAX_PARTS, applied, daemon},
    plain::strict_target,
    remote,
};
use crate::remote::Supervisor;
use gpui::{App, BorrowAppContext as _};
use sheprd_remote_control::control::fleet::{
    View,
    sync::{Admission, Answer, Apply, Ask},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::AtomicBool,
    time::Instant,
};

/// A fleet a reached computer is a member of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Offer {
    /// Its id, hex.
    pub(crate) fleet: String,
    /// How many members and hosts it has.
    pub(crate) members: usize,
    pub(crate) hosts: usize,
}

impl Offer {
    /// What `view`, a reached computer's fleet, offers: nothing unless that
    /// computer is a member of it, as a shared host cannot admit anyone.
    pub(crate) fn of(view: &View) -> Option<Self> {
        view.members
            .iter()
            .any(|member| member.id == view.me)
            .then(|| Self {
                fleet: view.fleet.clone(),
                members: view.members.len(),
                hosts: view.hosts.len(),
            })
    }
}

impl Fleet {
    /// The fleets this computer, in none, may join: one machine for each,
    /// the first by id that offers it.
    pub(crate) fn offers(cx: &App) -> Vec<(String, Machine, Offer)> {
        cx.try_global::<Self>()
            .map(|this| this.offered(this.reached(Instant::now()), true))
            .unwrap_or_default()
    }

    /// Every reached machine that offers a fleet, several for one fleet
    /// included: what `sheprd fleet join MACHINE` may name.
    pub(crate) fn all_offers(cx: &App) -> Vec<(String, Machine, Offer)> {
        cx.try_global::<Self>()
            .map(|this| this.offered(this.reached(Instant::now()), false))
            .unwrap_or_default()
    }

    /// The machines among `reached` that offer a fleet, the first for each
    /// fleet alone when `once`.
    pub(super) fn offered(
        &self,
        reached: BTreeMap<String, Machine>,
        once: bool,
    ) -> Vec<(String, Machine, Offer)> {
        if self.view.is_some() {
            return Vec::new();
        }
        let mut seen = BTreeSet::new();
        reached
            .into_iter()
            .filter_map(|(id, machine)| {
                let offer = self.links.get(&id)?.offer.clone()?;
                (seen.insert(offer.fleet.clone()) || !once).then_some((id, machine, offer))
            })
            .collect()
    }

    /// Asks every reached machine again, as Settings > Fleet opens, so a
    /// fleet founded elsewhere since shows without waiting.
    pub(crate) fn rediscover(cx: &mut App) {
        let Some(this) = cx.try_global::<Self>() else {
            return;
        };
        if this.view.is_some() {
            return;
        }
        cx.update_global::<Self, _>(|this, _| {
            for link in this.links.values_mut() {
                link.restart();
            }
        });
        Self::tick(cx);
    }

    /// Asks each reached machine that is due which fleet it is in.
    /// Only while Remote control runs here: without it, this computer
    /// could not join anyway.
    pub(super) fn discover_due(cx: &mut App, now: Instant) {
        if Supervisor::client(cx).is_none() {
            return;
        }
        let Some(this) = cx.try_global::<Self>() else {
            return;
        };
        let due: Vec<(String, Machine)> = this
            .reached(now)
            .into_iter()
            .filter(|(id, _)| {
                !this.acting.contains(id)
                    && this.links.get(id).is_none_or(|link| link.due(&[], now))
            })
            .collect();
        for (id, machine) in due {
            discover(cx, id, machine);
        }
    }
}

/// Asks `machine` which fleet it is a member of, for this computer in
/// none.
fn discover(cx: &mut App, id: String, machine: Machine) {
    Fleet::set_link(cx, &id, |link| {
        link.running = true;
        link.last = Some(Instant::now());
    });
    let task = cx.background_executor().spawn(async move {
        match strict_target(&machine.target) {
            Ok(_) => {}
            Err(Error::Unchecked) => return Ok((Standing::Unchecked, None)),
            Err(_) => return Ok((Standing::Outside, None)),
        }
        match remote::status(&machine.target, &AtomicBool::new(false)) {
            Ok(view) => Ok((Standing::Outside, view.as_ref().and_then(Offer::of))),
            Err(Error::NoProgram) => Ok((Standing::NoDaemon, None)),
            Err(error) => Err::<_, Error>(error),
        }
    });
    cx.spawn(async move |cx| {
        let outcome = task.await;
        cx.update(|cx| {
            Fleet::set_link(cx, &id, |link| {
                link.running = false;
                match outcome {
                    Ok((standing, offer)) => {
                        link.standing = standing;
                        link.offer = offer;
                    }
                    Err(error) => {
                        tracing::debug!(%error, "Could not ask a machine for its fleet");
                        link.standing = Standing::Failed;
                        link.offer = None;
                    }
                }
            });
            cx.refresh_windows();
        });
    })
    .detach();
}

/// Joins the fleet `machine` is a member of (a person here asked for it):
/// this computer signs its bundle for that fleet, the member admits it
/// over SSH, and the member's log is carried back here, which admits it.
pub(crate) fn join(
    cx: &mut App,
    id: String,
    machine: Machine,
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
            // Asked again, so the fleet joined is the one it is in now.
            let offer = executor
                .spawn(async move {
                    strict_target(&target)?;
                    remote::status(&target, &AtomicBool::new(false))
                })
                .await?
                .as_ref()
                .and_then(Offer::of)
                .ok_or(Error::NotMember)?;
            let fleet = offer.fleet;
            let admission = match daemon(
                &client,
                Ask::Bundle {
                    fleet: fleet.clone(),
                },
            )
            .await?
            {
                Answer::Bundle { bundle, signature } => Admission { bundle, signature },
                _ => return Err(Error::Unexpected),
            };
            let target = machine.target.clone();
            executor
                .spawn(async move { remote::admit(&target, &admission, &AtomicBool::new(false)) })
                .await?;
            take_log(&client, &executor, &machine, &fleet).await
        }
        .await;
        cx.update(|cx| {
            Fleet::set_acting(cx, &id, false);
            // In the fleet now: every machine is asked again, as a member.
            if outcome.is_ok() {
                cx.update_global::<Fleet, _>(|this, _| {
                    this.links.values_mut().for_each(Link::restart);
                });
            }
            Fleet::refresh(cx);
            done(outcome.err().map(|error| error.to_string()), cx);
        });
    })
    .detach();
}

/// Carries `fleet`'s log from `machine`, a member that just admitted this
/// computer, one answer at a time: this computer keeps the history as far
/// as it arrived, and joins once it holds its admission.
async fn take_log(
    client: &crate::remote::supervisor::Client,
    executor: &gpui::BackgroundExecutor,
    machine: &Machine,
    fleet: &str,
) -> Result<(), Error> {
    let mut ours = Vec::new();
    for _ in 0..MAX_PARTS {
        let ask = Apply {
            fleet: Some(fleet.to_owned()),
            heads: ours,
            ..Apply::default()
        };
        let target = machine.target.clone();
        let part = executor
            .spawn(async move { remote::apply(&target, &ask, &AtomicBool::new(false)) })
            .await?;
        if part.ops.is_empty() {
            return Ok(());
        }
        let more = part.more;
        let kept = applied(
            client,
            Apply {
                fleet: Some(fleet.to_owned()),
                heads: part.heads,
                ops: part.ops,
                ..Apply::default()
            },
        )
        .await?;
        if !more {
            return Ok(());
        }
        ours = kept.heads;
    }
    Err(Error::TooLarge)
}

/// Sets the fleet `machine` offers, as a discovery that just ran would.
#[cfg(test)]
pub(crate) fn set_offer(cx: &mut App, machine: &str, offer: Option<Offer>) {
    Fleet::set_link(cx, machine, |link| link.offer = offer);
}
