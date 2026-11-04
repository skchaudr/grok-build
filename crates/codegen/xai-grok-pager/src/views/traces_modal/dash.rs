//! The sessions list drawn the way grok's Dashboard draws agents: a header with counts, a row of views, sections under a
//! rule, and two-line rows (title and age, then where the session ended up) with grok's selection band.
//! Fed by the same rows the ledger loads (local index plus the other machines), so nothing here queries sessions itself.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use ratatui::text::Line;
use ratatui::widgets::Widget as _;

use super::*;
use crate::input::key::key;
use crate::views::agent_status::AgentStatusBar;
use crate::views::shortcuts_bar::{HintItem, ShortcutsBar};

/// Most sessions that get a generated summary: the newest ended roots.
pub(super) const MAX_SUMMARIES: usize = 30;
/// `claude -p` runs at once while summaries are generated.
const SUMMARY_WORKERS: usize = 3;
/// How often active times are recomputed (local index and, over ssh, the other machines).
const ACTIVE_EVERY: Duration = Duration::from_secs(60);
const SUMMARY_PROMPT: &str = "You read the tail of a coding-agent session. Reply with one line of at most 14 words saying where the session \
ended up: what got done, what was left open, or what it was stuck on. Plain words, no preamble, no ids, no quotes.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// Running sessions first, then the rest newest first.
    Live,
    /// By active time, longest first.
    Longest,
    /// Sessions that ended on an error, newest first; subagents included.
    Failed,
    Project,
    Harness,
}

impl View {
    const ALL: [View; 5] = [View::Live, View::Longest, View::Failed, View::Project, View::Harness];

