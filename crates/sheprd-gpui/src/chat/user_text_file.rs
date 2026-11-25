//! Long user messages in the desktop transcript. Paste boundaries aren't part
//! of the wire format. Project the stored prompt, including in older chats,
//! and materialize a local text file only when the reader opens it.

use super::selection::{Cell, Region};
use crate::{SheprdWindow, config::corners, i18n::fl, notifications::safe_text, window::Flash};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use gpui::{prelude::*, *};
use sha2::{Digest, Sha256};
use sheprd_chat_protocol::{EntryId, assist::typed};
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

const LINES: usize = 12;
const CHARACTERS: usize = 2048;
const FILENAME: &str = "Message.txt"; // i18n: data

#[derive(Debug)]
pub(crate) struct UserTextFile {
    pub(super) attachments: String,
    pub(super) preview: String,
    text: Arc<str>,
}

impl UserTextFile {
    pub(super) fn from_prompt(shown: &str, files: usize) -> Option<Self> {
        let text = typed(shown, files);
        if text.lines().nth(LINES - 1).is_none() && text.chars().nth(CHARACTERS - 1).is_none() {
            return None;
        }
        let first = text.trim_start().lines().next().unwrap_or_default();
        Some(Self {
            attachments: shown[text.len()..].trim_start_matches('\n').into(),
            preview: safe_text(first, 100),
            text: text.into(),
        })
    }
}

// Publish a complete file, even on concurrent opens. Reuse the same working
// copy on subsequent clicks so opening it again preserves edits in the editor.
fn write(root: &Path, text: &str) -> io::Result<PathBuf> {
    let folder = root.join(URL_SAFE_NO_PAD.encode(Sha256::digest(text.as_bytes())));
    super::page::private_dir(&folder)?;
    let path = folder.join(FILENAME);
    let mut file = tempfile::NamedTempFile::new_in(folder)?;
    file.write_all(text.as_bytes())?;
    match file.persist_noclobber(&path) {
        Ok(_) => {}
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.error),
    }
    Ok(path)
}

impl SheprdWindow {
    pub(in crate::chat) fn render_user_text_file(
        &self,
        pane: &str,
        entry: EntryId,
        file: &UserTextFile,
        cx: &mut Context<Self>,
    ) -> Div {
        let text = file.text.clone();
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .gap(px(5.))
            .when(!file.preview.is_empty(), |row| {
                row.child(self.render_chat_text(
                    pane,
                    entry,
                    Region::Body(0),
                    Cell::default(),
                    file.preview.clone(),
                    cx,
                ))
            })
            .child(
                div()
                    .id(("chat-open-message", entry.0))
                    .debug_selector(|| "chat-open-message".into())
                    .role(Role::Button)
                    .aria_label(fl!("chat-open-message-text"))
                    .self_start()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .px(px(10.))
                    .py(px(5.))
                    .rounded(px(corners::CONTROL))
                    .bg(rgb(self.theme.surface))
                    .border_1()
                    .border_color(rgb(self.theme.active))
                    .hover(|el| el.bg(rgb(self.theme.active)))
                    .cursor_pointer()
                    .child(crate::icons::icon("icons/file.svg", 14., self.theme.muted))
                    .child(FILENAME)
                    .child(fl!("chat-file-open"))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.open_user_text_file(text.clone(), cx);
                    })),
            )
            .when(!file.attachments.is_empty(), |row| {
                row.child(self.render_chat_text(
                    pane,
                    entry,
                    Region::Body(1),
                    Cell::default(),
                    file.attachments.clone(),
                    cx,
                ))
            })
    }

    fn open_user_text_file(&self, text: Arc<str>, cx: &mut Context<Self>) {
        let saved = cx.background_spawn(async move {
            let root = super::page::private_folder("messages")
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
            write(&root, &text)
        });
        cx.spawn(async move |this, cx| {
            let result = saved.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(path) => this.open_in_system_app(path, cx),
                Err(error) => this.show_flash(
                    Flash::warning(fl!("chat-message-file-error", error = error.to_string())),
                    cx,
                ),
            });
        })
        .detach();
    }
}
