//! What the index keeps: sessions with their totals, the bounded events of
//! each, and per-day activity for charts.
use serde::{Serialize, Serializer};
use sheprd_client::herdr::AgentKind;

/// What one event of a session is.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// A person's prompt.
    User,
    /// The agent's reply text.
    Agent,
    /// The agent's reasoning or its summary.
    Thinking,
    /// A tool the agent called, with its input.
    ToolCall,
    /// What a tool returned.
    ToolResult,
    /// A failed tool call or an error the agent reported.
    Error,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Agent => "agent",
            Self::Thinking => "thinking",
            Self::ToolCall => "tool_call",
            Self::ToolResult => "tool_result",
            Self::Error => "error",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "user" => Self::User,
            "agent" => Self::Agent,
            "thinking" => Self::Thinking,
            "tool_call" => Self::ToolCall,
            "tool_result" => Self::ToolResult,
            "error" => Self::Error,
            _ => return None,
        })
    }
}

/// Who may see a session. Only the person, in the app, changes it; the
/// index records `Private` for everything it adds.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    #[default]
    Private,
    Link,
    Public,
}

impl Visibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Link => "link",
            Self::Public => "public",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "private" => Self::Private,
            "link" => Self::Link,
            "public" => Self::Public,
            _ => return None,
        })
    }
}

/// One bounded, redacted event. `seq` orders events within their session
/// and stays the same when the file is read again, so adding it twice is a
/// no-op.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Event {
    pub seq: i64,
    /// Unix milliseconds, when the record carried a time.
    pub at: Option<i64>,
    pub kind: EventKind,
    pub tool: Option<String>,
    pub text: String,
}

/// One indexed session with its totals.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Session {
    pub id: String,
    pub host_id: String,
    pub host: String,
    #[serde(serialize_with = "agent_name")]
    pub agent: AgentKind,
    pub native_id: String,
    pub user: String,
    pub title: Option<String>,
    pub model: Option<String>,
    pub cwd: Option<String>,
    pub branch: Option<String>,
    pub agent_version: Option<String>,
    pub started_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub messages: i64,
    pub user_messages: i64,
    pub tool_calls: i64,
    pub errors: i64,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub visibility: Visibility,
}

/// Activity of one agent on one UTC day.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Activity {
    /// `YYYY-MM-DD`, UTC.
    pub day: String,
    #[serde(serialize_with = "agent_name")]
    pub agent: AgentKind,
    pub sessions: i64,
    pub messages: i64,
}

fn agent_name<S: Serializer>(agent: &AgentKind, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(agent.name())
}