    fn label(self) -> &'static str {
        match self {
            View::Live => "Live and recent",
            View::Longest => "Longest",
            View::Failed => "Failed",
            View::Project => "By project",
            View::Harness => "By harness",
        }
    }

    fn step(self, by: isize) -> View {
        let i = Self::ALL.iter().position(|v| *v == self).unwrap_or(0) as isize;
        let n = Self::ALL.len() as isize;
        Self::ALL[(i + by).rem_euclid(n) as usize]
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(super) struct Summary {
    pub text: String,
    /// The session's event count when summarized; a session that grew since gets a new one.
    pub events: i64,
}

pub(super) struct Dash {
    pub view: View,
    pub compact: bool,
    sel: usize,
    sel_key: Option<String>,
    scroll: usize,
    /// Parents whose subagents are expanded in place (`e`).
    open: HashSet<String>,
    /// Collapsed sections, by label.
    folded: HashSet<String>,
    active: HashMap<String, i64>,
    active_rx: Option<mpsc::Receiver<HashMap<String, i64>>>,
    active_at: Option<Instant>,
    pub summaries: HashMap<String, Summary>,
    summary_rx: mpsc::Receiver<(String, Summary)>,
    summary_tx: mpsc::Sender<(String, Summary)>,
    /// Sessions already queued this run, so a session is asked for once.
    asked: HashSet<String>,
    queue: Arc<Mutex<VecDeque<(String, String, i64)>>>,
    workers: usize,
    /// `GROK_TRACES_NO_SUMMARIES=1` turns generation off (tests, screenshots); cached ones still show.
    generate: bool,
}

fn cache_path() -> PathBuf {
    xai_grok_traces::default_db_path().with_file_name("summaries.json")
}

impl Dash {
    pub fn new() -> Self {
        let (summary_tx, summary_rx) = mpsc::channel();
        let summaries = std::fs::read(cache_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self {
            view: View::Live,
            compact: false,
            sel: 0,
            sel_key: None,
            scroll: 0,
            open: HashSet::new(),
            folded: HashSet::new(),
            active: HashMap::new(),
            active_rx: None,
            active_at: None,
            summaries,
            summary_rx,
            summary_tx,
            asked: HashSet::new(),
            queue: Arc::new(Mutex::new(VecDeque::new())),
            workers: 0,
            generate: std::env::var_os("GROK_TRACES_NO_SUMMARIES").is_none_or(|v| v != "1"),
        }
    }
}

/// One line of the list: a section header or a session.
enum Item<'a> {
    Section { label: String, count: usize },
    Row { row: &'a SessionRow, depth: u16, kids: usize },
}

impl Item<'_> {
    fn key(&self) -> String {
        match self {
            Item::Section { label, .. } => format!("\u{a7}{label}"),
            Item::Row { row, depth, .. } => format!("{depth}:{}", row.id),
        }
    }
}

fn running(r: &SessionRow, kids: &HashMap<String, Vec<SessionRow>>) -> bool {
    r.status == Status::Active || kids.get(&r.id).is_some_and(|k| k.iter().any(|c| running(c, kids)))
}

fn last_at(r: &SessionRow) -> i64 {
    r.last_event_at.or(r.updated_at).unwrap_or(0)
}

/// The folder a session ran in, by name: a worktree counts as its repo.
pub(super) fn project_of(r: &SessionRow) -> String {
    let Some(cwd) = r.cwd.as_deref().filter(|c| !c.is_empty()) else { return "no folder".into() };
    let base = cwd.split("/.claude/worktrees/").next().unwrap_or(cwd);
    let base = base.split("/.worktrees/").next().unwrap_or(base);
    base.trim_end_matches('/').rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("/").to_string()
}

pub(super) fn fmt_active(ms: i64) -> String {
    let m = ms / 60_000;
    match m {
        0 => format!("{}s", (ms / 1000).max(0)),
        1..60 => format!("{m}m"),
        _ => format!("{}h{:02}m", m / 60, m % 60),
    }
}

/// Sections for `view` over the ledger's roots and children.
fn sections<'a>(
    view: View,
    roots: &'a [SessionRow],
    kids: &'a HashMap<String, Vec<SessionRow>>,
    active: &HashMap<String, i64>,
) -> Vec<(String, Vec<&'a SessionRow>)> {
    let mut recent: Vec<&SessionRow> = roots.iter().collect();
    recent.sort_by_key(|r| std::cmp::Reverse(last_at(r)));
    match view {
        View::Live => {
            let (live, rest): (Vec<_>, Vec<_>) = recent.into_iter().partition(|r| running(r, kids));
            [("Live", live), ("Recent", rest)].into_iter().filter(|(_, v)| !v.is_empty()).map(|(l, v)| (l.to_string(), v)).collect()
        }
        View::Longest => {
            let mut rows = recent;
            rows.sort_by_key(|r| std::cmp::Reverse(active.get(&r.id).copied().unwrap_or(0)));
            vec![("Longest active time".into(), rows)]
        }
        View::Failed => {
            let mut rows: Vec<&SessionRow> = roots.iter().chain(kids.values().flatten()).filter(|r| r.failed).collect();
            rows.sort_by_key(|r| std::cmp::Reverse(last_at(r)));
            rows.dedup_by(|a, b| a.id == b.id);
            if rows.is_empty() { Vec::new() } else { vec![("Ended on an error".into(), rows)] }
        }
        View::Project | View::Harness => {
            let mut groups: Vec<(String, Vec<&SessionRow>)> = Vec::new();
            for r in recent {
                let name = if view == View::Project { project_of(r) } else { source_label(&r.source).to_string() };
                match groups.iter_mut().find(|(n, _)| *n == name) {
                    Some((_, v)) => v.push(r),
                    None => groups.push((name, vec![r])),
                }
            }
            groups
        }
    }
}

fn build<'a>(d: &Dash, roots: &'a [SessionRow], kids: &'a HashMap<String, Vec<SessionRow>>) -> Vec<Item<'a>> {
    let mut out = Vec::new();
    for (label, rows) in sections(d.view, roots, kids, &d.active) {
        let folded = d.folded.contains(&label);
        out.push(Item::Section { label, count: rows.len() });
        if folded {
            continue;
        }
        for row in rows {
            push_row(&mut out, d, row, 0, kids);
        }
    }
    out
}

fn push_row<'a>(out: &mut Vec<Item<'a>>, d: &Dash, row: &'a SessionRow, depth: u16, kids: &'a HashMap<String, Vec<SessionRow>>) {
    let children = kids.get(&row.id).map(Vec::as_slice).unwrap_or(&[]);
    out.push(Item::Row { row, depth, kids: children.len().max(row.children as usize) });
    if d.open.contains(&row.id) && depth < 6 {
        for c in children {
            push_row(out, d, c, depth + 1, kids);
        }
    }
}

/// Screen lines an item takes, its trailing spacer included.
fn height(d: &Dash, item: &Item) -> usize {
    match item {
        Item::Section { .. } => if d.compact { 1 } else { 2 },
        Item::Row { depth, .. } if *depth > 0 => 1,
        Item::Row { .. } => if d.compact { 1 } else { 3 },
    }
}

