//! t3craft for Horizon: see, read and reply to your T3 Code threads without
//! leaving the game. A Rust port of the Fabric mod `t3craft`
//! (github.com/addidotlol/t3craft) to Horizon's native mod hooks.
//!
//! This crate is the bundle's `[[crate]]`: all of the logic lives here and is
//! unit-tested with `cargo test`; the mixin's helpers are thin and only hand
//! game calls across this API (drawing commands out, input events in).

mod api;
mod config;
mod format;
mod mcp;
mod model;
mod monitor;
mod oauth;
mod ui;

pub use config::{Config, HudAnchor};
pub use model::{Attention, Connection, Snapshot, ThreadRef};

use config::{now_millis, ConfigStore};
use monitor::{Command, Event, Monitor};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};
pub use ui::DrawCmd;
use ui::{Action, ScreenUi, Toast};

/// The mod's id, as Options > Mods and the settings folder name it.
pub const ID: &str = "t3craft";
pub const NAME: &str = "T3 Craft";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DESCRIPTION: &str =
    "See, read and reply to your T3 Code threads without leaving the game.";

/// Its two rebindable keys: name, the label Options shows, the default.
pub const KEY_OPEN: &str = "open_threads";
pub const KEY_OPEN_LABEL: &str = "Open T3 Code threads";
pub const KEY_OPEN_DEFAULT: &str = "key.keyboard.y";
pub const KEY_HUD: &str = "toggle_hud";
pub const KEY_HUD_LABEL: &str = "Toggle T3 Code HUD";
pub const KEY_HUD_DEFAULT: &str = "key.keyboard.unknown";

/// Everything the helpers need, built once (`ensure_started`).
struct Engine {
    store: Mutex<ConfigStore>,
    config: RwLock<Config>,
    ui: Mutex<ScreenUi>,
    monitor: Monitor,
    events: Mutex<mpsc::Receiver<Event>>,
    effects: Mutex<Effects>,
    /// The key the open key was last bound as, so a release matches.
    hud_hidden_by_key: Mutex<bool>,
    toasts: Mutex<Vec<Toast>>,
    started: Mutex<bool>,
}

/// What `tick` hands back for the game to do (opening/closing the screen and
/// printing chat lines need `&mut Game`, which this crate cannot name).
#[derive(Default, Clone)]
pub struct Effects {
    pub open_screen: bool,
    pub close_screen: bool,
    pub chat: Vec<String>,
}

static ENGINE: OnceLock<Engine> = OnceLock::new();

/// Start the monitor once, with the config dir the game gives (`native_config_dir`).
/// Safe to call every tick; only the first call does anything.
pub fn ensure_started(config_dir: PathBuf) -> bool {
    let engine = ENGINE.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Event>();
        let monitor = Monitor::spawn(tx);
        let directory = config_dir.join("t3craft");
        let store = ConfigStore::load(directory);
        let config = store.config.clone();
        monitor.send(Command::SetPollSeconds(config.poll_seconds as i64));
        Engine {
            store: Mutex::new(store),
            config: RwLock::new(config),
            ui: Mutex::new(ScreenUi::default()),
            monitor,
            events: Mutex::new(rx),
            effects: Mutex::new(Effects::default()),
            hud_hidden_by_key: Mutex::new(false),
            toasts: Mutex::new(Vec::new()),
            started: Mutex::new(false),
        }
    });
    // First call only: connect with the saved credential.
    let mut started = engine.started.lock().unwrap();
    if *started {
        return false;
    }
    *started = true;
    let credential = engine.store.lock().unwrap().credential.clone();
    let server = engine.config.read().unwrap().normalized_server_url();
    let usable = credential
        .as_ref()
        .filter(|c| c.is_usable_for(&server, now_millis()))
        .cloned();
    let had_expired = credential.is_some() && usable.is_none();
    engine.monitor.send(Command::Connect(usable));
    if had_expired {
        push_chat(engine, "T3 Craft: sign-in expired — open the threads screen to sign in again.");
    }
    true
}

/// Per client tick (`native_tick`): drain monitor events, expire toasts, and
/// hand the game what it must do. `our_screen_open` is whether the mod's own
/// screen is the open one (`native_mod_screen`), so the reply focus resets
/// when it closes.
pub fn tick(our_screen_open: bool) -> Effects {
    let Some(engine) = ENGINE.get() else {
        return Effects::default();
    };
    drain_events(engine);
    {
        let mut ui = engine.ui.lock().unwrap();
        if !our_screen_open {
            ui.reply_focused = false;
            ui.sending = false;
        }
        ui.keep_selection_in(&engine.monitor.snapshot().threads);
    }
    // Expire toasts.
    if let Ok(mut toasts) = engine.toasts.lock() {
        let now = Instant::now();
        toasts.retain(|t| t.until > now);
    }
    std::mem::take(&mut *engine.effects.lock().unwrap())
}

