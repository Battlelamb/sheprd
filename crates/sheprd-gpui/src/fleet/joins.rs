//! Phones asking to join through this computer's daemon (`Ask::Joins`).
//!
//! A join is admitted only with a person's approval here: either the
//! pairing dialog's Also add to your fleet, given for one device id and
//! good for [`EXPECT`], or the second click on Admit in Settings > Fleet.
//! Nothing else ever admits a join.

use super::Fleet;
use gpui::{App, BorrowAppContext};
use sheprd_remote_control::control::{
    DeviceId, Request, Response,
    fleet::{
        Method,
        sync::{Answer, Ask, Join},
    },
};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

/// How long a pairing's Also add to your fleet waits for the phone's join;
/// the daemon keeps a join as long.
pub(super) const EXPECT: Duration = Duration::from_secs(600);
/// How often the daemon is asked while a join is expected.
pub(super) const WATCH: Duration = Duration::from_secs(5);

/// Splits `joins` into those an approval in `expected` covers, and the
/// rest, which wait for a person. An approval is used once, and dropped
/// once it is older than [`EXPECT`].
pub(super) fn settle(
    expected: &mut HashMap<DeviceId, Instant>,
    joins: Vec<Join>,
    now: Instant,
) -> (Vec<Join>, Vec<Join>) {
    expected.retain(|_, at| now.duration_since(*at) < EXPECT);
    joins
        .into_iter()
        .partition(|join| expected.remove(&join.device_id).is_some())
}

/// What admits `join`.
pub(crate) fn admit(join: &Join) -> Ask {
    Ask::Admit {
        bundle: join.bundle.clone(),
        signature: join.signature.clone(),
        // A phone joins only once it is paired, and pairing is the QR
        // code or nearby code comparison.
        method: Method::Qr,
    }
}

impl Fleet {
    /// Admits the phone `device` when it asks to join within [`EXPECT`]:
    /// the person ticked Also add to your fleet when approving it.
    pub(crate) fn expect(cx: &mut App, device: DeviceId) {
        if !cx.has_global::<Self>() {
            return;
        }
        cx.update_global::<Self, _>(|this, _| {
            this.expected.insert(device, Instant::now());
        });
        Self::refresh(cx);
    }

    /// The joins waiting for a person, oldest first.
    pub(crate) fn joins(cx: &App) -> &[Join] {
        cx.try_global::<Self>()
            .map_or(&[], |fleet| fleet.joins.as_slice())
    }

    /// How soon the view is read again.
    pub(super) fn pace(&self) -> Duration {
        if self.expected.is_empty() {
            super::VIEW
        } else {
            WATCH
        }
    }

    /// Reads the joins waiting, admits those approved at pairing, and
    /// keeps the rest for the page.
    pub(super) fn read_joins(cx: &mut App) {
        let Some(client) = crate::remote::Supervisor::client(cx) else {
            return;
        };
        let task = client.request(Request::Fleet(Box::new(Ask::Joins)));
        cx.spawn(async move |cx| {
            let answer = task.await;
            cx.update(|cx| {
                let joins = match answer {
                    Ok(Response::Fleet(answer)) => match *answer {
                        Answer::Joins { joins } => joins,
                        _ => return,
                    },
                    // An older daemon has no joins to list.
                    _ => return,
                };
                let admit = cx.update_global::<Self, _>(|this, _| {
                    let (admit, rest) = settle(&mut this.expected, joins, Instant::now());
                    this.joins = rest;
                    admit
                });
                for join in &admit {
                    Self::act(cx, self::admit(join), |failure, cx| {
                        if failure.is_some() {
                            cx.update_global::<Self, _>(|this, _| this.error = failure);
                        }
                    });
                }
                cx.refresh_windows();
            });
        })
        .detach();
    }
}
