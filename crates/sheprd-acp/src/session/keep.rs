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
    Change, Entry, EntryId, EntryKind, OptionCategory, OptionKind, Transcript,
    thread::{self, Record, Thread, ThreadId, file},
};

use super::{Session, Update, now_ms};

/// Entries marked before they are written without waiting for the turn.
const MARKED: usize = 64;

/// The thread a chat starts with, `--thread`'s or a new one, and what is
/// already kept of it.
#[derive(Debug)]
pub(crate) struct Start {
    dir: PathBuf,
    id: ThreadId,
    /// The newest entry id the thread's file knows, kept or trimmed.
    last: Option<EntryId>,
    record: Option<Record>,
    writer: file::Writer,
}

impl Start {
    /// Reads what thread `id` in `dir` keeps: nothing for a thread not kept
    /// yet. Anything kept that cannot be read is an error, so a thread is
    /// never written over with less than it held.
    pub(crate) fn read(dir: PathBuf, id: ThreadId) -> Result<Self, thread::Error> {
        let record = match Record::read(&thread::record_path(&dir, &id)) {
            Ok(record) => Some(record),
            Err(thread::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let writer = file::Writer::open(&dir, &id)?;
        let last = file::Contents::read(writer.path())?.last_id();
        Ok(Self {
            dir,
            id,
            last,
            record,
            writer,
        })
    }
}

/// What the session keeps of its thread on disk.
#[derive(Debug)]
pub(super) struct Keep {
    /// When the conversation started, once it has: only then is the thread
    /// kept.
    created_ms: Option<u64>,
    cwd: PathBuf,
    dir: PathBuf,
    marked: BTreeSet<EntryId>,
    /// While a continued session replays the conversation the transcript
    /// file already holds, its entries are not written again.
    quiet: bool,
    title: String,
    /// Whether the record is on disk.
    written: bool,
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
    /// takes over after the newest entry. A new thread keeps `entries`, the
    /// conversation the chat opened with, and is kept at once when they
    /// hold a prompt.
    pub(super) fn open(
        start: Start,
        agent: sheprd_chat_protocol::Agent,
        load: Option<&str>,
        entries: &[Entry],
        cwd: PathBuf,
    ) -> Opened {
        let Start {
            dir,
            id,
            last,
            record,
            writer,
        } = start;
        let newest = entries.last().map(|entry| entry.id);
        let mut keep = Self {
            created_ms: None,
            cwd,
            dir,
            marked: BTreeSet::new(),
            quiet: false,
            title: String::new(),
            written: record.is_some(),
            writer,
        };
        let thread = match record {
            Some(record) => {
                let mut thread = record.thread;
                let same = thread.current().is_some_and(|current| {
                    current.agent == agent && load.is_some() && current.session.as_deref() == load
                });
                if !same {
                    thread.switch(agent, newest.max(last));
                }
                keep.created_ms = Some(record.created_ms);
                keep.title = record.title;
                // The file holds the conversation the session replays.
                keep.quiet = load.is_some();
                thread
            }
            None => {
                keep.marked.extend(entries.iter().map(|entry| entry.id));
                if let Some(prompt) = entries.iter().find_map(|entry| match &entry.kind {
                    EntryKind::User { text, .. } => Some(text),
                    _ => None,
                }) {
                    keep.created_ms = Some(now_ms());
                    keep.title = thread::title(prompt);
                }
                Thread::new(id, agent)
            }
        };
        Opened { keep, last, thread }
    }

    /// Whether the record is on disk, or will be from now on.
    fn kept(&self) -> bool {
        self.created_ms.is_some()
    }

    fn write_record(&mut self, thread: &Thread) -> Result<(), thread::Error> {
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
        .write(&self.dir)?;
        self.written = true;
        Ok(())
    }

    /// Writes the marked entries the transcript still holds.
    fn flush(&mut self, transcript: &Transcript) -> Result<(), thread::Error> {
        if self.marked.is_empty() || !self.kept() {
            return Ok(());
        }
        // A transcript file is never left without the record naming it.
        if !self.written
            && let Some(thread) = &transcript.thread
        {
            self.write_record(thread)?;
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
    /// Marks the entry `change` made or changed, right after it applied and
    /// before the transcript may drop it for room.
    pub(super) fn keep_mark(&mut self, change: &Change) {
        let Some(keep) = self.keep.as_mut().filter(|keep| !keep.quiet) else {
            return;
        };
        match change {
            Change::Push { entry } | Change::Replace { entry } => {
                keep.marked.insert(entry.id);
            }
            Change::Append { id, .. } => {
                keep.marked.insert(*id);
            }
            _ => {}
        }
    }

    /// What `change` did to the thread once every client has it: entries
    /// written when enough wait, the model the agent now runs, and a
    /// failure to keep the thread said.
    pub(super) fn keep_after(&mut self, change: &Change) {
        if self
            .keep
            .as_ref()
            .is_some_and(|keep| keep.marked.len() >= MARKED)
        {
            self.keep_entries();
        }
        if matches!(change, Change::Options { .. }) {
            let ran = model(&self.transcript)
                .map(str::to_owned)
                .zip(self.transcript.thread.as_mut())
                .is_some_and(|(model, thread)| thread.ran(&model));
            if ran {
                self.thread_changed();
            }
        }
        self.say_keep_failure();
    }

    /// Writes entries the transcript is about to drop before `before`. A
    /// failure is said once the change that drops them is out.
    pub(super) fn keep_before_truncate(&mut self, before: EntryId) {
        let Some(keep) = &mut self.keep else {
            return;
        };
        if keep.marked.first().is_some_and(|id| *id < before)
            && let Err(error) = keep.flush(&self.transcript)
        {
            self.keep_failed(&error);
        }
    }

    /// Writes every marked entry now.
    pub(super) fn keep_entries(&mut self) {
        let Some(keep) = &mut self.keep else {
            return;
        };
        if let Err(error) = keep.flush(&self.transcript) {
            self.keep_failed(&error);
            self.say_keep_failure();
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
        if let Some(keep) = &mut self.keep
            && let Err(error) = keep.write_record(&thread)
        {
            self.keep_failed(&error);
        }
        self.emit(Change::Thread { thread });
    }

    /// Keeping failed: nothing more is kept, and the chat says so once.
    fn keep_failed(&mut self, error: &thread::Error) {
        if self.keep.take().is_some() {
            self.keep_failure = Some(format!(
                "This conversation can no longer be kept as a thread: {error}"
            ));
        }
    }

    fn say_keep_failure(&mut self) {
        if let Some(text) = self.keep_failure.take() {
            self.notice(sheprd_chat_protocol::NoticeLevel::Warning, text);
        }
    }
}
