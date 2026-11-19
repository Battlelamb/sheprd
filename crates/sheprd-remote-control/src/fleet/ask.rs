//! What Sheprd asks the daemon about the fleet (`control::fleet::sync::Ask`).
//! Every operation this computer signs is one a person here asked for in
//! Sheprd's own dialog; the daemon only signs, keeps, and passes them on.

use super::{Error, Fleet, Result, me};
use crate::control::fleet::{
    Access, Admit, Body, Bundle, HostIntro, Remove, View,
    sync::{self, Answer, Ask},
};
use crate::web::State;
use std::collections::BTreeSet;

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
                .collect::<crate::control::fleet::Result<BTreeSet<_>>>()?;
            // The host and its exclusions land together or not at all: a
            // host kept without them would be shared wider than asked.
            fleet.sign_all(now_ms, |log| {
                std::iter::once(Body::HostAdd(intro))
                    .chain(deny.into_iter().map(|member| {
                        Body::Access(Access {
                            host,
                            member,
                            allow: false,
                            rev: sync::next_rev(log.state(), host, member),
                        })
                    }))
                    .collect()
            })?;
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
        Ask::Intro => {
            let mut intro = me::intro(state, now_ms)?;
            intro.kind = crate::control::fleet::HostKind::Sheprd;
            Ok(Answer::Intro {
                intro: sync::text(&intro.to_bytes()?),
            })
        }
        Ask::Scrub => {
            super::scrub::live(state, fleet)?;
            Ok(Answer::Done)
        }
        Ask::Apply(apply) => {
            // A caller naming another fleet learns nothing of this one.
            if let Some(named) = &apply.fleet
                && let Some(log) = &fleet.inner().log
                && *named != log.fleet().to_string()
            {
                return Err(Error::OtherFleet);
            }
            let heads = apply.heads()?;
            let ops = apply.ops()?;
            if apply.more && fleet.inner().log.is_none() {
                // A part of a log handed over: nothing to answer yet.
                fleet.stage(ops, now_ms)?;
                return Ok(Answer::Applied(sync::Applied::default()));
            }
            let refused = fleet.receive(ops, now_ms)?;
            let mut applied = {
                let inner = fleet.inner();
                let log = inner.log.as_ref().ok_or(Error::NoFleet)?;
                sync::answer(log, &heads, refused)
            };
            if let (Some(nonce), Some(authority)) = (&apply.nonce, &state.authority) {
                let message = sync::proof_message(nonce, &applied.fleet, &applied.heads);
                if let (Some(key), Some(signature)) =
                    (authority.public_key(), authority.sign(&message))
                {
                    applied.proof = Some(sync::Proof {
                        certificate: sync::text(authority.ca_der()),
                        key: sync::text(&key),
                        signature: sync::text(&signature),
                    });
                }
            }
            Ok(Answer::Applied(applied))
        }
    }
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
