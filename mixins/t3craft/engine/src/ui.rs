//! What the game draws and where clicks land. The helpers call `hud_model`
//! every HUD frame and `screen_commands` every screen frame; both hand back
//! plain drawing commands, so this file never names a game type. It is also
//! where mouse, keys and typed text arrive (the screen is a canvas the game
//! owns nothing on).

use crate::config::{Config, HudAnchor};
use crate::model::*;
use std::time::Instant;

// ---- colours, ported from ThreadStyle.kt ----
pub const PANEL_BG: u32 = 0xB0_10_10_14;
pub const PANEL_BORDER: u32 = 0x40_FF_FF_FF;
pub const TEXT_PRIMARY: u32 = 0xF2_F2_F2;
pub const TEXT_MUTED: u32 = 0x9A_9A_A6;
pub const ROW_HOVER: u32 = 0x30_FF_FF_FF;
pub const ROW_SELECTED: u32 = 0x50_FF_FF_FF;
pub const USER_ACCENT: u32 = 0x7F_B7_FF;
pub const ASSISTANT_ACCENT: u32 = 0xC9_A0_FF;
pub const INPUT_BG: u32 = 0xFF_00_00_00;
pub const BUTTON_BG: u32 = 0x60_60_60_60;
pub const BUTTON_ACCENT: u32 = 0xA0_3B_82_F6;

pub fn attention_color(attention: Attention) -> u32 {
    match attention {
        Attention::NeedsInput => 0xFF_B3_47,
        Attention::Running => 0x5A_C8_FA,
        Attention::Ready => 0x7E_E0_81,
        Attention::Quiet => TEXT_MUTED,
    }
}

pub fn status_color(status: &str) -> u32 {
    if status == "failed" {
        0xFF_6B_6B
    } else {
        attention_color(attention_for(status))
    }
}

// ---- draw commands the helpers turn into native_fill/native_text ----

#[derive(Clone, Debug, PartialEq)]
pub enum DrawCmd {
    Fill { rect: [f32; 4], argb: u32 },
    Text { at: [f32; 2], text: String, rgb: u32, shadow: bool },
}

/// A rectangle a click means something on.
#[derive(Clone, Debug)]
pub struct HitRegion {
    pub rect: [f32; 4],
    pub action: Action,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Select(usize),
    FocusReply,
    Send,
    Refresh,
    SignIn,
    SignOut,
    CancelSignIn,
    ScrollList,
    ScrollDetail,
}

pub const LINE_HEIGHT: f32 = 10.0;

/// What the HUD draws: a title line, rows, an optional status line and a
/// toast card while one is fresh.
#[derive(Clone, Debug, Default)]
pub struct HudModel {
    pub anchor: HudAnchor,
    pub offset: [f32; 2],
    pub scale: f32,
    pub rows: Vec<(String, u32)>,
    pub toast: Option<(String, String, u32)>,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub title: String,
    pub line: String,
    pub color: u32,
    pub until: Instant,
}

/// One row of the thread list, sections expanded.
#[derive(Clone, Debug)]
pub enum ListRow {
    Section(&'static str, u32),
    Thread(usize),
}

/// The screen's mutable state: selection, detail, the reply box and scroll.
#[derive(Default)]
pub struct ScreenUi {
    pub selected: Option<ThreadRef>,
    pub detail: Option<ThreadDetail>,
    pub detail_loading: bool,
    pub detail_error: Option<String>,
    pub list_scroll: f32,
    pub detail_scroll: f32,
    pub reply: String,
    pub reply_cursor: usize,
    pub reply_focused: bool,
    pub sending: bool,
    pub send_error: Option<String>,
    pub hit: Vec<HitRegion>,
    pub list_rows: Vec<ListRow>,
    pub auth_url: Option<String>,
    /// Scroll bounds computed during the last layout, for input clamping.
    pub list_scroll_max: f32,
    pub detail_scroll_max: f32,
}

impl ScreenUi {
    /// Forget the thread pane when the snapshot drops it (e.g. it settled).
    pub fn keep_selection_in(&mut self, threads: &[ThreadSummary]) {
        if let Some(selected) = &self.selected {
            if !threads.iter().any(|t| &t.thread_ref == selected) {
                self.selected = None;
                self.detail = None;
            }
        }
    }

