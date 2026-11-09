//! The polling thread, a port of `t3/ThreadMonitor.kt`: one background worker
//! that owns the MCP connection, refreshes the open threads every
//! `poll_seconds`, and posts what changed for the game to read.

use crate::api::T3Api;
use crate::config::StoredCredential;
use crate::mcp::{describe, McpClient, McpError};
use crate::model::*;
use crate::oauth;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, RwLock};
use std::time::{Duration, Instant};

const PROJECT_REFRESH: Duration = Duration::from_secs(60);
const MOD_VERSION: &str = env!("CARGO_PKG_VERSION");

/// What the game thread asks the monitor.
pub enum Command {
    /// Try the saved token; `None` goes straight to signed out.
    Connect(Option<StoredCredential>),
    /// OAuth: register, hand back a URL, wait for the callback.
    SignIn(String),
    SignOut,
    PollNow,
    SetPollSeconds(i64),
    ReadThread(ThreadRef),
    SendMessage(ThreadRef, String),
    Shutdown,
}

/// What the monitor tells the game thread (drained in `tick`).
pub enum Event {
    /// `snapshot` changed; re-read it.
    Snapshot,
    /// Threads newly needing you or newly finished (for toasts).
    Transitions(Vec<Transition>),
    /// Sign-in wants a browser: this URL. The engine opens it and shows it.
    AuthorizeUrl(String),
    /// OAuth finished: keep this credential.
    SignedIn(StoredCredential),
    /// OAuth did not finish.
    SignInFailed(String),
    /// The saved credential was refused; forget it.
    CredentialRejected,
    /// `read_thread`'s answer for the open thread page.
    DetailLoaded(ThreadRef, Result<ThreadDetail, String>),
    /// `send_message`'s answer.
    Sent(ThreadRef, Result<(), String>),
}

pub struct Monitor {
    tx: mpsc::Sender<Command>,
    snapshot: Arc<RwLock<Snapshot>>,
    stop: Arc<AtomicBool>,
}