// ---- monitor events -> state + effects ----

fn drain_events(engine: &Engine) {
    let rx = engine.events.lock().unwrap();
    let pending: Vec<Event> = rx.try_iter().collect();
    drop(rx);
    for event in pending {
        match event {
            Event::Snapshot => {}
            Event::Transitions(transitions) => {
                let notify = engine.config.read().unwrap().notify_on_change;
                if notify {
                    let mut toasts = engine.toasts.lock().unwrap();
                    for transition in transitions.iter().take(3) {
                        let (title, color) = match transition.thread.attention() {
                            Attention::NeedsInput => ("A T3 thread needs you", ui::attention_color(Attention::NeedsInput)),
                            _ => ("A T3 thread finished", ui::attention_color(Attention::Ready)),
                        };
                        toasts.push(Toast {
                            title: title.to_owned(),
                            line: transition.thread.title.clone(),
                            color,
                            until: Instant::now() + Duration::from_secs(6),
                        });
                    }
                    if transitions.len() > 3 {
                        toasts.push(Toast {
                            title: "T3 Code".to_owned(),
                            line: format!("{} threads changed", transitions.len()),
                            color: ui::TEXT_MUTED,
                            until: Instant::now() + Duration::from_secs(6),
                        });
                    }
                }
            }
            Event::AuthorizeUrl(url) => {
                engine.ui.lock().unwrap().auth_url = Some(url.clone());
                // Best effort: open the desktop browser; the URL stays on the
                // sign-in pane for machines with no browser.
                if let Err(e) = open::that_detached(&url) {
                    push_chat(engine, &format!("T3 Craft: open this link to sign in: {url} ({e})"));
                } else {
                    push_chat(engine, "T3 Craft: approve the sign-in in the browser tab that just opened.");
                }
            }
            Event::SignedIn(credential) => {
                engine.store.lock().unwrap().save_credential(Some(credential));
                engine.ui.lock().unwrap().auth_url = None;
                push_chat(engine, "T3 Craft: connected to T3 Code.");
            }
            Event::SignInFailed(message) => {
                engine.ui.lock().unwrap().auth_url = None;
                push_chat(engine, &format!("T3 Craft: {message}"));
            }
            Event::CredentialRejected => {
                engine.store.lock().unwrap().save_credential(None);
            }
            Event::DetailLoaded(thread_ref, result) => {
                let mut ui = engine.ui.lock().unwrap();
                if ui.selected.as_ref() == Some(&thread_ref) {
                    ui.detail_loading = false;
                    match result {
                        Ok(detail) => {
                            ui.detail = Some(detail);
                            ui.detail_error = None;
                            // A thread opens on its latest messages: the next
                            // layout snaps the pane to the bottom.
                            ui.detail_scroll_to_end = true;
                        }
                        Err(e) => {
                            ui.detail_error = Some(e);
                        }
                    }
                }
            }
            Event::Sent(thread_ref, result) => {
                let mut ui = engine.ui.lock().unwrap();
                if ui.selected.as_ref() == Some(&thread_ref) {
                    ui.sending = false;
                    match result {
                        Ok(()) => {
                            ui.reply.clear();
                            ui.reply_cursor = 0;
                            ui.send_error = None;
                        }
                        Err(e) => ui.send_error = Some(format!("Couldn't send: {e}")),
                    }
                }
            }
        }
    }
}

fn push_chat(engine: &Engine, line: &str) {
    engine.effects.lock().unwrap().chat.push(line.to_owned());
}

// ---- keys (native_key, after the helper checks the binding) ----

/// The open-threads key went down: ask for the screen.
pub fn open_threads() {
    if let Some(engine) = ENGINE.get() {
        engine.effects.lock().unwrap().open_screen = true;
    }
}

/// The toggle-HUD key went down.
pub fn toggle_hud() {
    if let Some(engine) = ENGINE.get() {
        let mut hidden = engine.hud_hidden_by_key.lock().unwrap();
        *hidden = !*hidden;
    }
}

// ---- the HUD (native_hud) ----