impl TracesModalState {
    pub(super) fn dash_active(&self) -> bool {
        self.dash.is_some() && self.mode == Mode::List && self.typing == Typing::No
    }

    /// Background work for the list: active times, generated summaries. Returns whether to redraw.
    pub(super) fn poll_dash(&mut self) -> bool {
        let Some(mut d) = self.dash.take() else { return false };
        let mut redraw = false;
        if let Some(rx) = &d.active_rx
            && let Ok(map) = rx.try_recv()
        {
            d.active = map;
            d.active_rx = None;
            redraw = true;
        }
        if d.active_rx.is_none() && d.active_at.is_none_or(|t| t.elapsed() >= ACTIVE_EVERY) {
            d.active_at = Some(Instant::now());
            let (tx, rx) = mpsc::channel();
            d.active_rx = Some(rx);
            let path = self.index.as_ref().map(|i| i.db_path().to_path_buf());
            let hosts = self.hosts.clone();
            std::thread::spawn(move || {
                let mut map = path.and_then(|p| TraceIndex::open(&p).ok()).and_then(|i| i.active_ms().ok()).unwrap_or_default();
                for h in hosts {
                    map.extend(remote_active(&h));
                }
                let _ = tx.send(map);
            });
        }
        let mut got = false;
        while let Ok((id, s)) = d.summary_rx.try_recv() {
            d.summaries.insert(id, s);
            got = true;
        }
        if got {
            if let Ok(json) = serde_json::to_vec(&d.summaries) {
                let _ = std::fs::write(cache_path(), json);
            }
            redraw = true;
        }
        if d.generate {
            self.queue_summaries(&mut d);
        }
        self.dash = Some(d);
        redraw
    }

    /// The newest ended roots without a current summary go to the `claude -p` workers, at most [`MAX_SUMMARIES`] per run.
    fn queue_summaries(&self, d: &mut Dash) {
        let mut ended: Vec<&SessionRow> = self.shown_roots.iter().filter(|r| !running(r, &self.shown_children)).collect();
        ended.sort_by_key(|r| std::cmp::Reverse(last_at(r)));
        let want: Vec<&SessionRow> = ended
            .into_iter()
            .take(MAX_SUMMARIES)
            .filter(|r| !d.asked.contains(&r.id) && d.summaries.get(&r.id).is_none_or(|s| s.events != r.event_count))
            .collect();
        if want.is_empty() {
            return;
        }
        let index = self.index.as_ref().map(|i| i.db_path().to_path_buf());
        for r in want {
            d.asked.insert(r.id.clone());
            let input = summary_input(r, index.as_deref());
            d.queue.lock().unwrap().push_back((r.id.clone(), input, r.event_count));
        }
        while d.workers < SUMMARY_WORKERS {
            d.workers += 1;
            let (queue, tx) = (d.queue.clone(), d.summary_tx.clone());
            std::thread::spawn(move || {
                loop {
                    let next = queue.lock().unwrap().pop_front();
                    let Some((id, input, events)) = next else { break };
                    if let Some(text) = summarize(&input) {
                        let _ = tx.send((id, Summary { text, events }));
                    }
                }
            });
        }
    }
}

/// What `claude -p` reads: the session's name, harness, folder and outcome, then its last events from the index.
/// Rows from another machine have only what the list carries.
fn summary_input(r: &SessionRow, index: Option<&std::path::Path>) -> String {
    let mut s = format!(
        "Session: {}\nHarness: {}  Folder: {}  Calls: {}  Errors: {}{}\n",
        clean_name(r),
        source_label(&r.source),
        project_of(r),
        r.tool_count,
        r.error_count,
        if r.failed { "  (ended on an error)" } else { "" }
    );
    let events = index
        .filter(|_| r.host.is_none())
        .and_then(|p| TraceIndex::open(p).ok())
        .and_then(|i| i.list_events(&r.id, &EventFilter { limit: EVENT_PAGE, ..EventFilter::default() }).ok())
        .unwrap_or_default();
    s.push_str("Last events:\n");
    for e in events.iter().rev().take(30).collect::<Vec<_>>().into_iter().rev() {
        let who = e.role.as_deref().unwrap_or(&e.kind);
        let line: String = format!("{who}: {} {}", e.title, e.preview).chars().take(240).collect();
        s.push_str(&format!("- {}\n", one_line(&line)));
    }
    if events.is_empty()
        && let Some(now) = &r.now
    {
        s.push_str(&format!("- last: {now}\n"));
    }
    s
}

