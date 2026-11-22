//! `Request::TestPush`: Sheprd's Send Test Push. One sealed test alert to
//! each phone registered for alerts, at once, as each phone's own test
//! sends it (`web::test_push::alert`), answered within the control
//! socket's wait.

use crate::{
    control::{
        ASK_WAIT, DeviceId,
        push::{Outcome, TestPush, TestPushes},
    },
    web::{Shared, test_push},
};
use tokio::{runtime::Handle, task::JoinSet};

/// Phones tested at most: more than anyone pairs.
pub(crate) const MAX_PHONES: usize = 16;

/// Sends the test alerts on `runtime` and waits for them, from one of the
/// control socket's threads, never the runtime's own.
pub(crate) fn answer(state: &Shared, runtime: &Handle) -> TestPushes {
    let mode = state.apns.mode();
    if state.apns.is_off() {
        return TestPushes {
            mode,
            pushes: Vec::new(),
        };
    }
    // Phones that never registered with APNs are left out before any push.
    let phones: Vec<(DeviceId, String)> = state
        .auth()
        .devices()
        .records()
        .iter()
        .filter(|record| record.apns.is_some())
        .take(MAX_PHONES)
        .map(|record| (record.id.clone(), record.name.clone()))
        .collect();
    let mut sending = JoinSet::new();
    for (index, (device, _)) in phones.iter().enumerate() {
        let (state, device) = (state.clone(), device.clone());
        let id = uuid::Uuid::new_v4().to_string();
        sending.spawn_on(
            async move { (index, test_push::alert(&state, &device, id).await) },
            runtime,
        );
    }
    // `None` until it answers; then `Some(None)` for a phone with no alert
    // handle the push relay takes.
    let mut outcomes: Vec<Option<Option<Outcome>>> = vec![None; phones.len()];
    runtime.block_on(async {
        let _ = tokio::time::timeout(ASK_WAIT, async {
            while let Some(joined) = sending.join_next().await {
                if let Ok((index, sent)) = joined
                    && let Some(slot) = outcomes.get_mut(index)
                {
                    *slot = Some(sent.as_ref().map(Outcome::from));
                }
            }
        })
        .await;
    });
    let pushes = phones
        .into_iter()
        .zip(outcomes)
        .filter_map(|((_, device), outcome)| match outcome {
            None => Some(TestPush {
                device,
                outcome: Outcome::Unreachable,
            }),
            Some(Some(outcome)) => Some(TestPush { device, outcome }),
            Some(None) => None,
        })
        .collect();
    TestPushes { mode, pushes }
}