    pub fn cursor_char(&self) -> usize {
        self.reply_cursor.min(self.reply.chars().count())
    }
}

// ---- text layout ----

/// Greedily wrap `text` so no line measures past `width` GUI pixels.
pub fn wrap(text: &str, width: f32, measure: &dyn Fn(&str) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    for raw in text.split('\n') {
        let mut line = String::new();
        for word in raw.split(' ') {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if measure(&candidate) <= width || line.is_empty() {
                line = candidate;
            } else {
                lines.push(std::mem::take(&mut line));
                line = word.to_owned();
            }
        }
        lines.push(line);
    }
    lines
}

/// Shorten `text` to `width`, ending it in "…" when it is cut.
pub fn ellipsize(text: &str, width: f32, measure: &dyn Fn(&str) -> f32) -> String {
    if measure(text) <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    for c in text.chars() {
        if measure(&format!("{out}…{c}")) > width {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

// ---- the HUD ----

/// The rows the corner box shows: one per live thread (attention order,
/// sections dropped — the box is small), capped at `max_rows`, plus a
/// "+N more" line and a status footer.
pub fn hud_rows(snapshot: &Snapshot, config: &Config) -> Vec<(String, u32)> {
    let mut rows = Vec::new();
    match snapshot.connection() {
        Connection::Connected => {
            let shown: Vec<&ThreadSummary> = snapshot
                .threads
                .iter()
                .filter(|t| {
                    config.show_ready_threads || t.attention() != Attention::Ready
                })
                .take(config.hud_max_rows as usize)
                .collect();
            let rest = snapshot
                .threads
                .iter()
                .filter(|t| config.show_ready_threads || t.attention() != Attention::Ready)
                .count()
                .saturating_sub(shown.len());
            for thread in &shown {
                rows.push((
                    format!("● {}", thread.title),
                    attention_color(thread.attention()),
                ));
            }
            if rest > 0 {
                rows.push((format!("+{rest} more"), TEXT_MUTED));
            }
            if shown.is_empty() {
                rows.push(("Nothing running or waiting on you".into(), TEXT_MUTED));
            }
        }
        state => {
            rows.push((connection_label(state).to_owned(), TEXT_MUTED));
        }
    }
    rows
}

pub fn connection_label(state: Connection) -> &'static str {
    match state {
        Connection::SignedOut => "Not connected to T3 Code",
        Connection::SigningIn => "Approve sign-in in the browser…",
        Connection::Connecting => "Connecting…",
        Connection::Connected => "Connected",
        Connection::Unreachable => "Can't reach T3 Code",
        Connection::SignInExpired => "Sign-in expired, sign in again",
    }
}

// ---- the screen's layout (everything in GUI pixels) ----

const MARGIN: f32 = 14.0;
const HEADER: f32 = 22.0;
const LIST_RATIO: f32 = 0.38;
const ROW_H: f32 = 14.0;
const SECTION_H: f32 = 13.0;
const REPLY_H: f32 = 18.0;
const BUTTON_W: f32 = 52.0;
const BUTTON_H: f32 = 14.0;

/// Lay the screen out: draw commands plus the hit regions a click consults.
/// `rect` is the window (`screen.rect`), `mouse` the cursor for hover paint.
pub fn layout_screen(
    ui: &mut ScreenUi,
    snapshot: &Snapshot,
    _config: &Config,
    measure: &dyn Fn(&str) -> f32,
    rect: [f32; 4],
    mouse: [f32; 2],
) -> Vec<DrawCmd> {
    let mut out = Vec::new();
    let mut hit = Vec::new();
    let [rx, ry, rw, rh] = rect;

    // Dim the world under the panel.
    out.push(DrawCmd::Fill { rect, argb: 0x90_00_00_00 });
    let panel = [rx + MARGIN, ry + MARGIN, rw - 2.0 * MARGIN, rh - 2.0 * MARGIN];
    fill(&mut out, panel, PANEL_BG);
    border(&mut out, panel, PANEL_BORDER);

    // Header: title left, connection state right.
    text(&mut out, [panel[0] + 8.0, panel[1] + 7.0], "T3 Code", TEXT_PRIMARY);
    let status = connection_label(snapshot.connection());
    let sw = measure(status);
    text(
        &mut out,
        [panel[0] + panel[2] - sw - 8.0, panel[1] + 7.0],
        status,
        if snapshot.connection() == Connection::Connected { TEXT_MUTED } else { 0xFF_B3_47 },
    );

    let body = [
        panel[0] + 4.0,
        panel[1] + HEADER,
        panel[2] - 8.0,
        panel[3] - HEADER - 4.0,
    ];
    let list_w = (body[2] * LIST_RATIO).floor();
    let list = [body[0], body[1], list_w, body[3]];
    let detail = [body[0] + list_w + 4.0, body[1], body[2] - list_w - 4.0, body[3]];

    let connected = snapshot.connection() == Connection::Connected;
    layout_list(ui, snapshot, connected, measure, list, mouse, &mut out, &mut hit);
    match snapshot.connection() {
        Connection::Connected | Connection::Unreachable => {
            layout_detail(ui, snapshot, measure, detail, mouse, &mut out, &mut hit)
        }
        state => layout_connect(ui, state, measure, detail, mouse, &mut out, &mut hit),
    }

    ui.hit = hit;
    out
}

/// The thread list: section headers then one row a thread, clipped by the
/// pane (scroll in pixels).
fn layout_list(
    ui: &mut ScreenUi,
    snapshot: &Snapshot,
    connected: bool,
    measure: &dyn Fn(&str) -> f32,
    rect: [f32; 4],
    mouse: [f32; 2],
    out: &mut Vec<DrawCmd>,
    hit: &mut Vec<HitRegion>,
) {
    fill(out, rect, 0x40_00_00_00);
    hit.push(HitRegion { rect, action: Action::ScrollList });

    // Sections in urgency order; empty ones are dropped.
    let mut rows: Vec<ListRow> = Vec::new();
    for (attention, title) in [
        (Attention::NeedsInput, "Needs you"),
        (Attention::Running, "Running"),
        (Attention::Ready, "Ready for review"),
        (Attention::Quiet, "Quiet"),
    ] {
        let mut first = true;
        for (index, thread) in snapshot.threads.iter().enumerate() {
            if thread.attention() != attention {
                continue;
            }
            if first {
                rows.push(ListRow::Section(title, attention_color(attention)));
                first = false;
            }
            rows.push(ListRow::Thread(index));
        }
    }

    if rows.is_empty() {
        // Only promise "nothing running" once connected; while signed out the
        // right pane explains the state and a hint here collides with the
        // header's status line.
        if connected {
            text(
                out,
                [rect[0] + 6.0, rect[1] + 6.0],
                "Nothing running or waiting on you",
                TEXT_MUTED,
            );
        }
        ui.list_rows = rows;
        ui.list_scroll_max = 0.0;
        return;
    }

    // Total height for scroll clamping.
    let total: f32 = rows
        .iter()
        .map(|r| match r {
            ListRow::Section(..) => SECTION_H,
            ListRow::Thread(_) => ROW_H,
        })
        .sum();
    ui.list_scroll_max = (total - rect[3]).max(0.0);
    ui.list_scroll = ui.list_scroll.clamp(0.0, ui.list_scroll_max);

    let mut y = rect[1] - ui.list_scroll;
    let bottom = rect[1] + rect[3];
    for (row_index, row) in rows.iter().enumerate() {
        let height = match row {
            ListRow::Section(..) => SECTION_H,
            ListRow::Thread(_) => ROW_H,
        };
        let visible = y + height > rect[1] && y < bottom;
        match row {
            ListRow::Section(title, color) if visible => {
                text(out, [rect[0] + 6.0, y + 3.0], title, *color);
            }
            ListRow::Thread(index) if visible => {
                let thread = &snapshot.threads[*index];
                let selected = ui.selected.as_ref() == Some(&thread.thread_ref);
                let hovered = inside(mouse, [rect[0], y, rect[2], height]);
                if selected {
                    fill(out, [rect[0] + 1.0, y, rect[2] - 2.0, height], ROW_SELECTED);
                } else if hovered {
                    fill(out, [rect[0] + 1.0, y, rect[2] - 2.0, height], ROW_HOVER);
                }
                let title = ellipsize(&thread.title, rect[2] - 14.0, measure);
                text(
                    out,
                    [rect[0] + 6.0, y + 3.0],
                    &format!("● {title}"),
                    attention_color(thread.attention()),
                );
                hit.push(HitRegion {
                    rect: [rect[0], y.max(rect[1]), rect[2], (y + height).min(bottom) - y.max(rect[1])],
                    action: Action::Select(row_index),
                });
            }
            _ => {}
        }
        y += height;
    }
    ui.list_rows = rows;
}

/// The right pane when there is no connection: state, error, sign-in button.
fn layout_connect(
    ui: &mut ScreenUi,
    state: Connection,
    measure: &dyn Fn(&str) -> f32,
    rect: [f32; 4],
    mouse: [f32; 2],
    out: &mut Vec<DrawCmd>,
    hit: &mut Vec<HitRegion>,
) {
    let mut y = rect[1] + 10.0;
    text(out, [rect[0] + 8.0, y], connection_label(state), TEXT_PRIMARY);
    y += LINE_HEIGHT * 2.0;
    match state {
        Connection::SigningIn => {
            for line in wrap(
                "Approve the sign-in in your browser. If it did not open, copy this link:",
                rect[2] - 16.0,
                measure,
            ) {
                text(out, [rect[0] + 8.0, y], &line, TEXT_MUTED);
                y += LINE_HEIGHT;
            }
            if let Some(url) = &ui.auth_url {
                y += 2.0;
                for line in wrap(url, rect[2] - 16.0, measure) {
                    text(out, [rect[0] + 8.0, y], &line, 0x7F_B7_FF);
                    y += LINE_HEIGHT;
                }
            }
            y += 8.0;
            button(out, hit, measure, mouse, [rect[0] + 8.0, y, 90.0, BUTTON_H], "Cancel sign-in", Action::CancelSignIn);
        }
        _ => {
            for line in wrap(
                "Connect the game to your T3 Code to see your threads here.",
                rect[2] - 16.0,
                measure,
            ) {
                text(out, [rect[0] + 8.0, y], &line, TEXT_MUTED);
                y += LINE_HEIGHT;
            }
            y += 8.0;
            button(out, hit, measure, mouse, [rect[0] + 8.0, y, 76.0, BUTTON_H], "Sign in to T3 Code", Action::SignIn);
        }
    }
}

/// The right pane connected: the selected thread or a hint, messages, reply.
fn layout_detail(
    ui: &mut ScreenUi,
    snapshot: &Snapshot,
    measure: &dyn Fn(&str) -> f32,
    rect: [f32; 4],
    mouse: [f32; 2],
    out: &mut Vec<DrawCmd>,
    hit: &mut Vec<HitRegion>,
) {
    // Top-right buttons: refresh, sign out.
    let mut bx = rect[0] + rect[2] - 8.0;
    for (label, action) in [("Sign out", Action::SignOut), ("Refresh", Action::Refresh)] {
        bx -= BUTTON_W + 4.0;
        button(out, hit, measure, mouse, [bx, rect[1] + 2.0, BUTTON_W, BUTTON_H], label, action);
    }
    hit.push(HitRegion { rect, action: Action::ScrollDetail });

    let Some(selected) = ui.selected.clone() else {
        text(out, [rect[0] + 8.0, rect[1] + 10.0], "Pick a thread on the left", TEXT_MUTED);
        return;
    };
    let summary = snapshot
        .threads
        .iter()
        .find(|t| t.thread_ref == selected);

    let mut y = rect[1] + BUTTON_H + 6.0;
    let x = rect[0] + 8.0;
    let width = rect[2] - 16.0;

    let title = summary.map(|t| t.title.as_str()).or(ui.detail.as_ref().map(|d| d.title.as_str()));
    if let Some(title) = title {
        text(out, [x, y], &ellipsize(title, width - 2.0 * (BUTTON_W + 8.0), measure), TEXT_PRIMARY);
        y += LINE_HEIGHT;
    }
    let status = summary
        .map(|t| t.status.clone())
        .or_else(|| ui.detail.as_ref().map(|d| d.status.clone()))
        .unwrap_or_default();
    text(out, [x, y], &status_label(&status), status_color(&status));
    y += LINE_HEIGHT;

    if let Some(detail) = &ui.detail {
        let mut meta = Vec::new();
        if let Some(run) = detail.latest_run {
            if let Some(started) = run.started_at {
                meta.push(format!("running {}", crate::format::since(started)));
            } else if let Some(done) = run.completed_at {
                meta.push(format!("finished {} ago", crate::format::since(done)));
            }
        }
        if !detail.branch.is_empty() {
            meta.push(format!("⌥ {}", detail.branch));
        }
        if !detail.model.is_empty() {
            meta.push(crate::format::model(&detail.model));
        }
        if !detail.runtime_mode.is_empty() {
            meta.push(crate::format::runtime_mode(&detail.runtime_mode));
        }
        if detail.pending_request_count > 0 {
            meta.push(format!("{} waiting on you", detail.pending_request_count));
        }
        if !meta.is_empty() {
            text(out, [x, y], &ellipsize(&meta.join(" · "), width, measure), TEXT_MUTED);
            y += LINE_HEIGHT;
        }
    }
    y += 4.0;

    // Messages fill the space down to the reply box.
    let reply_top = rect[1] + rect[3] - REPLY_H - 4.0;
    let msg_rect = [x, y, width, (reply_top - y - 4.0).max(0.0)];

    let mut lines: Vec<(String, u32)> = Vec::new();
    if ui.detail_loading {
        lines.push(("Loading…".into(), TEXT_MUTED));
    } else if let Some(error) = &ui.detail_error {
        for l in wrap(error, width, measure) {
            lines.push((l, 0xFF_6B_6B));
        }
    } else if let Some(detail) = &ui.detail {
        if detail.pending_request_count > 0 {
            lines.push((
                "Waiting on a question or approval. Answer it in T3 Code.".into(),
                0xFF_B3_47,
            ));
        }
        for message in &detail.messages {
            let (who, color) = match message.role {
                MessageRole::User => ("You", USER_ACCENT),
                MessageRole::Assistant => ("T3", ASSISTANT_ACCENT),
            };
            lines.push((who.to_owned(), color));
            for l in wrap(&message.text, width - 10.0, measure) {
                lines.push((l, TEXT_PRIMARY));
            }
            lines.push((String::new(), TEXT_PRIMARY));
        }
        if detail.messages.is_empty() {
            lines.push(("No messages yet".into(), TEXT_MUTED));
        }
    }

    let total_h = lines.len() as f32 * LINE_HEIGHT;
    ui.detail_scroll_max = (total_h - msg_rect[3]).max(0.0);
    ui.detail_scroll = ui.detail_scroll.clamp(0.0, ui.detail_scroll_max);
    let mut ly = msg_rect[1] - ui.detail_scroll;
    for (line, color) in &lines {
        if ly + LINE_HEIGHT > msg_rect[1] && ly < msg_rect[1] + msg_rect[3] {
            text(out, [msg_rect[0], ly], line, *color);
        }
        ly += LINE_HEIGHT;
    }

    // The reply box: click to focus, type, Enter sends.
    let input = [x, reply_top, width - 54.0, REPLY_H];
    fill(out, input, INPUT_BG);
    border(out, input, if ui.reply_focused { BUTTON_ACCENT } else { PANEL_BORDER });
    hit.push(HitRegion { rect: input, action: Action::FocusReply });
    let hint = if ui.reply.is_empty() && !ui.reply_focused {
        "Reply… (Enter sends, Shift+Enter new line)"
    } else if ui.sending {
        "Sending…"
    } else {
        ""
    };
    if !hint.is_empty() {
        text(out, [input[0] + 4.0, input[1] + 5.0], hint, TEXT_MUTED);
    } else {
        let shown = reply_with_cursor(&ui.reply, ui.cursor_char(), ui.reply_focused);
        text(out, [input[0] + 4.0, input[1] + 5.0], &ellipsize_tail(&shown, input[2] - 8.0, measure), TEXT_PRIMARY);
    }
    if let Some(error) = &ui.send_error {
        text(out, [input[0], input[1] - LINE_HEIGHT], error, 0xFF_6B_6B);
    }
    button(
        out,
        hit,
        measure,
        mouse,
        [x + width - 50.0, reply_top, 50.0, REPLY_H],
        "Send",
        Action::Send,
    );
}

/// `reply` with a blinking block at the char cursor when focused.
fn reply_with_cursor(reply: &str, cursor: usize, focused: bool) -> String {
    let mut chars: Vec<char> = reply.chars().collect();
    let mark = if focused { '▌' } else { ' ' };
    let at = cursor.min(chars.len());
    chars.insert(at, mark);
    chars.into_iter().collect()
}

/// Keep the tail of a long line (the cursor's end) inside the box.
fn ellipsize_tail(text: &str, width: f32, measure: &dyn Fn(&str) -> f32) -> String {
    if measure(text) <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    for c in text.chars().rev() {
        if measure(&format!("…{c}{out}")) > width {
            break;
        }
        out.insert(0, c);
    }
    format!("…{out}")
}

/// A labelled rectangle that reports hover and clicks.
fn button(
    out: &mut Vec<DrawCmd>,
    hit: &mut Vec<HitRegion>,
    measure: &dyn Fn(&str) -> f32,
    mouse: [f32; 2],
    rect: [f32; 4],
    label: &str,
    action: Action,
) {
    let hovered = inside(mouse, rect);
    fill(out, rect, if hovered { 0x90_90_90_90 } else { BUTTON_BG });
    border(out, rect, PANEL_BORDER);
    let w = measure(label);
    text(
        out,
        [rect[0] + (rect[2] - w) / 2.0, rect[1] + (rect[3] - 7.0) / 2.0],
        label,
        TEXT_PRIMARY,
    );
    hit.push(HitRegion { rect, action });
}

fn status_label(status: &str) -> String {
    match status {
        "idle" => "idle",
        "preparing" => "preparing",
        "queued" => "queued",
        "starting" => "starting",
        "running" => "running",
        "waiting" => "needs you",
        "completed" => "done",
        "interrupted" => "interrupted",
        "failed" => "failed",
        "cancelled" => "cancelled",
        "rolled_back" => "rolled back",
        other => other,
    }
    .to_owned()
}

fn inside(at: [f32; 2], rect: [f32; 4]) -> bool {
    at[0] >= rect[0] && at[0] < rect[0] + rect[2] && at[1] >= rect[1] && at[1] < rect[1] + rect[3]
}

fn fill(out: &mut Vec<DrawCmd>, rect: [f32; 4], argb: u32) {
    out.push(DrawCmd::Fill { rect, argb });
}

fn border(out: &mut Vec<DrawCmd>, rect: [f32; 4], argb: u32) {
    let [x, y, w, h] = rect;
    fill(out, [x, y, w, 1.0], argb);
    fill(out, [x, y + h - 1.0, w, 1.0], argb);
    fill(out, [x, y, 1.0, h], argb);
    fill(out, [x + w - 1.0, y, 1.0, h], argb);
}

fn text(out: &mut Vec<DrawCmd>, at: [f32; 2], text: &str, rgb: u32) {
    out.push(DrawCmd::Text {
        at,
        text: text.to_owned(),
        rgb,
        shadow: true,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed(s: &str) -> f32 {
        s.chars().count() as f32 * 6.0
    }

    #[test]
    fn wraps_on_words() {
        let lines = wrap("one two three four", 60.0, &fixed);
        assert_eq!(lines, vec!["one two", "three four"]);
    }

    #[test]
    fn keeps_a_long_word_alone() {
        let lines = wrap("supercalifragilistic", 30.0, &fixed);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn ellipsizes() {
        assert_eq!(ellipsize("short", 60.0, &fixed), "short");
        let cut = ellipsize("a rather long title here", 60.0, &fixed);
        assert!(cut.ends_with('…'));
        assert!(fixed(&cut) <= 66.0);
    }
}