/// One `claude -p --model haiku` run with no tools, no saved session and no user settings (so no hooks write receipts).
fn summarize(input: &str) -> Option<String> {
    let mut child = Command::new("claude")
        .args(["-p", "--model", "haiku", "--no-session-persistence", "--tools", "", "--setting-sources", "", "--system-prompt", SUMMARY_PROMPT])
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(input.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(line.chars().take(200).collect())
}

/// Active times on another machine: its index answers [`xai_grok_traces::ACTIVE_SQL`] through `sqlite3` over ssh.
fn remote_active(target: &str) -> HashMap<String, i64> {
    let label = remote::label(target).to_string();
    let child = Command::new("ssh")
        .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=3", target, "sqlite3 -readonly ~/.grok/traces/traces.sqlite"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return HashMap::new() };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(format!("{};\n", xai_grok_traces::ACTIVE_SQL).as_bytes());
    }
    let Ok(out) = child.wait_with_output() else { return HashMap::new() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.rsplit_once('|'))
        .filter_map(|(id, ms)| Some((format!("{label}:{id}"), ms.trim().parse().ok()?)))
        .collect()
}

// ---- keys ----

/// Keys the list owns; `None` hands the key to the ledger's handler (Esc, `R`, `/`).
pub(super) fn key(state: &mut TracesModalState, key: &KeyEvent) -> Option<InputOutcome> {
    let mut d = state.dash.take()?;
    let out = dash_key(state, &mut d, key);
    state.dash = Some(d);
    out
}

fn dash_key(state: &mut TracesModalState, d: &mut Dash, key: &KeyEvent) -> Option<InputOutcome> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let items = build(d, &state.shown_roots, &state.shown_children);
    let len = items.len();
    let sel = d.sel.min(len.saturating_sub(1));
    let moved = |d: &mut Dash, to: usize| {
        d.sel = to.min(len.saturating_sub(1));
        d.sel_key = items.get(d.sel).map(Item::key);
    };
    match key.code {
        KeyCode::Char('g') if ctrl => d.view = d.view.step(1),
        _ if ctrl => return None,
        KeyCode::Char('j') | KeyCode::Down => moved(d, sel + 1),
        KeyCode::Char('k') | KeyCode::Up => moved(d, sel.saturating_sub(1)),
        KeyCode::PageDown => moved(d, sel + 10),
        KeyCode::PageUp => moved(d, sel.saturating_sub(10)),
        KeyCode::Char('g') | KeyCode::Home => moved(d, 0),
        KeyCode::Char('G') | KeyCode::End => moved(d, len),
        KeyCode::Tab => d.view = d.view.step(1),
        KeyCode::BackTab => d.view = d.view.step(-1),
        KeyCode::Char('c') => d.compact = !d.compact,
        KeyCode::Char('e') | KeyCode::Char(' ') | KeyCode::Right | KeyCode::Char('l') | KeyCode::Left | KeyCode::Char('h') => {
            let expand = matches!(key.code, KeyCode::Right | KeyCode::Char('l'));
            let collapse = matches!(key.code, KeyCode::Left | KeyCode::Char('h'));
            match items.get(sel) {
                Some(Item::Section { label, .. }) => {
                    if (d.folded.contains(label) && !collapse) || (!d.folded.contains(label) && !expand) {
                        toggle(&mut d.folded, label);
                    }
                }
                Some(Item::Row { row, kids, depth }) => {
                    let is_open = d.open.contains(&row.id);
                    if *kids > 0 && ((is_open && !expand) || (!is_open && !collapse)) {
                        toggle(&mut d.open, &row.id);
                    } else if collapse && *depth > 0 {
                        // `←` on a subagent goes to its parent and folds it.
                        if let Some(p) = (0..sel).rev().find(|&i| matches!(items.get(i), Some(Item::Row { depth: pd, .. }) if pd < depth)) {
                            if let Some(Item::Row { row: parent, .. }) = items.get(p) {
                                d.open.remove(&parent.id);
                            }
                            moved(d, p);
                        }
                    }
                }
                None => {}
            }
        }
        KeyCode::Enter => match items.get(sel) {
            Some(Item::Section { label, .. }) => toggle(&mut d.folded, label),
            Some(Item::Row { row, .. }) => {
                let row = (*row).clone();
                return Some(open_row(state, row));
            }
            None => {}
        },
        _ => return None,
    }
    Some(InputOutcome::Changed)
}

