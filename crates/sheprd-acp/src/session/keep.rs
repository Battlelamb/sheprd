//! Keeping the chat's thread on disk (`sheprd_chat_protocol::thread`): its
//! record, once a prompt reached an agent, and every entry in the thread's
//! transcript file.
//!
//! Entries change as they stream, so the session marks each one it pushes,
//! appends to, or replaces, and writes the marked ones at their latest when
//! a turn ends, the agent changes, the chat ends, enough of them wait, or
//! the transcript is about to drop them. A failure to write is said once in
//! the chat, and the thread is no longer kept.

use std::{collections::BTreeSet, path::PathBuf};

use sheprd_chat_protocol::{
    Change, EntryId, OptionCategory, OptionKind, Transcript,
    thread::{self, Record, Thread, ThreadId, file},
};

use super::{Session, Update, now_ms};

/// Entries marked before they are written without waiting for the turn.
const MARKED: usize = 64;

/// The thread a chat starts with: `--thread`'s, or a new one.
#[derive(Debug)]
pub(crate) struct Start {
    pub dir: PathBuf,
    pub id: ThreadId,
}

/// What the session keeps of its thread on disk.
#[derive(Debug)]
pub(super) struct Keep {
    /// When the first prompt was sent, once it was: only then is the
    /// thread kept.
    created_ms: Option<u64>,
    cwd: PathBuf,
    dir: PathBuf,
    marked: BTreeSet<EntryId>,
    /// While a continued session replays the conversation the transcript
    /// file already holds, its entries are not written again.
    quiet: bool,
    title: String,
    writer: file::Writer,
}

/// The thread a chat continues or starts, with what is kept of it.
pub(super) struct Opened {
    pub keep: Keep,
    /// The newest entry id the thread's file knows, which new entries
    /// must come after.
    pub last: Option<EntryId>,
    pub thread: Thread,
}

impl Keep {
    /// Opens the thread of `start` for `agent`, continuing its record when
    /// there is one. `load` is the session the chat continues: a thread
    /// whose last segment is that session goes on in it; otherwise `agent`
    /// takes over after `last`, the transcript's newest entry.
    pub(super) fn open(
        start: Start,
        agent: sheprd_chat_protocol::Agent,
        load: Option<&str>,
        last: Option<EntryId>,
        cwd: PathBuf,
    ) -> Opened {
        let record = Record::read(&thread::record_path(&start.dir, &start.id)).ok();
        let writer = file::Writer::new(&start.dir, &start.id);
        let known = record
            .as_ref()
            .and_then(|_| file::Contents::read(writer.path()).ok())
            .and_then(|contents| contents.last_id());
        let (thread, created_ms, title) = match record {
            Some(record) => {
                let mut thread = record.thread;
                let same = thread.current().is_some_and(|current| {
                    current.agent == agent && load.is_some() && current.session.as_deref() == load
                });
                if !same {
                    thread.switch(agent, last.max(known));
                }
                (thread, Some(record.created_ms), record.title)
            }
            None => (Thread::new(start.id, agent), None, String::new()),
        };
        Opened {
            keep: Self {
                created_ms,
                cwd,
                dir: start.dir,
                marked: BTreeSet::new(),
                quiet: load.is_some(),
                title,
                writer,
            },
            last: known,
            thread,
        }
    }

    /// Whether the record is on disk, or will be from now on.
    fn kept(&self) -> bool {
        self.created_ms.is_some()
    }

    fn write_record(&self, thread: &Thread) -> Result<(), thread::Error> {
        let Some(created_ms) = self.created_ms else {
            return Ok(());
        };
        Record {
            created_ms,
            cwd: self.cwd.clone(),
            thread: thread.clone(),
            title: self.title.clone(),
            updated_ms: now_ms(),
        }
        .write(&self.dir)
    }

