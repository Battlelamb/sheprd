//! `POST /api/v2/chat/{pane}/summarize`: asks the chat's agent program, run
//! once on its own, where the chat stands. It answers at once; the summary
//! reaches every phone's list on `home` when the agent is done.

use crate::{
    hub::{RowSummary, row_summary},
    web::{ApiError, Device, Shared, now_ms},
};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

/// `202` with an empty object, which a phone's JSON client reads.
type Accepted = (StatusCode, Json<serde_json::Value>);

fn accepted() -> Accepted {
    (StatusCode::ACCEPTED, Json(serde_json::json!({})))
}

pub(crate) async fn summarize(
    State(state): State<Shared>,
    _device: Device,
    Path(pane): Path<String>,
) -> Result<Accepted, ApiError> {
    state.hub.layout().chat(&pane).ok_or(ApiError::NotFound)?;
    let link = state.hub.chat(&pane).ok_or(ApiError::NotFound)?;
    let (transcript, _) = link.base();
    let input = row_summary::excerpt_of(&transcript);
    if input.is_empty() {
        return Err(ApiError::Invalid("nothing has been said in this chat yet"));
    }
    let slot = row_summary::Slot::take()
        .map_err(|_| ApiError::Invalid("too many summaries are running; try again in a moment"))?;
    if !link.begin_summary(now_ms()) {
        return Ok(accepted());
    }
    let agent = transcript.agent;
    tokio::spawn(async move {
        let answer = tokio::task::spawn_blocking(move || {
            let _slot = slot;
            row_summary::run(&agent, &input)
        })
        .await;
        let summary = match answer {
            Ok(Ok(text)) => RowSummary::Ready {
                text,
                at_ms: now_ms(),
            },
            Ok(Err(error)) => RowSummary::Failed {
                reason: error.to_string(),
                at_ms: now_ms(),
            },
            Err(_) => RowSummary::Failed {
                reason: row_summary::Error::Failed.to_string(),
                at_ms: now_ms(),
            },
        };
        link.finish_summary(summary);
    });
    Ok(accepted())
}