fn toggle(set: &mut HashSet<String>, k: &str) {
    if !set.remove(k) {
        set.insert(k.to_string());
    }
}

/// Enter: grok's read-only session view for every harness it can read (a running session follows), else the ledger's trajectory.
fn open_row(state: &mut TracesModalState, row: SessionRow) -> InputOutcome {
    let readable = matches!(row.source.as_str(), "claude" | "pi" | "dpi" | "codex" | "grok");
    if readable && (row.host.is_none() || row.source != "dpi") {
        let row = Box::new(row);
        return InputOutcome::Action(if row.source == "grok" { Action::TracesOpenGrok(row) } else { Action::TracesOpenClaude(row) });
    }
    state.trail.clear();
    state.live_return = false;
    state.open_session(row);
    InputOutcome::Changed
}

pub(super) fn wheel(state: &mut TracesModalState, down: bool) -> InputOutcome {
    let Some(mut d) = state.dash.take() else { return InputOutcome::Unchanged };
    let len = build(&d, &state.shown_roots, &state.shown_children).len();
    d.sel = if down { (d.sel + WHEEL as usize).min(len.saturating_sub(1)) } else { d.sel.saturating_sub(WHEEL as usize) };
    d.sel_key = None;
    state.dash = Some(d);
    InputOutcome::Changed
}

// ---- drawing ----

pub(super) fn render(buf: &mut Buffer, area: Rect, state: &mut TracesModalState) {
    let Some(mut d) = state.dash.take() else { return };
    draw(buf, area, state, &mut d);
    state.dash = Some(d);
}

fn draw(buf: &mut Buffer, area: Rect, state: &TracesModalState, d: &mut Dash) {
    let theme = Theme::current();
    buf.set_style(area, Style::default().bg(theme.bg_base));
    if area.height < 6 || area.width < 30 {
        return;
    }
    let now = now_ms();
    let pad = 2u16;
    let inner = Rect { x: area.x + pad, width: area.width.saturating_sub(pad * 2), ..area };
    let items = build(d, &state.shown_roots, &state.shown_children);
    // Selection follows its row across refreshes and view changes; otherwise it stays put.
    if let Some(k) = &d.sel_key
        && let Some(i) = items.iter().position(|it| it.key() == *k)
    {
        d.sel = i;
    }
    d.sel = d.sel.min(items.len().saturating_sub(1));
    d.sel_key = items.get(d.sel).map(Item::key);

    draw_header(buf, Rect { height: 1, ..inner }, state, &theme, now);
    draw_views(buf, Rect { y: area.y + 2, height: 1, ..inner }, d, &theme);

    let list = Rect { y: area.y + 4, height: area.height.saturating_sub(6), ..area };
    let footer = Rect { y: area.y + area.height - 1, height: 1, ..inner };
    draw_footer(buf, footer, state, d, &items);

    if items.is_empty() {
        let msg = if state.scanning { "Loading sessions…" } else if d.view == View::Failed { "No session ended on an error" } else { "No sessions yet" };
        buf.set_string(inner.x, list.y, msg, theme.dim().bg(theme.bg_base));
        return;
    }
    let heights: Vec<usize> = items.iter().map(|it| height(d, it)).collect();
    let top_of = |i: usize| heights[..i].iter().sum::<usize>();
    let (sel_top, sel_h) = (top_of(d.sel), heights[d.sel]);
    let h = list.height as usize;
    if sel_top < d.scroll {
        d.scroll = sel_top;
    } else if sel_top + sel_h > d.scroll + h {
        d.scroll = (sel_top + sel_h).saturating_sub(h);
    }
    // The band's half-cell halo above the selection needs the spacer before it on screen.
    if d.sel > 0 && !d.compact && d.scroll == sel_top && sel_top > 0 {
        d.scroll -= 1;
    }
    let body_w = list.width.saturating_sub(1);
    let mut y = 0usize;
    for (i, it) in items.iter().enumerate() {
        let ih = heights[i];
        if y + ih <= d.scroll {
            y += ih;
            continue;
        }
        if y >= d.scroll + h {
            break;
        }
        let selected = i == d.sel;
        let sy = y as isize - d.scroll as isize;
        let at = |dy: usize| -> Option<u16> {
            let row = sy + dy as isize;
            (row >= 0 && (row as usize) < h).then(|| list.y + row as u16)
        };
        match it {
            Item::Section { label, count } => {
                if let Some(ly) = at(0) {
                    section_header(buf, Rect { x: inner.x, y: ly, width: body_w.saturating_sub(pad), height: 1 }, &theme, label, *count, d.folded.contains(label), selected);
                }
            }
            Item::Row { row, depth, kids } => {
                let lines = if *depth > 0 || d.compact { 1 } else { 2 };
                let bg = if selected { theme.bg_highlight } else { theme.bg_base };
                for dy in 0..lines {
                    if let Some(ly) = at(dy) {
                        buf.set_string(list.x, ly, " ".repeat(body_w as usize), Style::default().bg(bg));
                        if selected {
                            buf.set_string(list.x + 1, ly, crate::glyphs::selection_bar(), Style::default().bg(bg).fg(theme.accent_user).add_modifier(Modifier::BOLD));
                        }
                    }
                }
                if selected && !d.compact && *depth == 0 {
                    // grok's halo: half-cell bands above and below the selected row.
                    let halo = "\u{2580}".repeat(body_w as usize);
                    if let Some(ly) = at(0).and_then(|y| y.checked_sub(1)).filter(|y| *y >= list.y) {
                        buf.set_string(list.x, ly, &halo, Style::default().fg(theme.bg_base).bg(theme.bg_highlight));
                    }
                    if let Some(ly) = at(2) {
                        buf.set_string(list.x, ly, &halo, Style::default().fg(theme.bg_highlight).bg(theme.bg_base));
                    }
                }
                let x = inner.x + 2 * depth;
                let w = (list.x + body_w).saturating_sub(x).saturating_sub(1);
                if let Some(ly) = at(0) {
                    title_line(buf, Rect { x, y: ly, width: w, height: 1 }, &theme, d, row, *kids, *depth, bg, selected, now);
                }
                if lines == 2
                    && let Some(ly) = at(1)
                {
                    let text = second_line(d, row, &state.shown_children);
                    let style = if selected { Style::default().bg(bg).fg(theme.text_secondary) } else { theme.dim().bg(bg) };
                    let tx = x + 2;
                    buf.set_string(tx, ly, truncate_str(&text, w.saturating_sub(2) as usize), style);
                }
            }
        }
        y += ih;
    }
    let total: usize = heights.iter().sum();
    if total > h {
        let thumb = (h * h / total).max(1);
        let pos = (d.scroll * (h - thumb)) / total.saturating_sub(h).max(1);
        for i in 0..thumb {
            buf.set_string(list.x + list.width - 1, list.y + (pos + i) as u16, "\u{2588}", Style::default().fg(theme.selection_border).bg(theme.bg_base));
        }
    }
}

