//! `POST /api/apns/test`: the phone asks this computer to send it one
//! sealed alert now, to see notifications arrive. The answer is what APNs
//! said; the phone's extension records the id when the alert opens, so the
//! app can tell whether, and how fast, it came.

use super::{ApiError, Device, Shared, now_ms};
use crate::apns::{
    self, Outcome, Registration,
    alert::{Sealed, valid_test_id},
};
use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TestRequest {
    id: String,
}

/// What APNs answered. `reason` is one of Apple's fixed names.
#[derive(Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub(super) enum TestAnswer {
    Delivered,
    /// APNs no longer knows this phone's alert token: it is forgotten, and
    /// the phone registers a new one when it next opens.
    Gone,
    Limited,
    Refused {
        status: u16,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// APNs could not be reached.
    Unreachable,
}

pub(super) async fn send(
    State(state): State<Shared>,
    Device(device): Device,
    Json(request): Json<TestRequest>,
) -> Result<Json<TestAnswer>, ApiError> {
    let sender = state.apns.as_ref().ok_or(ApiError::NoPush)?;
    if !valid_test_id(&request.id) {
        return Err(ApiError::Invalid("that test id is not valid"));
    }
    let registration = Registration::from_stored(
        state
            .auth()
            .devices()
            .get(&device)
            .and_then(|record| record.apns.as_ref()),
    );
    let (token, key) = registration
        .alerts()
        .map(|(token, key)| (token.clone(), *key))
        .ok_or(ApiError::Invalid(
            "this phone has not registered for alerts",
        ))?;
    let badge = u32::try_from(state.attention().len()).unwrap_or(u32::MAX);
    let sealed = Sealed::test(
        request.id.clone(),
        apns::computer_id(&state.fingerprint),
        &state.computer,
        badge,
    );
    let tag = format!("test-{}", request.id);
    let push = apns::alert::push(sealed, &tag, |plaintext| {
        Ok(apns::seal::seal(&key, plaintext, apns::seal::INFO)?)
    })
    .map_err(|_| ApiError::Invalid("the test push could not be sealed"))?;
    let answer = match sender.post(&token, &push, now_ms() / 1000).await {
        Ok(Outcome::Delivered) => TestAnswer::Delivered,
        Ok(Outcome::Gone) => {
            let state = state.clone();
            let _ = tokio::task::spawn_blocking(move || {
                crate::server::forget(&state, &device, &token);
            })
            .await;
            TestAnswer::Gone
        }
        Ok(Outcome::RateLimited) => TestAnswer::Limited,
        Ok(Outcome::Rejected { reason, status }) => TestAnswer::Refused { status, reason },
        Err(error) => {
            tracing::warn!(%error, "could not send a test push");
            TestAnswer::Unreachable
        }
    };
    Ok(Json(answer))
}