/// The corner box's contents, or None when hidden (setting off or
/// key-toggled). `measure` is `native_text_width`, so rows can be cut to
/// the box's real width.
pub fn hud_model(measure: &dyn Fn(&str) -> f32) -> Option<ui::HudModel> {
    let engine = ENGINE.get()?;
    let config = engine.config.read().unwrap().clone();
    if !config.hud_enabled || *engine.hud_hidden_by_key.lock().unwrap() {
        return None;
    }
    let snapshot = engine.monitor.snapshot();
    let rows = ui::hud_rows(&snapshot, &config, measure);
    let toast = engine
        .toasts
        .lock()
        .unwrap()
        .first()
        .map(|t| {
            (
                t.title.clone(),
                ui::ellipsize(&t.line, 220.0, measure),
                t.color,
            )
        });
    Some(ui::HudModel {
        anchor: config.hud_anchor,
        offset: [config.hud_offset_x as f32, config.hud_offset_y as f32],
        scale: config.hud_scale,
        rows,
        toast,
    })
}

// ---- the mod's own screen ----

/// Whether our screen is the one open (`native_mod_screen`) — the helpers ask
/// this before feeding input.
pub fn is_signing_in() -> bool {
    ENGINE
        .get()
        .map(|e| e.monitor.snapshot().connection() == Connection::SigningIn)
        .unwrap_or(false)
}

/// Draw the screen: commands the helper renders with `native_fill`/`native_text`.
/// Also records the hit regions a later click reads. `measure` is
/// `native_text_width`, `rect` the screen's, `mouse` the cursor.
pub fn screen_commands(
    measure: &dyn Fn(&str) -> f32,
    rect: [f32; 4],
    mouse: [f32; 2],
) -> Vec<DrawCmd> {
    let Some(engine) = ENGINE.get() else {
        return Vec::new();
    };
    let snapshot = engine.monitor.snapshot();
    let config = engine.config.read().unwrap().clone();
    let mut ui = engine.ui.lock().unwrap();
    ui::layout_screen(&mut ui, &snapshot, &config, measure, rect, mouse)
}

/// A mouse press on our screen (`native_screen_mouse_button_pressed_pre`).
/// True when it hit something ours — the press goes no further either way on
/// a mod screen, so this is really "handled".
pub fn screen_click(at: [f32; 2], right: bool) {
    let Some(engine) = ENGINE.get() else {
        return;
    };
    // Last matching region wins: pane scroll regions are pushed first, the
    // buttons and rows inside them later, so the topmost is found last.
    let action = {
        let ui = engine.ui.lock().unwrap();
        ui.hit
            .iter()
            .rev()
            .find(|h| inside(at, h.rect))
            .map(|h| h.action.clone())
    };
    match action {
        Some(Action::Select(row_index)) => select_row(engine, row_index),
        Some(Action::FocusReply) => {
            engine.ui.lock().unwrap().reply_focused = true;
        }
        Some(Action::Send) => send_reply(engine),
        Some(Action::Refresh) => engine.monitor.send(Command::PollNow),
        Some(Action::SignIn) => {
            let server = engine.config.read().unwrap().normalized_server_url();
            engine.monitor.send(Command::SignIn(server));
        }
        Some(Action::SignOut) => {
            engine.store.lock().unwrap().save_credential(None);
            engine.monitor.send(Command::SignOut);
            let mut ui = engine.ui.lock().unwrap();
            ui.detail = None;
            ui.selected = None;
        }
        Some(Action::CancelSignIn) => {
            engine.monitor.send(Command::SignOut);
        }
        Some(Action::ScrollList | Action::ScrollDetail) | None => {
            // Scroll regions also catch clicks: pressing anywhere but a field
            // unfocuses the reply box.
            engine.ui.lock().unwrap().reply_focused = false;
        }
    }
    let _ = right;
}

/// The wheel over our screen (`native_screen_mouse_scrolled_pre`).
pub fn screen_scroll(at: [f32; 2], delta: f64) {
    let Some(engine) = ENGINE.get() else {
        return;
    };
    let mut ui = engine.ui.lock().unwrap();
    // The first scroll region containing the point wins: rows and buttons sit
    // inside their pane's region, so the wheel scrolls wherever it points.
    for region in &ui.hit {
        if !inside(at, region.rect) {
            continue;
        }
        match region.action {
            Action::ScrollList => {
                ui.list_scroll = (ui.list_scroll - delta as f32 * 10.0)
                    .clamp(0.0, ui.list_scroll_max);
                return;
            }
            Action::ScrollDetail => {
                ui.detail_scroll = (ui.detail_scroll - delta as f32 * 10.0)
                    .clamp(0.0, ui.detail_scroll_max);
                return;
            }
            _ => {}
        }
    }
}