fn draw_header(buf: &mut Buffer, area: Rect, state: &TracesModalState, theme: &Theme, now: i64) {
    let bg = Style::default().bg(theme.bg_base);
    let all = || state.shown_roots.iter();
    let live = all().filter(|r| running(r, &state.shown_children)).count();
    let failed = all().filter(|r| r.failed).count();
    let idle = all().count() - live;
    let frames = crate::glyphs::dot_spinner_frames();
    let spinner = frames.get((now / 120) as usize % frames.len().max(1)).copied().unwrap_or("");
    let mut status = AgentStatusBar::new(theme);
    for (label, glyph, color, count) in [
        ("live", spinner, theme.accent_running, live),
        ("ended", crate::glyphs::diamond_hollow(), theme.gray_dim, idle),
        ("failed", crate::glyphs::diamond_filled(), theme.accent_error, failed),
    ] {
        if count > 0 {
            status.push(label, Line::from(vec![Span::styled(glyph, bg.fg(color)), Span::styled(format!(" {count} {label}"), bg.fg(theme.gray))]));
        }
    }
    let rects = status.render(buf, area);
    let budget = rects.values().map(|r| r.x).min().unwrap_or(area.x + area.width).saturating_sub(area.x + 2) as usize;
    let mut machines = vec![state.local_host.clone()];
    machines.extend(state.hosts.iter().map(|h| remote::label(h).to_string()));
    let machines = machines.join(" + ");
    let mut x = area.x;
    for (text, style) in [("traces ", bg.fg(theme.text_secondary)), (machines.as_str(), theme.dim().bg(theme.bg_base))] {
        let left = budget.saturating_sub((x - area.x) as usize);
        let t = truncate_str(text, left);
        buf.set_string(x, area.y, &t, style);
        x += t.width() as u16;
    }
    if let Some(msg) = state.status.as_deref().or(state.health.as_deref()) {
        let left = budget.saturating_sub((x - area.x) as usize + 3);
        buf.set_string(x, area.y, truncate_str(&format!(" · {msg}"), left), Style::default().bg(theme.bg_base).fg(theme.warning));
    }
}

