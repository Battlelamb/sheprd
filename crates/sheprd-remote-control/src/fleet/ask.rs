//! What Sheprd asks the daemon about the fleet (`control::fleet::sync::Ask`).
//! Every operation this computer signs is one a person here asked for in
//! Sheprd's own dialog; the daemon only signs, keeps, and passes them on.

use super::{Error, Fleet, Result, me};
use crate::control::fleet::{
    Access, Admit, Body, Bundle, HostIntro, Remove, View,
    sync::{self, Answer, Ask},
};
use crate::web::State;

/// Answers `ask` at `now_ms`. Blocking.
pub(crate) fn answer(state: &State, fleet: &Fleet, ask: Ask, now_ms: u64) -> Result<Answer> {
    match ask {
        Ask::Status => Ok(Answer::Status {
            view: view(fleet).map(Box::new),
        }),
        Ask::Found => {
            fleet.found(me::bundle(state, fleet, now_ms)?, now_ms)?;
            Ok(Answer::Done)
        }
        Ask::Bundle { fleet: id } => {
            if fleet.inner().log.is_some() {
                return Err(Error::InFleet);
            }
            let id = sync::fleet(&id)?;
            let bundle = me::bundle(state, fleet, now_ms)?;
            let message = bundle
                .join_message(&id)
                .ok_or(crate::control::fleet::Error::Invalid)?;
            let signature: p256::ecdsa::Signature =
                p256::ecdsa::signature::Signer::sign(fleet.key(), &message);
            Ok(Answer::Bundle {
                bundle: sync::text(&bundle.to_bytes()?),
                signature: sync::text(&signature.to_bytes()),
            })
        }
        Ask::Admit {
            bundle,
            signature,
            method,
        } => {
            let bundle = Bundle::from_bytes(&sync::bytes(&bundle)?)?;
            let member = bundle.id();
            fleet.joins().admissible(&member, &paired(state))?;
            let joiner_signature = sync::bytes(&signature)?
                .try_into()
                .map_err(|_| crate::control::fleet::Error::JoinSignature)?;
            fleet.sign(
                Body::Admit(Admit {
                    bundle,
                    method,
                    joiner_signature,
                }),
                now_ms,
            )?;
            fleet.joins().admitted(&member);
            Ok(Answer::Done)
        }
        Ask::Remove {
            member,
            also_members,
            also_hosts,
            reason,
        } => {
            let body = Body::Remove(Remove {
                member: sync::member(&member)?,
                also_members: also_members
                    .iter()
                    .map(|member| sync::member(member))
                    .collect::<crate::control::fleet::Result<_>>()?,
                also_hosts: also_hosts
                    .iter()
                    .map(|host| sync::host(host))
                    .collect::<crate::control::fleet::Result<_>>()?,
                reason,
            });
            fleet.sign(body, now_ms)?;
            Ok(Answer::Done)
        }
        Ask::AddHost { intro, deny } => {
            let intro = HostIntro::from_bytes(&sync::bytes(&intro)?)?;
            let host = intro.id()?;
            let deny = deny
                .iter()
                .map(|member| sync::member(member))
                .collect::<crate::control::fleet::Result<Vec<_>>>()?;
            fleet.sign(Body::HostAdd(intro), now_ms)?;
            for member in deny {
                access(fleet, host, member, false, now_ms)?;
            }
            Ok(Answer::Done)
        }
        Ask::Access {
            host,
            member,
            allow,
        } => {
            access(
                fleet,
                sync::host(&host)?,
                sync::member(&member)?,
                allow,
                now_ms,
            )?;
            Ok(Answer::Done)
        }
        Ask::RemoveHost { host } => {
            fleet.sign(Body::HostRemove(sync::host(&host)?), now_ms)?;
            Ok(Answer::Done)
        }
        Ask::Apply(apply) => {
            let heads = apply.heads()?;
            let ops = apply.ops()?;
            let refused = fleet.receive(ops, now_ms)?;
            let inner = fleet.inner();
            let log = inner.log.as_ref().ok_or(Error::NoFleet)?;
            Ok(Answer::Applied(sync::answer(log, &heads, refused)))
        }
        Ask::Joins => {
            let paired = paired(state);
            Ok(Answer::Joins {
                joins: fleet.joins().list(now_ms, &paired),
            })
        }
    }
}

/// The devices paired here now, read before the joins are locked so the
/// two locks never nest.
fn paired(state: &State) -> std::collections::HashSet<crate::control::DeviceId> {
    state.auth().devices().ids().cloned().collect()
}

fn access(
    fleet: &Fleet,
    host: crate::control::fleet::HostId,
    member: crate::control::fleet::MemberId,
    allow: bool,
    now_ms: u64,
) -> Result<()> {
    let rev = {
        let inner = fleet.inner();
        let log = inner.log.as_ref().ok_or(Error::NoFleet)?;
        sync::next_rev(log.state(), host, member)
    };
    fleet.sign(
        Body::Access(Access {
            host,
            member,
            allow,
            rev,
        }),
        now_ms,
    )?;
    Ok(())
}

/// The fleet as this computer sees it, when it is in one.
pub(crate) fn view(fleet: &Fleet) -> Option<View> {
    let suspended = fleet.suspended();
    let inner = fleet.inner();
    inner
        .log
        .as_ref()
        .map(|log| View::of(log, &fleet.me(), &suspended))
}