impl Monitor {
    pub fn spawn(events: mpsc::Sender<Event>) -> Self {
        let (tx, rx) = mpsc::channel::<Command>();
        let snapshot = Arc::new(RwLock::new(Snapshot::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker = Worker {
            rx,
            events,
            snapshot: Arc::clone(&snapshot),
            stop: Arc::clone(&stop),
            mcp: None,
            pending_login: None,
            projects: Vec::new(),
            run_starts: HashMap::new(),
            projects_fetched_at: Instant::now() - PROJECT_REFRESH,
            has_baseline: false,
            poll_seconds: 5,
            next_poll: Instant::now(),
        };
        let _ = std::thread::Builder::new()
            .name("t3craft-monitor".into())
            .spawn(move || worker.run());
        Self { tx, snapshot, stop }
    }

    pub fn send(&self, command: Command) {
        let _ = self.tx.send(command);
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.read().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.tx.send(Command::Shutdown);
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct Worker {
    rx: mpsc::Receiver<Command>,
    events: mpsc::Sender<Event>,
    snapshot: Arc<RwLock<Snapshot>>,
    stop: Arc<AtomicBool>,
    mcp: Option<McpClient>,
    pending_login: Option<oauth::PendingLogin>,
    projects: Vec<(Environment, Vec<Project>)>,
    run_starts: HashMap<(ThreadRef, String), chrono::DateTime<chrono::Utc>>,
    projects_fetched_at: Instant,
    has_baseline: bool,
    poll_seconds: i64,
    next_poll: Instant,
}

impl Worker {
    fn run(mut self) {
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return;
            }
            let timeout = self
                .next_poll
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(250));
            match self.rx.recv_timeout(timeout) {
                Ok(command) => self.handle(command),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
            self.check_login();
            if self.mcp.is_some() && Instant::now() >= self.next_poll {
                self.poll();
                self.next_poll = Instant::now()
                    + Duration::from_secs(self.poll_seconds.max(2) as u64);
            }
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Connect(credential) => match credential {
                Some(credential) => self.connect(credential),
                None => {
                    self.disconnect();
                    self.publish_snapshot(Snapshot {
                        connection: Some(Connection::SignedOut),
                        ..Snapshot::default()
                    });
                }
            },
            Command::SignIn(server_url) => self.sign_in(&server_url),
            Command::SignOut => {
                if let Some(login) = self.pending_login.take() {
                    login.cancel();
                }
                self.disconnect();
                self.publish_snapshot(Snapshot {
                    connection: Some(Connection::SignedOut),
                    ..Snapshot::default()
                });
            }
            Command::PollNow => {
                if self.mcp.is_some() {
                    self.poll();
                    self.next_poll = Instant::now()
                        + Duration::from_secs(self.poll_seconds.max(2) as u64);
                }
            }
            Command::SetPollSeconds(seconds) => self.poll_seconds = seconds.max(2),
            Command::ReadThread(thread_ref) => {
                let detail = match Self::api_of(&mut self.mcp) {
                    Some(mut api) => api
                        .read_thread(&thread_ref, 40)
                        .map_err(|e| describe(&e)),
                    None => Err("Not connected to T3 Code.".to_owned()),
                };
                let _ = self.events.send(Event::DetailLoaded(thread_ref, detail));
            }
            Command::SendMessage(thread_ref, message) => {
                let sent = match Self::api_of(&mut self.mcp) {
                    Some(mut api) => api
                        .send_message(&thread_ref, &message)
                        .map_err(|e| describe(&e)),
                    None => Err("Not connected to T3 Code.".to_owned()),
                };
                let failed = sent.is_err();
                let _ = self.events.send(Event::Sent(thread_ref, sent));
                if !failed {
                    self.next_poll = Instant::now();
                }
            }
            Command::Shutdown => self.stop.store(true, Ordering::Relaxed),
        }
    }

    // ---- connection ----

    /// The API over the live client, as a borrow of the `mcp` field only, so
    /// the rest of `self` stays reachable while it is alive.
    fn api_of(mcp: &mut Option<McpClient>) -> Option<T3Api<'_>> {
        mcp.as_mut().map(T3Api::new)
    }

    fn connect(&mut self, credential: StoredCredential) {
        self.disconnect();
        self.mcp = Some(McpClient::new(
            &credential.server_url,
            &credential.access_token,
            MOD_VERSION,
        ));
        self.publish_snapshot(Snapshot {
            connection: Some(Connection::Connecting),
            ..Snapshot::default()
        });
        self.next_poll = Instant::now();
    }

    fn disconnect(&mut self) {
        if let Some(mut mcp) = self.mcp.take() {
            mcp.close();
        }
        self.projects.clear();
        self.run_starts.clear();
        self.has_baseline = false;
    }

    fn sign_in(&mut self, server_url: &str) {
        if let Some(login) = self.pending_login.take() {
            login.cancel();
        }
        match oauth::start(server_url) {
            Ok(login) => {
                let _ = self
                    .events
                    .send(Event::AuthorizeUrl(login.authorize_url.clone()));
                self.pending_login = Some(login);
                self.publish_snapshot(Snapshot {
                    connection: Some(Connection::SigningIn),
                    ..Snapshot::default()
                });
            }
            Err(e) => {
                self.publish_snapshot(Snapshot {
                    connection: Some(Connection::SignedOut),
                    error: Some(e.clone()),
                    ..Snapshot::default()
                });
                let _ = self.events.send(Event::SignInFailed(e));
            }
        }
    }

    fn check_login(&mut self) {
        let Some(login) = &self.pending_login else {
            return;
        };
        match login.result.try_recv() {
            Ok(Ok(credential)) => {
                self.pending_login = None;
                let _ = self.events.send(Event::SignedIn(credential.clone()));
                self.connect(credential);
            }
            Ok(Err(e)) => {
                self.pending_login = None;
                self.publish_snapshot(Snapshot {
                    connection: Some(Connection::SignedOut),
                    error: Some(e.clone()),
                    ..Snapshot::default()
                });
                let _ = self.events.send(Event::SignInFailed(e));
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.pending_login = None;
            }
        }
    }

    // ---- polling (ThreadMonitor.poll) ----

    fn poll(&mut self) {
        if self.projects_fetched_at.elapsed() > PROJECT_REFRESH {
            let discovered = match Self::api_of(&mut self.mcp) {
                Some(mut api) => Self::discover_projects(&mut api),
                None => return,
            };
            match discovered {
                Ok(projects) => {
                    self.projects = projects;
                    self.projects_fetched_at = Instant::now();
                }
                Err(McpError::Unauthorized) => {
                    self.rejected();
                    return;
                }
                Err(e) => {
                    self.publish_snapshot(Snapshot {
                        connection: Some(Connection::Unreachable),
                        error: Some(describe(&e)),
                        ..self.snapshot()
                    });
                    return;
                }
            }
        }
        // The thread list: every project's open threads. The api borrow is
        // scoped so `self` methods stay callable around it.
        let mut threads = Vec::new();
        let mut failed: Option<McpError> = None;
        let mut unauthorized = false;
        {
            let Some(mut api) = Self::api_of(&mut self.mcp) else {
                return;
            };
            'outer: for (environment, projects) in &self.projects {
                for project in projects {
                    match api.list_open_threads(project) {
                        Ok(mut list) => threads.append(&mut list),
                        Err(e) => {
                            if matches!(e, McpError::Unauthorized) {
                                unauthorized = true;
                                break 'outer;
                            }
                            if !environment.is_remote() {
                                failed = Some(e);
                                break 'outer;
                            }
                            // A remote that went away this poll is skipped.
                        }
                    }
                }
            }
            if !unauthorized && failed.is_none() {
                threads.sort_by(|a, b| {
                    a.attention()
                        .cmp(&b.attention())
                        .then(b.updated_at.cmp(&a.updated_at))
                });
                threads = Self::with_run_starts(&mut self.run_starts, &mut api, threads);
            }
        }
        if unauthorized {
            self.rejected();
            return;
        }
        if let Some(e) = failed {
            self.publish_snapshot(Snapshot {
                connection: Some(Connection::Unreachable),
                error: Some(describe(&e)),
                ..self.snapshot()
            });
            return;
        }
        self.publish(threads);
    }