/// grok's actions row, used for the views: the current one bright, the rest dim, `│` between.
fn draw_views(buf: &mut Buffer, area: Rect, d: &Dash, theme: &Theme) {
    let mut x = area.x;
    let end = area.x + area.width;
    for (i, v) in View::ALL.iter().enumerate() {
        if i > 0 {
            if x + 3 >= end {
                break;
            }
            buf.set_string(x, area.y, " \u{2502} ", Style::default().fg(theme.selection_border).bg(theme.bg_base));
            x += 3;
        }
        let style = if *v == d.view {
            Style::default().fg(theme.accent_user).bg(theme.bg_base).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_secondary).bg(theme.bg_base)
        };
        let t = truncate_str(v.label(), end.saturating_sub(x) as usize);
        buf.set_string(x, area.y, &t, style);
        x += t.width() as u16;
    }
    let density = if d.compact { "Compact" } else { "Comfortable" };
    let w = density.width() as u16;
    if x + w + 2 < end {
        buf.set_string(end - w, area.y, density, theme.dim().bg(theme.bg_base));
    }
}

/// grok's section header: disclosure, bold label, count, then a rule to the edge.
fn section_header(buf: &mut Buffer, rect: Rect, theme: &Theme, label: &str, count: usize, folded: bool, selected: bool) {
    let label_style = if selected { Style::default().fg(theme.accent_user) } else { theme.muted() }.bg(theme.bg_base).add_modifier(Modifier::BOLD);
    let glyph = if folded { crate::glyphs::disclosure_closed() } else { crate::glyphs::disclosure_open() };
    let head = format!("{glyph} {label}");
    let head = truncate_str(&head, rect.width as usize);
    buf.set_string(rect.x, rect.y, &head, label_style);
    let mut x = rect.x + head.width() as u16;
    let count = format!(" {count} ");
    if x + count.width() as u16 >= rect.x + rect.width {
        return;
    }
    buf.set_string(x, rect.y, &count, theme.dim().bg(theme.bg_base));
    x += count.width() as u16;
    let rule = "\u{2500}".repeat((rect.x + rect.width).saturating_sub(x) as usize);
    buf.set_string(x, rect.y, rule, Style::default().fg(theme.selection_border).bg(theme.bg_base));
}

#[allow(clippy::too_many_arguments)]
fn title_line(buf: &mut Buffer, rect: Rect, theme: &Theme, d: &Dash, row: &SessionRow, kids: usize, depth: u16, bg: Color, selected: bool, now: i64) {
    let live = row.status == Status::Active;
    let frames = crate::glyphs::dot_spinner_frames();
    let (icon, color) = if live {
        (frames.get((now / 120) as usize % frames.len().max(1)).copied().unwrap_or("*"), theme.accent_running)
    } else if row.failed {
        (crate::glyphs::diamond_filled(), theme.accent_error)
    } else {
        (crate::glyphs::diamond_hollow(), theme.gray_dim)
    };
    buf.set_string(rect.x, rect.y, icon, Style::default().bg(bg).fg(color));
    let x = rect.x + 2;
    let end = rect.x + rect.width;
    // Right side: `Subagents N · <age>` (Longest also shows active time).
    let age = crate::util::format_time_ago(Duration::from_millis((now - last_at(row)).max(0) as u64));
    let mut meta = age;
    if d.view == View::Longest && depth == 0 {
        meta = format!("{} active · {meta}", fmt_active(d.active.get(&row.id).copied().unwrap_or(0)));
    }
    let chip = (kids > 0 && !d.open.contains(&row.id)).then(|| if rect.width > 60 { format!("Subagents {kids}") } else { format!("Sub {kids}") });
    let meta_w = meta.width() as u16;
    let mut right = end.saturating_sub(meta_w);
    buf.set_string(right, rect.y, &meta, Style::default().bg(bg).fg(theme.gray));
    if let Some(chip) = chip {
        let cw = chip.width() as u16 + 3;
        if right > x + cw + 12 {
            right -= cw;
            buf.set_string(right, rect.y, format!("{chip} · "), Style::default().bg(bg).fg(theme.gray));
        }
    }
    let avail = right.saturating_sub(x).saturating_sub(2) as usize;
    let title = clean_name(row);
    let title_style = if selected { Style::default().fg(theme.text_primary).add_modifier(Modifier::BOLD) } else { Style::default().fg(theme.text_primary) }.bg(bg);
    let t = truncate_str(&title, avail);
    buf.set_string(x, rect.y, &t, title_style);
    let mut cx = x + t.width() as u16;
    // Words, not ids: harness and folder (and the machine, when it is not this one) after the title, dim.
    let mut sub = vec![source_label(&row.source).to_string()];
    if depth == 0 && d.view != View::Project {
        sub.push(project_of(row));
    }
    if let Some(h) = &row.host {
        sub.push(h.clone());
    }
    let sub = format!(" · {}", sub.join(" · "));
    let left = (x as usize + avail).saturating_sub(cx as usize);
    if left > 4 {
        let s = truncate_str(&sub, left);
        buf.set_string(cx, rect.y, &s, theme.dim().bg(bg));
        cx += s.width() as u16;
    }
    if row.failed && (x as usize + avail).saturating_sub(cx as usize) >= 9 {
        buf.set_string(cx, rect.y, " · ", theme.dim().bg(bg));
        buf.set_string(cx + 3, rect.y, "failed", Style::default().bg(bg).fg(theme.accent_error));
    }
}