    /// Writes the marked entries the transcript still holds.
    fn flush(&mut self, transcript: &Transcript) -> Result<(), thread::Error> {
        if self.marked.is_empty() || !self.kept() {
            return Ok(());
        }
        let entries: Vec<_> = std::mem::take(&mut self.marked)
            .into_iter()
            .filter_map(|id| transcript.entry(id))
            .collect();
        self.writer.append(&entries)
    }
}

/// The current value of the transcript's model setting.
fn model(transcript: &Transcript) -> Option<&str> {
    match &transcript.option(OptionCategory::Model)?.kind {
        OptionKind::Select { current, .. } => Some(current),
        OptionKind::Toggle { .. } => None,
    }
}

impl<R: FnMut(Update)> Session<R> {
    /// Notes what `change` did to the thread, after it applied: entries to
    /// write, and the model the agent now runs.
    pub(super) fn keep_after(&mut self, change: &Change) {
        let Some(keep) = &mut self.keep else {
            return;
        };
        match change {
            Change::Push { entry } | Change::Replace { entry } if !keep.quiet => {
                keep.marked.insert(entry.id);
            }
            Change::Append { id, .. } if !keep.quiet => {
                keep.marked.insert(*id);
            }
            Change::Options { .. } => {
                let ran = model(&self.transcript)
                    .map(str::to_owned)
                    .zip(self.transcript.thread.as_mut())
                    .is_some_and(|(model, thread)| thread.ran(&model));
                if ran {
                    self.thread_changed();
                }
                return;
            }
            _ => return,
        }
        if keep.marked.len() >= MARKED {
            self.keep_entries();
        }
    }

    /// Writes entries the transcript is about to drop before `before`.
    pub(super) fn keep_before_truncate(&mut self, before: EntryId) {
        if self
            .keep
            .as_ref()
            .is_some_and(|keep| keep.marked.first().is_some_and(|id| *id < before))
        {
            self.keep_entries();
        }
    }

    /// Writes every marked entry now.
    pub(super) fn keep_entries(&mut self) {
        let Some(keep) = &mut self.keep else {
            return;
        };
        if let Err(error) = keep.flush(&self.transcript) {
            self.keep_failed(&error);
        }
    }

    /// The first prompt, `shown`, reached the agent: the thread is kept from
    /// now on, titled by it.
    pub(super) fn keep_started(&mut self, shown: &str) {
        let Some(keep) = &mut self.keep else {
            return;
        };
        if keep.kept() {
            return;
        }
        keep.created_ms = Some(now_ms());
        keep.title = thread::title(shown);
        self.thread_changed();
    }

    /// The session continued or opened: entries from now on are new.
    pub(super) fn keep_opened(&mut self, session: &str) {
        if let Some(keep) = &mut self.keep {
            keep.quiet = false;
        }
        if let Some(thread) = &mut self.transcript.thread {
            thread.opened(session);
            self.thread_changed();
        }
    }

    /// `agent` takes over the thread after the transcript's newest entry.
    pub(super) fn keep_switched(&mut self, agent: sheprd_chat_protocol::Agent) {
        self.keep_entries();
        let last = self.transcript.last_entry_id();
        if let Some(thread) = &mut self.transcript.thread {
            thread.switch(agent, last);
        }
        if let Some(keep) = &mut self.keep {
            keep.quiet = false;
        }
    }

    /// The thread's segments changed: clients and the record hear of it.
    pub(super) fn thread_changed(&mut self) {
        let Some(thread) = self.transcript.thread.clone() else {
            return;
        };
        let written = self
            .keep
            .as_ref()
            .map_or(Ok(()), |keep| keep.write_record(&thread));
        if let Err(error) = written {
            self.keep_failed(&error);
        }
        self.emit(Change::Thread { thread });
    }

    /// Keeping failed: the chat says so once and keeps nothing more.
    fn keep_failed(&mut self, error: &thread::Error) {
        if self.keep.take().is_some() {
            self.notice(
                sheprd_chat_protocol::NoticeLevel::Warning,
                format!("This conversation can no longer be kept as a thread: {error}"),
            );
        }
    }
}
