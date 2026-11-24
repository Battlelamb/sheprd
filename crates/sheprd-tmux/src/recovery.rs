//! Reopen a saved pane on a new host boot. Only a login shell or an exact
//! provider session may be resumed. A provider may keep its history outside
//! the default location, so its session directory overrides travel with it.
use super::{Error, Result, Tmux};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sheprd_backend::{Spawn, TerminalId, TerminalInfo};
use sheprd_chat_protocol::{Agent, ENDPOINT_ENV, EndpointId, TOKEN};
use sheprd_client::shell_quote;
use std::{collections::BTreeMap, path::PathBuf};

const SCRIPT: &str = include_str!("recovery.sh");

#[derive(Deserialize, Serialize)]
struct Saved {
    id: String,
    cwd: Option<PathBuf>,
    name: Option<String>,
    chat: Option<ChatRecovery>,
    #[serde(default)]
    labels: BTreeMap<String, String>,
}

/// Continuation information supplied by the running ACP host at session/open.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ChatRecovery {
    pub agent: Agent,
    pub adapter: bool,
    pub args: Vec<String>,
    pub endpoint: String,
    pub session: String,
    pub helper: PathBuf,
    #[serde(default)]
    pub directories: BTreeMap<String, String>,
}

impl Saved {
    fn json(&self) -> Result<String> {
        let json = serde_json::to_string(self).map_err(|_| Error::Invalid)?;
        (json.len() <= 16_000).then_some(json).ok_or(Error::Invalid)
    }

    fn spawn(self, id: &TerminalId) -> Result<Spawn> {
        if self.id != id.as_str() || self.cwd.as_ref().is_some_and(|p| !p.is_absolute()) {
            return Err(Error::Invalid);
        }
        let mut spawn = Spawn {
            cwd: self.cwd,
            name: self.name,
            labels: placement(self.labels),
            ..Default::default()
        };
        if let Some(chat) = self.chat {
            let endpoint: EndpointId = chat.endpoint.parse().map_err(|_| Error::Invalid)?;
            if !sheprd_chat_protocol::restore::session_id_is_plain(&chat.session)
                || !chat.helper.is_absolute()
                || !matches!(
                    chat.helper.file_name().and_then(|s| s.to_str()),
                    Some("herdr-acp" | "sheprd-acp")
                )
            {
                return Err(Error::Invalid);
            }
            spawn.argv.push(
                chat.helper
                    .into_os_string()
                    .into_string()
                    .map_err(|_| Error::Invalid)?,
            );
            if chat.adapter {
                spawn.argv.push("--acp".into());
            }
            spawn
                .argv
                .extend(["--resume".into(), chat.session, chat.agent.to_string()]);
            if !chat.args.is_empty() {
                spawn.argv.push("--".into());
                spawn.argv.extend(chat.args);
            }
            let allowed = sheprd_client::herdr::AgentKind::parse(chat.agent.name())
                .map(sheprd_client::herdr::sessions::environment_keys)
                .unwrap_or_default();
            if chat
                .directories
                .keys()
                .any(|key| !allowed.contains(&key.as_str()))
            {
                return Err(Error::Invalid);
            }
            spawn.env.extend(chat.directories);
            spawn.env.push((ENDPOINT_ENV.into(), endpoint.to_string()));
            spawn.labels.insert(TOKEN.into(), endpoint.to_string());
        }
        Ok(spawn)
    }
}

/// Keep only tab placement. A restored process must establish its own agent
/// status, endpoint and ownership of any automated work.
fn placement(mut labels: BTreeMap<String, String>) -> BTreeMap<String, String> {
    use sheprd_session::{
        NEW_WORKSPACE_LABEL, NEW_WORKSPACE_TITLE_LABEL, SESSION_LABEL, WORKSPACE_LABEL,
    };
    labels.retain(|key, _| {
        matches!(
            key.as_str(),
            NEW_WORKSPACE_LABEL | NEW_WORKSPACE_TITLE_LABEL | SESSION_LABEL | WORKSPACE_LABEL
        )
    });
    labels
}

fn script(action: &str, id: &TerminalId, json: &str) -> Result<String> {
    Tmux::target(id)?;
    Ok(format!(
        "/bin/sh -c {} sh {} {} {}",
        shell_quote(SCRIPT),
        shell_quote(action),
        shell_quote(id.as_str()),
        shell_quote(json)
    ))
}

impl Tmux {
    fn recovery_command(&self, action: &str, id: &TerminalId, json: &str) -> Result<Vec<u8>> {
        let script = script(action, id, json)?;
        if let Some(host) = &self.target {
            return super::host::with_script(host.as_ref(), &script, false, Self::output);
        }
        let mut cmd = sheprd_client::new_command("/bin/sh");
        cmd.args(["-c", &script]);
        Self::output(cmd)
    }

    pub(super) fn keep_shell_script(&self, id: &TerminalId, spawn: &Spawn) -> Result<String> {
        let saved = Saved {
            id: id.to_string(),
            cwd: spawn.cwd.clone(),
            name: spawn.name.clone(),
            chat: None,
            labels: placement(spawn.labels.clone()),
        };
        Ok(format!("{}\n", script("shell", id, &saved.json()?)?))
    }