    /// Run start times for running threads, like `withRunStarts`: known ones
    /// stay, new runs are asked once.
    fn with_run_starts(
        run_starts: &mut HashMap<(ThreadRef, String), chrono::DateTime<chrono::Utc>>,
        api: &mut T3Api<'_>,
        threads: Vec<ThreadSummary>,
    ) -> Vec<ThreadSummary> {
        let known = std::mem::take(run_starts);
        let mut threads = threads;
        for thread in &mut threads {
            let Some(run_id) = thread.latest_run_id.clone() else {
                continue;
            };
            if thread.attention() != Attention::Running {
                continue;
            }
            let key = (thread.thread_ref.clone(), run_id.clone());
            let started_at = known
                .get(&key)
                .copied()
                .or_else(|| api.latest_run_timing(&thread.thread_ref).ok().flatten().and_then(|t| t.started_at));
            if let Some(at) = started_at {
                run_starts.insert(key, at);
                thread.run_started_at = Some(at);
            }
        }
        threads
    }

    fn discover_projects(
        api: &mut T3Api<'_>,
    ) -> Result<Vec<(Environment, Vec<Project>)>, McpError> {
        let local = api.local_environment()?;
        let local_projects = api.list_projects(&local)?;
        let mut all = vec![(local, local_projects)];
        for remote in api.reachable_linked_environments() {
            match api.list_projects(&remote) {
                Ok(projects) => all.push((remote, projects)),
                Err(McpError::Unauthorized) => return Err(McpError::Unauthorized),
                Err(_) => {} // unreachable remotes are skipped this round
            }
        }
        Ok(all)
    }

    /// New snapshot = connected; transitions are the attention changes that
    /// matter: newly needs you, or a running thread that finished.
    fn publish(&mut self, threads: Vec<ThreadSummary>) {
        let previous: HashMap<ThreadRef, Attention> = self
            .snapshot()
            .threads
            .iter()
            .map(|t| (t.thread_ref.clone(), t.attention()))
            .collect();
        let transitions: Vec<Transition> = threads
            .iter()
            .filter_map(|thread| {
                let before = previous.get(&thread.thread_ref).copied();
                let noteworthy = thread.attention() == Attention::NeedsInput
                    || (thread.attention() == Attention::Ready && before == Some(Attention::Running));
                (noteworthy && before != Some(thread.attention())).then(|| Transition {
                    thread: thread.clone(),
                    previous: before,
                })
            })
            .collect();
        self.publish_snapshot(Snapshot {
            connection: Some(Connection::Connected),
            threads,
            error: None,
        });
        if self.has_baseline && !transitions.is_empty() {
            let _ = self.events.send(Event::Transitions(transitions));
        }
        self.has_baseline = true;
    }

    /// 401/403 anywhere: drop the token and ask for sign-in again.
    fn rejected(&mut self) {
        self.disconnect();
        let _ = self.events.send(Event::CredentialRejected);
        self.publish_snapshot(Snapshot {
            connection: Some(Connection::SignInExpired),
            error: Some("T3 Code rejected the saved sign-in.".to_owned()),
            ..Snapshot::default()
        });
    }

    fn publish_snapshot(&self, snapshot: Snapshot) {
        if let Ok(mut current) = self.snapshot.write() {
            *current = snapshot;
        }
        let _ = self.events.send(Event::Snapshot);
    }

    fn snapshot(&self) -> Snapshot {
        self.snapshot.read().map(|s| s.clone()).unwrap_or_default()
    }
}