/// A named key over our screen (`native_screen_key_pressed_pre`).
/// True when the screen used it (the press goes no further); Escape and every
/// key the screen ignores stay false so the game closes or handles it.
pub fn screen_key(key: &str, repeat: bool, modifiers: u32) -> bool {
    let Some(engine) = ENGINE.get() else {
        return false;
    };
    let shift = modifiers & 1 != 0;
    let mut ui = engine.ui.lock().unwrap();
    if ui.reply_focused {
        match key {
            "key.keyboard.enter" | "key.keyboard.keypad.enter" => {
                if !shift {
                    drop(ui);
                    send_reply(engine);
                } else {
                    insert_char(&mut ui, '\n');
                }
                return true;
            }
            "key.keyboard.backspace" => {
                let cursor = ui.cursor_char();
                if cursor > 0 {
                    let mut chars: Vec<char> = ui.reply.chars().collect();
                    chars.remove(cursor - 1);
                    ui.reply = chars.into_iter().collect();
                    ui.reply_cursor = cursor - 1;
                }
                return true;
            }
            "key.keyboard.delete" => {
                let cursor = ui.cursor_char();
                let mut chars: Vec<char> = ui.reply.chars().collect();
                if cursor < chars.len() {
                    chars.remove(cursor);
                    ui.reply = chars.into_iter().collect();
                }
                return true;
            }
            "key.keyboard.left" => {
                ui.reply_cursor = ui.cursor_char().saturating_sub(1);
                return true;
            }
            "key.keyboard.right" => {
                ui.reply_cursor = (ui.cursor_char() + 1).min(ui.reply.chars().count());
                return true;
            }
            "key.keyboard.home" => {
                ui.reply_cursor = 0;
                return true;
            }
            "key.keyboard.end" => {
                ui.reply_cursor = ui.reply.chars().count();
                return true;
            }
            "key.keyboard.up" => {
                ui.reply_cursor = 0;
                return true;
            }
            "key.keyboard.down" => {
                ui.reply_cursor = ui.reply.chars().count();
                return true;
            }
            "key.keyboard.escape" => {
                // First Escape just drops the focus; the next closes.
                ui.reply_focused = false;
                return true;
            }
            _ => {}
        }
        // A key the box doesn't use stays the game's (Escape, F1…); typing
        // itself arrives as characters, not keys.
        return false;
    }
    // Not in the reply box: up/down walk the list. Escape falls through so
    // the game closes the screen.
    let select = match key {
        "key.keyboard.up" => next_row(engine, &ui, -1),
        "key.keyboard.down" => next_row(engine, &ui, 1),
        _ => None,
    };
    drop(ui);
    let _ = repeat;
    if let Some(row) = select {
        select_row(engine, row);
        return true;
    }
    false
}

/// Printable text on our screen (`native_screen_char_typed_pre`): into the
/// reply box when it's focused.
pub fn screen_char(c: char, _modifiers: u32) -> bool {
    let Some(engine) = ENGINE.get() else {
        return false;
    };
    let mut ui = engine.ui.lock().unwrap();
    if !ui.reply_focused || c.is_control() {
        return false;
    }
    insert_char(&mut ui, c);
    true
}

fn insert_char(ui: &mut ScreenUi, c: char) {
    let cursor = ui.cursor_char();
    let mut chars: Vec<char> = ui.reply.chars().collect();
    chars.insert(cursor, c);
    ui.reply = chars.into_iter().collect();
    ui.reply_cursor = cursor + 1;
}

/// The list row `delta` steps from the selection (Down from none picks the
/// first thread row), for `select_row` once the lock is dropped.
fn next_row(engine: &Engine, ui: &ScreenUi, delta: i32) -> Option<usize> {
    let thread_rows: Vec<usize> = ui
        .list_rows
        .iter()
        .enumerate()
        .filter_map(|(i, r)| match r {
            ui::ListRow::Thread(_) => Some(i),
            _ => None,
        })
        .collect();
    if thread_rows.is_empty() {
        return None;
    }
    let snapshot = engine.monitor.snapshot();
    let current = ui.selected.as_ref().and_then(|sel| {
        ui.list_rows.iter().enumerate().find_map(|(i, r)| match r {
            ui::ListRow::Thread(index)
                if snapshot.threads.get(*index).map(|t| &t.thread_ref) == Some(sel) =>
            {
                Some(i)
            }
            _ => None,
        })
    });
    let position = current
        .and_then(|c| thread_rows.iter().position(|r| *r == c))
        .map(|p| p as i32)
        .unwrap_or(if delta > 0 { -1 } else { thread_rows.len() as i32 });
    let next = (position + delta).clamp(0, thread_rows.len() as i32 - 1) as usize;
    Some(thread_rows[next])
}

