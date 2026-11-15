//! The question a terminal agent waits on, as a phone shows it: read from
//! the pane's screen (`sheprd_client::herdr::prompt`) and answered with the
//! keys a person would press there. Reading neither attaches nor resizes.
//! An answer names the choice it was shown, and the screen is read again
//! first, so a question that changed meanwhile is never answered blind.

use super::{ApiError, Device, Shared};
use crate::hub::ApiMethod;
use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sheprd_client::herdr::prompt::{self, Prompt};

/// The longest typed answer.
const MAX_TEXT: usize = 4 * 1024;

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PromptAnswer {
    /// `None` when the pane shows no menu.
    pub(crate) prompt: Option<Prompt>,
}

/// `GET /api/term/{pane}/prompt`.
pub(super) async fn get(
    State(state): State<Shared>,
    _device: Device,
    Path(pane): Path<String>,
) -> Result<Json<PromptAnswer>, ApiError> {
    super::terminal::known(&state.hub.layout(), &pane)?;
    Ok(Json(PromptAnswer {
        prompt: current(&state, &pane).await?,
    }))
}

#[derive(Deserialize)]
pub(super) struct Choose {
    /// The choice's number and label as the phone showed them.
    number: u32,
    label: String,
    /// The words for a choice that takes them.
    #[serde(default)]
    text: Option<String>,
}

/// `POST /api/term/{pane}/prompt`: takes one choice.
pub(super) async fn choose(
    State(state): State<Shared>,
    device: Device,
    Path(pane): Path<String>,
    Json(request): Json<Choose>,
) -> Result<Json<Value>, ApiError> {
    super::terminal::known(&state.hub.layout(), &pane)?;
    let prompt = current(&state, &pane)
        .await?
        .ok_or(ApiError::Invalid("the question was already answered"))?;
    let index = prompt
        .choices
        .iter()
        .position(|choice| choice.number == request.number && choice.label == request.label)
        .ok_or(ApiError::Invalid("the question changed; look again"))?;
    let mut keys = prompt::keys_to(&prompt, index);
    if prompt.choices[index].text {
        let text = request
            .text
            .filter(|text| !text.trim().is_empty() && text.len() <= MAX_TEXT)
            .ok_or(ApiError::Invalid("this choice needs words"))?;
        // Moved there first; the words then go in it, and Enter takes them.
        keys.pop();
        send_keys(&state, &pane, &keys).await?;
        state
            .hub
            .call(
                ApiMethod::PaneSendText,
                json!({"pane_id": pane, "text": text}),
            )
            .await?;
        send_keys(&state, &pane, &["enter"]).await?;
    } else {
        send_keys(&state, &pane, &keys).await?;
    }
    state.typed.typed_by(&pane, &device.0);
    Ok(Json(json!({})))
}

async fn send_keys(state: &Shared, pane: &str, keys: &[&str]) -> Result<(), ApiError> {
    if keys.is_empty() {
        return Ok(());
    }
    state
        .hub
        .call(
            ApiMethod::PaneSendKeys,
            json!({"pane_id": pane, "keys": keys}),
        )
        .await?;
    Ok(())
}

async fn current(state: &Shared, pane: &str) -> Result<Option<Prompt>, ApiError> {
    let read = state
        .hub
        .call(
            ApiMethod::PaneRead,
            json!({"pane_id": pane, "source": "visible", "format": "text"}),
        )
        .await?;
    Ok(prompt::read(
        read["read"]["text"].as_str().unwrap_or_default(),
    ))
}