/// Under the title: what a running session is doing now, else where an ended one got to (generated, else its last event).
fn second_line(d: &Dash, row: &SessionRow, kids: &HashMap<String, Vec<SessionRow>>) -> String {
    if running(row, kids) {
        return row.now.as_deref().map(one_line).unwrap_or_else(|| "Working".into());
    }
    if let Some(s) = d.summaries.get(&row.id) {
        return s.text.clone();
    }
    row.now.as_deref().map(one_line).unwrap_or_default()
}

fn draw_footer(buf: &mut Buffer, area: Rect, state: &TracesModalState, d: &Dash, items: &[Item]) {
    let mut hints = Vec::new();
    match items.get(d.sel) {
        Some(Item::Section { label, .. }) => {
            hints.push(HintItem::new(key!(Enter), if d.folded.contains(label) { "expand" } else { "collapse" }));
        }
        Some(Item::Row { row, kids, .. }) => {
            hints.push(HintItem::new(key!(Enter), if row.status == Status::Active { "follow" } else { "open" }));
            if *kids > 0 {
                hints.push(HintItem::new(key!('e'), if d.open.contains(&row.id) { "hide subagents" } else { "subagents" }));
            }
        }
        None => {}
    }
    hints.push(HintItem::new(key!(Tab), "view"));
    hints.push(HintItem::new(key!('c'), if d.compact { "comfortable" } else { "compact" }));
    let _ = state;
    ShortcutsBar::new(&hints).compact(5, None).render(area, buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, at: i64) -> SessionRow {
        SessionRow { id: id.into(), source: "claude".into(), title: id.into(), last_event_at: Some(at), ..SessionRow::default() }
    }

    #[test]
    fn worktrees_count_as_their_repo() {
        let mut r = row("a", 0);
        r.cwd = Some("/r/grok-build/.claude/worktrees/traces-proto-a".into());
        assert_eq!(project_of(&r), "grok-build");
        r.cwd = None;
        assert_eq!(project_of(&r), "no folder");
    }

    #[test]
    fn live_first_then_newest_and_failed_children_show() {
        let mut live_kid = row("k", 1);
        live_kid.status = Status::Active;
        let mut failed_kid = row("f", 2);
        failed_kid.failed = true;
        let roots = vec![row("old", 5), row("parent", 1), row("new", 9)];
        let kids = HashMap::from([("parent".to_string(), vec![live_kid, failed_kid])]);
        let ids = |v: View| -> Vec<(String, Vec<String>)> {
            sections(v, &roots, &kids, &HashMap::new()).into_iter().map(|(l, rs)| (l, rs.iter().map(|r| r.id.clone()).collect())).collect()
        };
        assert_eq!(ids(View::Live), vec![("Live".into(), vec!["parent".into()]), ("Recent".into(), vec!["new".into(), "old".into()])]);
        assert_eq!(ids(View::Failed), vec![("Ended on an error".into(), vec!["f".into()])]);
        let active = HashMap::from([("old".to_string(), 50), ("new".to_string(), 10)]);
        let longest: Vec<String> = sections(View::Longest, &roots, &kids, &active)[0].1.iter().map(|r| r.id.clone()).collect();
        assert_eq!(longest, vec!["old", "new", "parent"]);
    }
}
