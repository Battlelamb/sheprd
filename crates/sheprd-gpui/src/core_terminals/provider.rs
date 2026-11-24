//! Select once at creation. Restore, mirror, relink, and close always use the
//! saved terminal's identity, even after the preference changes.
use super::{CoreTerminals, Error, Host, worker};
use crate::config::TerminalBackend;
use sheprd_backend::{Backend, Role, Size, Spawn, TerminalId, TerminalInfo};
use sheprd_core_client::CoreBackend;
#[cfg(feature = "tmux")]
use sheprd_tmux::Tmux;

#[derive(Clone)]
pub(super) enum Provider {
    Core(CoreBackend),
    #[cfg(feature = "tmux")]
    Tmux(Tmux),
}

impl CoreTerminals {
    #[cfg(feature = "tmux")]
    pub(super) fn tmux_on(&self, host: &Host, scrollback: usize) -> Option<Provider> {
        let tmux = Tmux::new(scrollback);
        match host {
            Host::Local => Some(tmux),
            Host::Remote(scope) => sheprd_remote::tmux::on_host(tmux, self.remote_target(scope)?),
        }
        .map(Provider::Tmux)
    }

    #[cfg(not(feature = "tmux"))]
    pub(super) fn tmux_on(&self, _host: &Host, _scrollback: usize) -> Option<Provider> {
        None
    }

    pub(super) fn provider(
        &self,
        host: &Host,
        backend: TerminalBackend,
        scrollback: usize,
    ) -> Option<Provider> {
        if backend == TerminalBackend::Tmux {
            return self.tmux_on(host, scrollback);
        }
        self.backend_of(host).cloned().map(Provider::Core)
    }

    pub(super) fn owner(
        &self,
        host: &Host,
        id: &TerminalId,
        scrollback: usize,
    ) -> Option<Provider> {
        self.provider(host, TerminalBackend::owner(id), scrollback)
    }
}

impl Provider {
    pub(super) fn terminals(&self) -> Result<Vec<TerminalInfo>, Error> {
        match self {
            Self::Core(core) => Ok(core.terminals()?),
            #[cfg(feature = "tmux")]
            Self::Tmux(tmux) => Ok(tmux.terminals()?),
        }
    }

    pub(super) fn spawn(&self, spawn: &Spawn) -> Result<TerminalInfo, Error> {
        match self {
            Self::Core(core) => Ok(core.spawn(spawn)?),
            #[cfg(feature = "tmux")]
            Self::Tmux(tmux) => Ok(tmux.spawn(spawn)?),
        }
    }

    pub(super) fn link(
        self,
        id: TerminalId,
        size: Size,
        role: Role,
    ) -> std::io::Result<worker::Link> {
        match self {
            Self::Core(core) => worker::spawn(core, id, size, role),
            #[cfg(feature = "tmux")]
            Self::Tmux(tmux) => worker::spawn(tmux, id, size, role),
        }
    }

    pub(super) fn kill(&self, id: &TerminalId) -> Result<(), Error> {
        match self {
            Self::Core(core) => Ok(core.kill(id)?),
            #[cfg(feature = "tmux")]
            Self::Tmux(tmux) => Ok(tmux.kill(id)?),
        }
    }
}
