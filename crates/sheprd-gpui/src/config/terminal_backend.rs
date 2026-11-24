/// The user's choice for new terminals. With no explicit
/// choice the older herdr_2_terminals setting still selects Herdr 1 or 2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
pub enum TerminalBackend {
    #[serde(rename = "herdr")]
    Herdr,
    #[serde(rename = "herdr-core")]
    Core,
    #[serde(rename = "tmux")]
    Tmux,
}

impl TerminalBackend {
    pub(crate) fn available(self) -> bool {
        self != Self::Tmux || cfg!(feature = "tmux")
    }

    pub(crate) fn owner(id: &sheprd_backend::TerminalId) -> Self {
        if id.is_tmux() { Self::Tmux } else { Self::Core }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Herdr => "herdr",
            Self::Core => "herdr-core",
            Self::Tmux => "tmux",
        }
    }
}