/// A list row clicked or walked to: select it and load its messages.
fn select_row(engine: &Engine, row_index: usize) {
    let thread_ref = {
        let ui = engine.ui.lock().unwrap();
        let Some(ui::ListRow::Thread(index)) = ui.list_rows.get(row_index) else {
            return;
        };
        let index = *index;
        engine
            .monitor
            .snapshot()
            .threads
            .get(index)
            .map(|t| t.thread_ref.clone())
    };
    let Some(thread_ref) = thread_ref else {
        return;
    };
    let mut ui = engine.ui.lock().unwrap();
    if ui.selected.as_ref() != Some(&thread_ref) {
        ui.selected = Some(thread_ref.clone());
        ui.detail = None;
        ui.detail_error = None;
        ui.detail_loading = true;
        ui.detail_scroll = 0.0;
        ui.send_error = None;
        drop(ui);
        engine.monitor.send(Command::ReadThread(thread_ref));
    }
}

fn send_reply(engine: &Engine) {
    let (thread_ref, message) = {
        let ui = engine.ui.lock().unwrap();
        (ui.selected.clone(), ui.reply.trim().to_owned())
    };
    let (Some(thread_ref), message) = (thread_ref, message) else {
        return;
    };
    if message.is_empty() {
        return;
    }
    engine.ui.lock().unwrap().sending = true;
    engine
        .monitor
        .send(Command::SendMessage(thread_ref, message));
}

// ---- Options > Mods' settings page ----

/// The buttons the settings page shows, labelled with current values.
pub fn settings_labels() -> Vec<String> {
    let Some(engine) = ENGINE.get() else {
        return Vec::new();
    };
    let config = engine.config.read().unwrap();
    let on = |b: bool| if b { "ON" } else { "OFF" };
    vec![
        format!("Show HUD: {}", on(config.hud_enabled)),
        format!("Corner: {}", config.hud_anchor.label()),
        format!("HUD scale: {:.1}", config.hud_scale),
        format!("HUD rows: {}", config.hud_max_rows),
        format!("Show finished: {}", on(config.show_ready_threads)),
        format!("Toasts: {}", on(config.notify_on_change)),
        format!("Poll every: {}s", config.poll_seconds),
        format!("Server: {}", config.normalized_server_url()),
        "Reset server URL to discovered".to_owned(),
    ]
}

/// A click on the page's `i`-th button: change the setting and save it.
pub fn settings_click(index: usize) {
    let Some(engine) = ENGINE.get() else {
        return;
    };
    {
        let mut config = engine.config.write().unwrap();
        match index {
            0 => config.hud_enabled = !config.hud_enabled,
            1 => {
                let next = (config
                    .hud_anchor as usize + 1)
                    % HudAnchor::ALL.len();
                config.hud_anchor = HudAnchor::ALL[next];
            }
            2 => {
                config.hud_scale = if config.hud_scale >= 2.0 { 0.5 } else { config.hud_scale + 0.5 };
            }
            3 => config.hud_max_rows = if config.hud_max_rows >= 20 { 1 } else { config.hud_max_rows + 1 },
            4 => config.show_ready_threads = !config.show_ready_threads,
            5 => config.notify_on_change = !config.notify_on_change,
            6 => config.poll_seconds = if config.poll_seconds >= 60 { 2 } else { config.poll_seconds + 1 },
            7 => {} // the URL itself: shown, changed below
            8 => config.server_url = config::discover_local_server_url(),
            _ => return,
        }
        let config = config.clone();
        engine.store.lock().unwrap().config = config;
        engine.store.lock().unwrap().save();
    }
    // Poll interval is live on the monitor.
    let seconds = engine.config.read().unwrap().poll_seconds;
    engine.monitor.send(Command::SetPollSeconds(seconds as i64));
}

fn inside(at: [f32; 2], rect: [f32; 4]) -> bool {
    at[0] >= rect[0] && at[0] < rect[0] + rect[2] && at[1] >= rect[1] && at[1] < rect[1] + rect[3]
}
