//! Live Activity payloads for the push relay, sealed to the phone so neither
//! the relay nor APNs reads them (`docs/apns-relay-plan.md`, Live Activity
//! privacy). `content-state`, and `attributes` on a start, each become one
//! `sealed` field under [`seal::ACTIVITY_INFO`]; the widget extension opens
//! them with the key alerts use. Only what iOS itself needs stays readable:
//!
//! ```json
//! { "aps": {
//!     "timestamp": 1800000000, "event": "start",
//!     "attributes-type": "ChatActivityAttributes",
//!     "attributes": { "sealed": "BASE64URL" },
//!     "content-state": { "sealed": "BASE64URL" },
//!     "alert": { "title": "Sheprd", "body": "New activity" } } }
//! ```
//!
//! An update's alert is the same cover, and an end keeps its
//! `dismissal-date`. What is sealed is what the phone may see, after
//! `Live::redacted`, cut until the payload fits APNs's 4 KB.

use super::{
    Error, Event, Live, Push, PushType, Result,
    alert::{COVER_BODY, COVER_TITLE},
    payload::{ATTRIBUTES_TYPE, MAX_PAYLOAD},
    seal,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use p256::PublicKey;
use serde::Serialize;
use serde_json::{Map, Value, json};
use sheprd_apns::{SealedPush, Sealer};

/// Text no shorter than this is left alone when cutting to fit.
const MIN_TEXT: usize = 8;

impl Live {
    /// The push for this change at `now_s`, sealed to `device`, with its
    /// longest text cut until it fits.
    pub(crate) fn sealed_push(&self, device: &PublicKey, now_s: u64) -> Result<SealedPush> {
        let mut live = self.clone();
        loop {
            match SealedPush::build(device, seal::ACTIVITY_INFO, |sealer| {
                live.sealed(sealer, now_s)
            }) {
                Err(Error::PayloadTooLarge { .. }) if live.shorten() => {}
                built => return built,
            }
        }
    }

    fn sealed(&self, sealer: &Sealer<'_>, timestamp_s: u64) -> Result<Push> {
        let cover = json!({ "title": COVER_TITLE, "body": COVER_BODY });
        let mut aps = Map::new();
        aps.insert("timestamp".into(), json!(timestamp_s));
        aps.insert("event".into(), json!(self.event.name()));
        aps.insert("content-state".into(), sealed_field(sealer, &self.state)?);
        match &self.event {
            Event::Start { attributes, .. } => {
                aps.insert("attributes-type".into(), json!(ATTRIBUTES_TYPE));
                aps.insert("attributes".into(), sealed_field(sealer, attributes)?);
                aps.insert("alert".into(), cover);
            }
            Event::Update { alert: Some(_) } => {
                aps.insert("alert".into(), cover);
            }
            Event::Update { alert: None } => {}
            Event::End { dismissal_s } => {
                aps.insert("dismissal-date".into(), json!(dismissal_s));
            }
        }
        let mut root = Map::new();
        root.insert("aps".into(), Value::Object(aps));
        let body = serde_json::to_vec(&Value::Object(root))?;
        if body.len() > MAX_PAYLOAD {
            return Err(Error::PayloadTooLarge { len: body.len() });
        }
        Ok(Push {
            body,
            collapse_id: None,
            priority: self.priority(),
            push_type: PushType::LiveActivity,
        })
    }

    /// Drops the earlier steps, else halves the longest text; `false` when
    /// nothing is left worth cutting.
    fn shorten(&mut self) -> bool {
        let state = &mut self.state;
        if !state.previous.is_empty() {
            state.previous.clear();
            return true;
        }
        let mut texts = vec![&mut state.step];
        texts.extend(state.question.as_mut());
        texts.extend(state.summary.as_mut());
        if let Event::Start { attributes, .. } = &mut self.event {
            texts.extend([&mut attributes.chat_title, &mut attributes.workspace]);
        }
        let Some(longest) = texts.into_iter().max_by_key(|text| text.len()) else {
            return false;
        };
        let chars = longest.chars().count();
        if chars <= MIN_TEXT {
            return false;
        }
        let kept: String = longest.chars().take(chars / 2).collect();
        *longest = format!("{kept}\u{2026}");
        true
    }
}

/// `{"sealed": BASE64URL}` holding `value` as JSON.
fn sealed_field(sealer: &Sealer<'_>, value: &impl Serialize) -> Result<Value> {
    let sealed = sealer.seal(&serde_json::to_vec(value)?)?;
    Ok(json!({ "sealed": URL_SAFE_NO_PAD.encode(sealed) }))
}
