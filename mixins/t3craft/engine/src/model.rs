//! The mod's view of T3 Code: environments, projects, threads and what asks
//! for attention. A Rust port of the Java mod's `t3/T3Api.kt` types.

use chrono::{DateTime, Utc};

/// Which group a thread sorts into, worst first (the Java mod's
/// `ThreadAttention`). `Ord` is by urgency for sorting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Attention {
    NeedsInput,
    Running,
    Ready,
    Quiet,
}

/// The thread's status word (the T3 thread's `status`) as an attention group.
pub fn attention_for(status: &str) -> Attention {
    match status {
        "waiting" => Attention::NeedsInput,
        "preparing" | "queued" | "starting" | "running" => Attention::Running,
        "completed" | "failed" | "interrupted" | "cancelled" => Attention::Ready,
        _ => Attention::Quiet,
    }
}

/// A machine T3 Code runs on. `id` is None for the machine the game runs on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Environment {
    pub id: Option<String>,
    pub label: String,
}

impl Environment {
    pub fn is_remote(&self) -> bool {
        self.id.is_some()
    }
}

#[derive(Clone, Debug)]
pub struct Project {
    pub id: String,
    pub title: String,
    pub environment: Environment,
}

/// Where a thread lives: its environment (None = this machine) and its id.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ThreadRef {
    pub environment_id: Option<String>,
    pub thread_id: String,
}

#[derive(Clone, Debug)]
pub struct ThreadSummary {
    pub thread_ref: ThreadRef,
    pub project_title: String,
    pub environment: Environment,
    pub title: String,
    pub status: String,
    pub updated_at: Option<DateTime<Utc>>,
    pub model: String,
    pub runtime_mode: String,
    pub latest_run_id: Option<String>,
    pub run_started_at: Option<DateTime<Utc>>,
}

impl ThreadSummary {
    pub fn attention(&self) -> Attention {
        attention_for(&self.status)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Clone, Debug)]
pub struct ThreadMessage {
    pub role: MessageRole,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RunTiming {
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct ThreadDetail {
    pub thread_ref: ThreadRef,
    pub title: String,
    pub status: String,
    pub pending_request_count: i64,
    pub model: String,
    pub runtime_mode: String,
    pub branch: String,
    pub latest_run: Option<RunTiming>,
    pub messages: Vec<ThreadMessage>,
}

/// Where the connection to T3 Code stands (the Java mod's `ConnectionState`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connection {
    SignedOut,
    SigningIn,
    Connecting,
    Connected,
    Unreachable,
    SignInExpired,
}

/// What the monitor last published: the connection and the open threads.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub connection: Option<Connection>,
    pub threads: Vec<ThreadSummary>,
    pub error: Option<String>,
}

impl Snapshot {
    pub fn connection(&self) -> Connection {
        self.connection.unwrap_or(Connection::SignedOut)
    }

    pub fn threads_with(&self, attention: Attention) -> impl Iterator<Item = &ThreadSummary> {
        self.threads.iter().filter(move |t| t.attention() == attention)
    }
}

/// A thread moved into a state worth telling the player about.
#[derive(Clone, Debug)]
pub struct Transition {
    pub thread: ThreadSummary,
    pub previous: Option<Attention>,
}