    /// Called only by the chat that has opened the provider session.
    pub fn keep_chat(&self, id: &TerminalId, cwd: PathBuf, chat: ChatRecovery) -> Result<()> {
        let (name, labels) = std::env::var("SHEPRD_TMUX_METADATA")
            .ok()
            .and_then(|text| super::URL_SAFE_NO_PAD.decode(text).ok())
            .and_then(|bytes| serde_json::from_slice::<super::Metadata>(&bytes).ok())
            .unwrap_or_default();
        let saved = Saved {
            id: id.to_string(),
            cwd: Some(cwd),
            name,
            labels: placement(labels),
            chat: Some(chat),
        };
        let json = saved.json()?;
        saved.spawn(id)?;
        self.recovery_command("chat", id, &json).map(drop)
    }

    pub fn close_chat(&self, id: &TerminalId) -> Result<()> {
        self.recovery_command("close-chat", id, "").map(drop)
    }

    pub(super) fn forget(&self, id: &TerminalId) -> Result<()> {
        self.recovery_command("forget", id, "").map(drop)
    }

    pub(super) fn recover(&self, id: &TerminalId) -> Result<Option<TerminalInfo>> {
        self.recover_saved(id, false)
    }

    /// The remote service can resume an explicitly recorded chat after a
    /// host reboot even if the desktop stays closed. Shells require a view.
    pub fn recover_chat(&self, id: &TerminalId) -> Result<Option<TerminalInfo>> {
        self.recover_saved(id, true)
    }

    fn recover_saved(&self, id: &TerminalId, chat_only: bool) -> Result<Option<TerminalInfo>> {
        let bytes = self.recovery_command("read", id, "")?;
        if bytes.is_empty() {
            return Ok(None);
        }
        let json = std::str::from_utf8(&bytes)
            .map_err(|_| Error::Invalid)?
            .trim_end();
        let saved: Saved = serde_json::from_str(json).map_err(|_| Error::Invalid)?;
        if chat_only && saved.chat.is_none() {
            return Ok(None);
        }
        // new-session serializes competing openers. Recheck after it owns the
        // name so a close between the first read and spawn also wins.
        let guard = format!("{} || exit 1\n", script("claim", id, json)?);
        self.spawn_id(id.clone(), &saved.spawn(id)?, Some(&guard))
            .map(Some)
    }
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};

    #[test]
    fn only_a_new_boot_can_revive_an_open_saved_pane() {
        let dir = tempfile::tempdir().unwrap();
        let id = Tmux::named_id("qa").unwrap();
        let run = |action, json| {
            let out = sheprd_client::new_command("/bin/sh")
                .env("XDG_STATE_HOME", dir.path())
                .args(["-c", &script(action, &id, json).unwrap()])
                .output()
                .unwrap();
            assert!(out.status.success(), "{:?}", out.stderr);
            String::from_utf8(out.stdout).unwrap()
        };
        let file = dir.path().join("herdr/acp/restore/tmux").join(id.as_str());
        run("shell", "shell");
        run("chat", "chat");
        assert_eq!(
            fs::metadata(file.with_extension("chat"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(run("read", ""), "");
        fs::write(file.with_extension("shell"), "old-boot\nshell\n").unwrap();
        fs::write(file.with_extension("chat"), "old-boot\nchat\n").unwrap();
        assert_eq!(run("read", ""), "chat\n");
        run("claim", "chat");
        assert_eq!(run("read", ""), "");
        run("close-chat", "");
        assert_eq!(run("read", ""), "shell\n");
        run("forget", "");
        assert_eq!(run("read", ""), "");
    }

    #[test]
    fn resume_requires_the_original_id_helper_and_provider_session() {
        let id = Tmux::named_id("chat").unwrap();
        let endpoint = EndpointId::random();
        let saved = || Saved {
            id: id.to_string(),
            cwd: Some("/work".into()),
            name: None,
            labels: [
                (sheprd_session::WORKSPACE_LABEL.into(), "desk".into()),
                ("stale-owner".into(), "old".into()),
            ]
            .into(),
            chat: Some(ChatRecovery {
                agent: Agent::Codex,
                adapter: true,
                args: vec!["--profile".into(), "work".into()],
                endpoint: endpoint.to_string(),
                session: "known-session".into(),
                helper: "/tools/sheprd-acp".into(),
                directories: [("CODEX_HOME".into(), "/data/codex".into())].into(),
            }),
        };
        let spawn = saved().spawn(&id).unwrap();
        assert_eq!(
            spawn.argv,
            [
                "/tools/sheprd-acp",
                "--acp",
                "--resume",
                "known-session",
                "codex",
                "--",
                "--profile",
                "work"
            ]
        );
        assert!(
            spawn
                .env
                .contains(&("CODEX_HOME".into(), "/data/codex".into()))
        );
        assert_eq!(spawn.labels[TOKEN], endpoint.as_str());
        assert_eq!(spawn.labels[sheprd_session::WORKSPACE_LABEL], "desk");
        assert!(!spawn.labels.contains_key("stale-owner"));
        assert!(saved().spawn(&Tmux::named_id("other").unwrap()).is_err());
        let mut bad = saved();
        bad.chat.as_mut().unwrap().session = "--new".into();
        assert!(bad.spawn(&id).is_err());
        let mut bad = saved();
        bad.chat.as_mut().unwrap().helper = "/bin/sh".into();
        assert!(bad.spawn(&id).is_err());
    }
}
