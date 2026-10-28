//! `/traces`: ledger of local agent sessions → trajectory → raw-line inspector.
//!
//! Queries run synchronously against the local index (cheap); discovery and
//! parsing run in a background effect and refresh the ledger when done.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use xai_grok_traces::live::{self, LiveRow, LiveState};
use xai_grok_traces::remote::{self, EventQuery, Fetched, RemoteFilter};
use xai_grok_traces::{EventFilter, EventRow, RECEIPT_WINDOW_MS, SOURCES, SessionFilter, SessionRow, Sort, Status, TraceIndex, source_label};

use crate::app::actions::Action;
use crate::app::app_view::InputOutcome;
use crate::render::line_utils::truncate_str;
use crate::theme::Theme;
use crate::views::modal_window::{self, ModalContentArea, ModalSizing, ModalWindowConfig, ModalWindowState, Shortcut};
use crate::views::traces_fold::{FoldNode, FoldState, FoldTree};
use crate::views::traces_outline::{self, OutlineItem, OutlineKey, PROMPT_LEAD};
use crate::views::traces_payload::{self, Payload, Shape};

pub(super) use super::traces_body::{pretty, INPUT_KEYS, TEXT_KEYS};

pub const TITLE: &str = "Traces";
const PAGE: usize = 500;
/// Rows one scroll-wheel notch moves the selection.
const WHEEL: isize = 3;
const EVENT_PAGE: usize = 5000;
/// Children shown under a parent before `+N more`.
const MAX_KIDS: usize = 6;
/// A tool run with at most this many calls starts expanded. Longer runs stay collapsed.
const RUN_OPEN_MAX_CALLS: usize = 5;
/// Most wrapped lines a message body shows before its `… Enter for inspector` tail.
const BODY_LINES: usize = 200;
/// Columns before a trajectory row's text: `#line` and the role chip, each with a space.
const TEXT_COL: usize = 14;
/// Columns before a turn's header text, and before its continuation rows: the fold mark.
const MARK_W: usize = 2;
/// Shortest prompt header kept beside a turn's counts suffix; less than this and the counts go.
const MIN_HEAD_W: usize = 8;
/// Deepest ledger level: roots are 0, and nothing below depth `MAX_DEPTH` is loaded or shown.
const MAX_DEPTH: usize = 8;
/// Tree-guide levels drawn in the ledger (the node's own level included); deeper ones are cut to `…`.
const GUIDE_LEVELS: usize = 3;
const PEEK_TAIL: usize = 20;
/// Separators between a folded call's body and its result.
pub(super) const RESULT_SEP: &str = "── result ──";
pub(super) const RESULT_ERR_SEP: &str = "── ✗ result ──";
/// Key prefix of a `+N more` node; it cannot collide with a session id.
const MORE_KEY: &str = "\u{1}more:";
/// How often remote hosts are refetched while the modal is open.
const REMOTE_EVERY: Duration = Duration::from_secs(10);
/// A status line stays up this long, then the next poll or key clears it.
const STATUS_TTL: Duration = Duration::from_secs(4);
/// Remote body fetches allowed in flight at once; the rest wait for a later render.
const MAX_BODY_FETCHES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    List,
    Trajectory,
    Inspector,
    /// `grok live` / `L`: the running agents, one row each.
    Live,
}

/// Which line editor has the keyboard, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Typing {
    No,
    Ledger,
    Events,
    Live,
}

/// A parent trajectory parked while its dispatch child is open: the session, the selected row and the outline folds.
struct Trail {
    session: SessionRow,
    sel: Option<OutlineKey>,
    folds: FoldState<OutlineKey>,
    seen: HashSet<OutlineKey>,
}

/// The event filters (search text, errors only, tool); a change re-opens the matching turns and runs.
type FilterSig = (String, bool, Option<usize>);

pub struct TracesModalState {
    pub window: ModalWindowState,
    pub mode: Mode,
    index: Option<TraceIndex>,
    pub scanning: bool,
    pub status: Option<String>,
    /// When `status` was set. Cleared with it; the line expires after [`STATUS_TTL`].
    status_at: Option<Instant>,
    /// The resume command `r` just sent to `TracesCopy`; names the copy in the status line.
    copy_note: Option<String>,
    /// Index health problem (over cap, ...); shown red under the tabs.
    pub health: Option<String>,
    /// Hook receipts arrived within `RECEIPT_WINDOW_MS`.
    pub live: bool,
    /// Highest receipt id seen; a change means the index moved under us.
    watermark: i64,
    typing: Typing,

    pub source_tab: usize,
    pub query: String,
    deep: bool,
    pub errors_only: bool,
    pub active_only: bool,
    pub sort: Sort,
    /// `a`: every session, flat (children and trivial ones included).
    pub show_all: bool,
    /// Root sessions as queried.
    pub sessions: Vec<SessionRow>,
    children: HashMap<String, Vec<SessionRow>>,
    /// Parents whose `+N more` overflow is expanded.
    kids_all: HashSet<String>,
    /// Collapsed ledger nodes, keyed by session id (or `+N more` key).
    folds: FoldState<String>,
    /// Parents that already got their default fold (collapsed below the roots).
    fold_seen: HashSet<String>,
    /// Every ledger node in preorder, folded or not.
    tree: FoldTree<String>,
    /// Parallel to `tree.nodes`.
    nodes: Vec<Node>,
    /// The visible ledger: indices into `nodes`. `selected` indexes this.
    ledger: Vec<usize>,
    pub selected: usize,
    scroll: usize,

    pub session: Option<SessionRow>,
    pub events: Vec<EventRow>,
    event_scroll: usize,
    /// The selected row's key and screen line (from the list top) as last painted.
    anchor: Option<(OutlineKey, usize)>,
    pub event_errors_only: bool,
    pub event_query: String,
    /// `n`: show system/bookkeeping rows too, dimmed.
    pub include_noise: bool,
    /// Per event: the tool result folded into that call row.
    folded: Vec<Option<Fold>>,
    /// Per event: a result shown inside its call row, so it gets no outline row of its own.
    hidden: Vec<bool>,
    /// The trajectory outline over `events`; `oshown` lists the visible rows (indices into `outline`).
    outline: FoldTree<OutlineKey>,
    outline_items: Vec<OutlineItem>,
    /// Collapsed outline rows.
    ofolds: FoldState<OutlineKey>,
    /// Foldable outline rows that already got their default fold.
    oseen: HashSet<OutlineKey>,
    oshown: Vec<usize>,
    /// Screen lines of each `oshown` row from the last trajectory render. `0` draws nothing and
    /// cannot be selected. Cleared when the outline changes, until the next render measures it.
    row_h: Vec<usize>,
    /// List or trajectory viewport from the last render, in screen cells. `None` until then.
    hit_list: Option<HitList>,
    /// The selected row, by key: kept across folds, filters and live growth.
    sel: Option<OutlineKey>,
    /// `f`: after a poll appends events, select the last visible row.
    follow: bool,
    /// Text width the outline was last built for; 0 until the first render, so no prompt rows yet.
    outline_w: usize,
    /// An opener's body arrived, so its prompt may have changed: rebuild before measuring.
    dirty_outline: bool,
    /// Event line of every opener turn. A stored `Ev` key for one of these means its turn row.
    opener_lines: HashSet<i64>,
    /// Full text per event line (pretty JSON of its raw line), loaded when its row shows.
    bodies: HashMap<i64, String>,
    body_pending: HashSet<i64>,
    /// Lines whose last read failed; not cached, so `l` retries them.
    body_failed: HashSet<i64>,
    body_tx: mpsc::Sender<BodyMsg>,
    body_rx: mpsc::Receiver<BodyMsg>,
    /// Children of the open session (dispatch targets), oldest first.
    session_children: Vec<SessionRow>,
    /// Per event: which of `session_children` its dispatch started.
    dispatch_child: Vec<Option<usize>>,
    /// Bumped by every synchronous query; a worker snapshot from an older one is dropped.
    generation: u64,
    inflight: Option<mpsc::Receiver<Snapshot>>,

    /// ssh targets from `~/.grok/traces/hosts`.
    hosts: Vec<String>,
    /// 0 = all, 1 = this machine, 2.. = `hosts[n - 2]`.
    pub host_filter: usize,
    /// Last rows per host (tagged: `host` set, `host:`-prefixed ids).
    remote_rows: HashMap<String, Vec<SessionRow>>,
    /// Per host: why the last fetch failed.
    remote_err: HashMap<String, String>,
    remote_rx: Option<mpsc::Receiver<HostResult>>,
    remote_pending: usize,
    remote_at: Option<Instant>,
    /// Events of the open remote session, unfiltered, as fetched with `remote_noise`.
    remote_all: Vec<EventRow>,
    remote_noise: bool,
    remote_loading: bool,
    remote_ev: Option<mpsc::Receiver<RemoteEvents>>,
    raw_rx: Option<mpsc::Receiver<(String, Result<String, String>)>>,
    /// Raw line of the inspected remote event, for `y`.
    raw_source: Option<String>,
    /// Synthetic raw lines (event line → source) when the index has none.
    raw_lines: HashMap<i64, String>,
    /// Tool-call line whose peek overlay is open.
    peek: Option<i64>,
    /// What the ledger shows: roots after filters and merge, with their children.
    shown_roots: Vec<SessionRow>,
    shown_children: HashMap<String, Vec<SessionRow>>,
    parent_name: Option<String>,
    /// Sessions we descended from with Enter on a dispatch row.
    trail: Vec<Trail>,
    /// Outline row to reveal once the restored trajectory has its events (remote loads are async).
    reveal: Option<OutlineKey>,
    /// Filters as the outline last saw them.
    filter_seen: FilterSig,
    tools: Vec<String>,
    pub tool_filter: Option<usize>,

    pub raw: Option<String>,
    raw_scroll: usize,
    /// What Enter shows: the payload (command and result, or text); `m` flips to `raw`.
    payload: Payload,

    /// Live screen: local rows as last folded, and the titles of parents the fold did not return.
    live_rows: Vec<LiveRow>,
    live_titles: HashMap<String, String>,
    remote_live: HashMap<String, RemoteLive>,
    live_collapsed: HashSet<String>,
    live_sel: Option<String>,
    live_sel_idx: usize,
    live_scroll: usize,
    live_query: String,
    standalone: bool,
    live_return: bool,
    live_exit: bool,
    live_gen: u64,
    live_worker: Option<FoldWorker>,
    live_inflight: bool,
    /// When the local rows were folded: their ages advance by the time since.
    live_fold_at: Instant,
    /// Empty-state receipt counts, from the fold worker; the total is asked for only until nonzero.
    live_folded: bool,
    live_receipts_hour: i64,
    live_receipts_any: bool,
    live_rx: Option<mpsc::Receiver<LiveHostResult>>,
    live_pending: usize,
    live_at: Option<Instant>,
    local_host: String,
}

#[derive(Clone, Debug)]
enum Item {
    Row { row: SessionRow },
    More { parent: String, hidden: usize },
}

/// One ledger node, parallel to `FoldTree::nodes`.
#[derive(Clone, Debug)]
struct Node {
    item: Item,
    /// Per level 1..=depth: whether that level's node is the last of its siblings.
    guides: Vec<bool>,
    /// Rows under this node, `+N more` overflow included.
    hidden: usize,
}

/// Preorder flattening of the ledger: a session, then its shown children, then its `+N more`.
struct Flatten<'a> {
    kids: &'a HashMap<String, Vec<SessionRow>>,
    all_kids: &'a HashSet<String>,
    tree: Vec<FoldNode<String>>,
    nodes: Vec<Node>,
    /// Cycle and repeat guard.
    seen: HashSet<String>,
}

impl Flatten<'_> {
    fn session(&mut self, row: &SessionRow, parent: Option<usize>, guides: Vec<bool>) {
        if !self.seen.insert(row.id.clone()) {
            return;
        }
        let me = self.push(Item::Row { row: row.clone() }, row.id.clone(), parent, &guides);
        if guides.len() >= MAX_DEPTH {
            return;
        }
        let kids_map: &HashMap<String, Vec<SessionRow>> = self.kids;
        let Some(kids) = kids_map.get(&row.id) else { return };
        let shown = if kids.len() > MAX_KIDS && !self.all_kids.contains(&row.id) { MAX_KIDS } else { kids.len() };
        let more = shown < kids.len();
        // Children that `session` will skip (already seen, or repeated in this list) get no guide, so the last one drawn is `└─`.
        let mut ids = HashSet::new();
        let live: Vec<&SessionRow> =
            kids.iter().take(shown).filter(|c| !self.seen.contains(&c.id) && ids.insert(c.id.as_str())).collect();
        for (i, c) in live.iter().enumerate() {
            let mut g = guides.clone();
            g.push(!more && i + 1 == live.len());
            self.session(c, Some(me), g);
        }
        if more {
            let mut g = guides;
            g.push(true);
            let item = Item::More { parent: row.id.clone(), hidden: kids.len() - shown };
            self.push(item, format!("{MORE_KEY}{}", row.id), Some(me), &g);
        }
    }

    fn push(&mut self, item: Item, key: String, parent: Option<usize>, guides: &[bool]) -> usize {
        self.tree.push(FoldNode { key, parent, depth: guides.len() });
        self.nodes.push(Node { item, guides: guides.to_vec(), hidden: 0 });
        self.tree.len() - 1
    }

    /// Fills `Node::hidden` bottom-up: each node counts its descendants.
    fn finish(mut self) -> (FoldTree<String>, Vec<Node>) {
        for j in (0..self.tree.len()).rev() {
            let Some(p) = self.tree.get(j).and_then(|n| n.parent) else { continue };
            let weight = match self.nodes.get(j).map(|n| &n.item) {
                Some(Item::More { hidden, .. }) => *hidden,
                _ => 1,
            };
            let add = self.nodes.get(j).map_or(0, |n| n.hidden) + weight;
            if let Some(parent) = self.nodes.get_mut(p) {
                parent.hidden += add;
            }
        }
        (FoldTree { nodes: self.tree }, self.nodes)
    }
}

/// Tree guides for a node: `│` past ancestors that have later siblings, `├─`/`└─` for its own level.
/// Only the last `GUIDE_LEVELS` levels are drawn; anything deeper is cut to a leading `…`.
fn guide_prefix(guides: &[bool]) -> String {
    let Some((&last, ancestors)) = guides.split_last() else { return String::new() };
    let keep = ancestors.len().saturating_sub(GUIDE_LEVELS - 1);
    let (mut s, ancestors) = match ancestors.get(keep..) {
        Some(tail) if keep > 0 => ("… ".to_string(), tail),
        _ => (String::new(), ancestors),
    };
    for &a in ancestors {
        s.push_str(if a { "   " } else { "│  " });
    }
    s.push_str(if last { "└─ " } else { "├─ " });
    s
}

/// A tool result folded into the call row before it.
#[derive(Clone, Debug, PartialEq)]
struct Fold {
    error: bool,
    detail: Option<String>,
}

/// An event row's text parts: a call's tool title (with its two spaces), the header text, and the suffix
/// (child session, folded result). `body_w` is the text cell, `suffix_w` the suffix cell (0 when none).
#[derive(Default)]
struct EventHead {
    title: String,
    head: String,
    suffix: String,
    suffix_w: usize,
    body_w: usize,
    failed: bool,
    chip: bool,
    age: bool,
    tokens: bool,
}

/// What `r` resumes: the harness and its own session id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResumeTarget {
    pub source: String,
    pub native_id: String,
}

pub fn resume_target(row: &SessionRow) -> Option<ResumeTarget> {
    let stem = row.file_path.file_stem()?.to_str()?;
    let native_id = match row.source.as_str() {
        "claude" => stem.to_string(),
        // rollout-<timestamp>-<uuid>.jsonl
        "codex" => stem.get(stem.len().checked_sub(36)?..)?.to_string(),
        "grok" => row.file_path.parent()?.file_name()?.to_str()?.to_string(),
        _ => return None,
    };
    Some(ResumeTarget { source: row.source.clone(), native_id })
}

/// What `r` does for a row: resume in this pager, or copy the harness's own resume command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResumeAction {
    InPager(ResumeTarget),
    CopyCommand(String),
}

pub fn resume_action(row: &SessionRow) -> Option<ResumeAction> {
    if let Some(t) = resume_target(row) {
        return Some(ResumeAction::InPager(t));
    }
    let q = |s: &str| shlex::try_quote(s).map(|c| c.into_owned()).ok();
    let path = q(row.file_path.to_str()?)?;
    let cd = match &row.cwd {
        Some(cwd) => format!("cd {} && ", q(cwd)?),
        None => String::new(),
    };
    let cmd = match row.source.as_str() {
        "pi" => format!("{cd}pi --session {path}"),
        "omp" => format!("{cd}omp --resume {path}"),
        // chat id = transcript stem
        "cursor" => format!("cursor-agent --resume {}", q(row.file_path.file_stem()?.to_str()?)?),
        _ => return None,
    };
    Some(ResumeAction::CopyCommand(cmd))
}

fn tab_source(tab: usize) -> Option<&'static str> {
    tab.checked_sub(1).and_then(|i| SOURCES.get(i).copied())
}

impl TracesModalState {
    pub fn open(index: Option<TraceIndex>) -> Self {
        Self::open_with_hosts(index, remote::hosts())
    }

    pub fn open_with_hosts(index: Option<TraceIndex>, hosts: Vec<String>) -> Self {
        let (body_tx, body_rx) = mpsc::channel();
        let mut s = Self {
            window: ModalWindowState::new(),
            mode: Mode::List,
            status: index.is_none().then(|| "trace index unavailable".to_string()),
            status_at: index.is_none().then(Instant::now),
            copy_note: None,
            index,
            health: None,
            live: false,
            watermark: 0,
            scanning: true,
            typing: Typing::No,
            source_tab: 0,
            query: String::new(),
            deep: false,
            errors_only: false,
            active_only: false,
            sort: Sort::Updated,
            show_all: false,
            sessions: Vec::new(),
            children: HashMap::new(),
            kids_all: HashSet::new(),
            folds: FoldState::new(),
            fold_seen: HashSet::new(),
            tree: FoldTree { nodes: Vec::new() },
            nodes: Vec::new(),
            ledger: Vec::new(),
            selected: 0,
            scroll: 0,
            session: None,
            events: Vec::new(),
            event_scroll: 0,
            anchor: None,
            event_errors_only: false,
            event_query: String::new(),
            include_noise: false,
            folded: Vec::new(),
            hidden: Vec::new(),
            outline: FoldTree { nodes: Vec::new() },
            outline_items: Vec::new(),
            ofolds: FoldState::new(),
            oseen: HashSet::new(),
            oshown: Vec::new(),
            row_h: Vec::new(),
            hit_list: None,
            sel: None,
            follow: false,
            outline_w: 0,
            dirty_outline: false,
            opener_lines: HashSet::new(),
            bodies: HashMap::new(),
            body_pending: HashSet::new(),
            body_failed: HashSet::new(),
            body_tx,
            body_rx,
            session_children: Vec::new(),
            dispatch_child: Vec::new(),
            generation: 0,
            inflight: None,
            hosts,
            host_filter: 0,
            remote_rows: HashMap::new(),
            remote_err: HashMap::new(),
            remote_rx: None,
            remote_pending: 0,
            remote_at: None,
            remote_all: Vec::new(),
            remote_noise: false,
            remote_loading: false,
            remote_ev: None,
            raw_rx: None,
            raw_source: None,
            raw_lines: HashMap::new(),
            peek: None,
            shown_roots: Vec::new(),
            shown_children: HashMap::new(),
            parent_name: None,
            trail: Vec::new(),
            reveal: None,
            filter_seen: FilterSig::default(),
            tools: Vec::new(),
            tool_filter: None,
            raw: None,
            raw_scroll: 0,
            payload: Payload::default(),
            live_rows: Vec::new(),
            live_titles: HashMap::new(),
            remote_live: HashMap::new(),
            live_collapsed: HashSet::new(),
            live_sel: None,
            live_sel_idx: 0,
            live_scroll: 0,
            live_query: String::new(),
            standalone: false,
            live_return: false,
            live_exit: false,
            live_gen: 0,
            live_worker: None,
            live_inflight: false,
            live_fold_at: Instant::now(),
            live_folded: false,
            live_receipts_hour: 0,
            live_receipts_any: false,
            live_rx: None,
            live_pending: 0,
            live_at: None,
            local_host: short_hostname(),
        };
        s.watermark = s.index.as_ref().and_then(|i| i.receipts_watermark().ok()).unwrap_or(0);
        s.refresh();
        s.poll_live();
        s
    }

    /// Live mode, driven by the app's 1s poll while the modal is open. Returns whether to redraw.
    pub fn poll_live(&mut self) -> bool {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
        self.poll_live_at(now)
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some(msg.into());
        self.status_at = Some(Instant::now());
    }

    fn clear_status(&mut self) {
        self.status = None;
        self.status_at = None;
    }

    /// Clear a status line that has been up for [`STATUS_TTL`]. True when it cleared.
    fn expire_status(&mut self) -> bool {
        if self.status_at.is_some_and(|at| at.elapsed() >= STATUS_TTL) {
            self.clear_status();
            true
        } else {
            false
        }
    }

    /// Cheap checks on the UI thread (three tiny queries); anything heavier runs on a worker
    /// and is applied by a later poll.
    fn poll_live_at(&mut self, now_ms: i64) -> bool {
        let mut redraw = self.expire_status();
        if let Some(rx) = &self.inflight {
            match rx.try_recv() {
                Ok(snap) => {
                    self.inflight = None;
                    redraw |= self.apply(snap);
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    // The worker failed before answering (index open); retry on the next poll.
                    self.inflight = None;
                    self.watermark = -1;
                }
            }
        }
        redraw |= self.poll_remote();
        if self.peek.is_some() {
            redraw |= self.refresh_peek();
        }
        if self.mode == Mode::Live {
            redraw |= self.poll_live_screen(now_ms);
        }
        let Some(index) = &self.index else { return redraw };
        let mark = index.receipts_watermark().unwrap_or(self.watermark);
        let live = index.last_receipt_at().ok().flatten().is_some_and(|at| now_ms - at < RECEIPT_WINDOW_MS);
        let health = index.health();
        redraw |= live != self.live || health != self.health;
        (self.live, self.health) = (live, health);
        // `grok live` never shows the ledger: its refresh runs only for an open trajectory.
        let ledger_idle = self.standalone && self.mode == Mode::Live;
        if mark != self.watermark && self.inflight.is_none() && !ledger_idle {
            self.watermark = mark;
            self.spawn_refresh();
        }
        redraw
    }

    /// Requery the ledger (and the open trajectory if it may have grown) off the UI thread.
    fn spawn_refresh(&mut self) {
        let Some(index) = &self.index else { return };
        let path = index.db_path().to_path_buf();
        let req = Request {
            generation: self.generation,
            sessions: self.session_filter(),
            browse: self.roots_only(),
            open: (self.mode != Mode::List).then(|| self.session.as_ref().map(|s| (s.id.clone(), s.event_count))).flatten(),
            events: self.event_filter(),
        };
        let (tx, rx) = mpsc::channel();
        self.inflight = Some(rx);
        std::thread::spawn(move || {
            if let Ok(index) = TraceIndex::open(&path) {
                let _ = tx.send(run_request(&index, req));
            }
        });
    }

    /// Install a worker snapshot unless the user changed filters since it was requested.
    fn apply(&mut self, snap: Snapshot) -> bool {
        if snap.generation != self.generation {
            // Filters moved on while it ran; make the next poll request again.
            self.watermark = -1;
            return false;
        }
        if let Ok((sessions, children)) = snap.ledger {
            // Follow the selected row wherever it moved.
            let keep = self.selected_key();
            (self.sessions, self.children) = (sessions, children);
            self.rebuild_ledger();
            self.keep_selection(keep);
        }
        if let Some(t) = snap.trajectory
            && self.mode != Mode::List
            && self.session.as_ref().is_some_and(|s| s.id == t.session_id)
        {
            // Follow the bottom if the user was there. Follow mode pins there whenever events grew.
            let at_bottom = self.sel_pos() + 1 >= self.oshown.len();
            let had = self.events.len();
            self.install_trajectory(t.rows, t.session, t.children);
            if at_bottom || (self.follow && self.events.len() > had) {
                self.goto(self.oshown.len().saturating_sub(1));
            }
        }
        true
    }

    /// Re-query the ledger (and the open trajectory) from the index.
    pub fn refresh(&mut self) {
        self.reload_sessions();
        if self.mode != Mode::List {
            self.load_events();
        }
    }

    /// Roots-only, trivial-hidden tree view; searching or `a` flattens to every session.
    fn browse(&self) -> bool {
        !self.show_all && self.query.is_empty()
    }

    /// The tree view nests children; the active and error filters list matches flat, so a
    /// child that matches is not lost behind a parent that does not.
    fn roots_only(&self) -> bool {
        self.browse() && !self.active_only && !self.errors_only
    }

    fn session_filter(&self) -> SessionFilter {
        let browse = self.browse();
        SessionFilter {
            source: tab_source(self.source_tab).map(str::to_string),
            text: (!self.query.is_empty()).then(|| self.query.clone()),
            deep: self.deep,
            errors_only: self.errors_only,
            active_only: self.active_only,
            sort: self.sort,
            limit: PAGE,
            roots_only: self.roots_only(),
            hide_trivial: browse,
            ..Default::default()
        }
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter {
            text: (!self.event_query.is_empty()).then(|| self.event_query.clone()),
            errors_only: self.event_errors_only,
            tool: self.tool_filter.and_then(|i| self.tools.get(i).cloned()),
            include_noise: self.include_noise,
            since_line: None,
            limit: EVENT_PAGE,
            offset: 0,
        }
    }

    fn reload_sessions(&mut self) {
        let Some(index) = &self.index else { return };
        self.generation += 1;
        self.health = index.health();
        match query_ledger(index, &self.session_filter(), self.roots_only()) {
            Ok((sessions, children)) => (self.sessions, self.children) = (sessions, children),
            Err(e) => self.set_status(format!("query failed: {e}")),
        }
        self.rebuild_ledger();
    }

    fn rebuild_ledger(&mut self) {
        let browse = self.roots_only();
        let local_ok = self.host_filter <= 1;
        let (remote_roots, remote_kids) = if self.host_filter == 1 { (Vec::new(), HashMap::new()) } else { self.remote_view(browse) };
        let mut roots = if local_ok { self.sessions.clone() } else { Vec::new() };
        let mut kids = if local_ok { self.children.clone() } else { HashMap::new() };
        if !remote_roots.is_empty() {
            roots.extend(remote_roots);
            sort_rows(&mut roots, self.sort);
            if browse && self.sort == Sort::Updated {
                let running = |r: &SessionRow, k: &HashMap<String, Vec<SessionRow>>| {
                    r.status == Status::Active || k.get(&r.id).is_some_and(|c| c.iter().any(|c| c.status == Status::Active))
                };
                let all: HashMap<String, Vec<SessionRow>> = kids.iter().chain(remote_kids.iter()).map(|(k, v)| (k.clone(), v.clone())).collect();
                roots.sort_by_key(|r| !running(r, &all));
            }
        }
        kids.extend(remote_kids);
        let mut flat = Flatten { kids: &kids, all_kids: &self.kids_all, tree: Vec::new(), nodes: Vec::new(), seen: HashSet::new() };
        for r in &roots {
            flat.session(r, None, Vec::new());
        }
        (self.tree, self.nodes) = flat.finish();
        (self.shown_roots, self.shown_children) = (roots, kids);
        self.fold_defaults();
        self.relist(None);
    }

    /// Drops fold state for nodes that left the tree. A parent below the roots starts collapsed
    /// the first time it appears; after that its fold is whatever the user left it.
    fn fold_defaults(&mut self) {
        self.folds.retain(&self.tree);
        {
            let live: HashSet<&str> = self.tree.nodes.iter().map(|n| n.key.as_str()).collect();
            self.fold_seen.retain(|k| live.contains(k.as_str()));
        }
        for i in 0..self.tree.nodes.len() {
            let Some(node) = self.tree.nodes.get(i) else { continue };
            if node.depth == 0 || !self.tree.has_children(i) || self.fold_seen.contains(node.key.as_str()) {
                continue;
            }
            self.folds.collapse(&self.tree, i);
            self.fold_seen.insert(node.key.clone());
        }
    }

    /// Recomputes the visible ledger from the fold state. `keep` is the key to stay on, if still shown;
    /// otherwise the index is clamped.
    fn relist(&mut self, keep: Option<String>) {
        self.ledger = self.tree.visible(&self.folds);
        self.selected = keep.and_then(|k| self.position_of(&k)).unwrap_or(self.selected.min(self.ledger.len().saturating_sub(1)));
    }

    /// Re-lists after a fold change on `node`, keeping the selection on it or on its nearest ancestor still shown.
    fn refold(&mut self, node: usize) {
        let shown = self.tree.visible(&self.folds);
        let mut at = Some(node);
        while let Some(i) = at {
            if shown.contains(&i) {
                break;
            }
            at = self.tree.nodes.get(i).and_then(|n| n.parent);
        }
        let keep = at.and_then(|i| self.tree.nodes.get(i)).map(|n| n.key.clone());
        self.relist(keep);
    }

    /// Selects the line for `key`, or the nearest ancestor of it that is still shown (a `+N more` row, or a
    /// session that left the filter). Leaves the selection alone when the key is unknown.
    fn keep_selection(&mut self, key: Option<String>) {
        let mut at = key.and_then(|k| self.tree.nodes.iter().position(|n| n.key == k));
        while let Some(i) = at {
            if let Some(pos) = self.ledger.iter().position(|&n| n == i) {
                self.selected = pos;
                return;
            }
            at = self.tree.nodes.get(i).and_then(|n| n.parent);
        }
    }

    fn selected_node(&self) -> Option<usize> {
        self.ledger.get(self.selected).copied()
    }

    /// Key of the selected line: a session id, or the `+N more` key.
    fn selected_key(&self) -> Option<String> {
        self.selected_node().and_then(|n| self.tree.nodes.get(n)).map(|n| n.key.clone())
    }

    fn position_of(&self, key: &str) -> Option<usize> {
        self.ledger.iter().position(|&n| self.tree.nodes.get(n).is_some_and(|x| x.key == key))
    }

    /// Applies a fold op to the selected node. False when it changed nothing.
    fn fold_with(&mut self, op: impl FnOnce(&mut FoldState<String>, &FoldTree<String>, usize)) -> bool {
        let Some(node) = self.selected_node() else { return false };
        let before = self.folds.clone();
        op(&mut self.folds, &self.tree, node);
        if self.folds == before {
            return false;
        }
        self.refold(node);
        true
    }

    /// `h` / `←`: collapse the selected node; on a leaf or an already collapsed node, select its parent.
    fn collapse_selected(&mut self) -> bool {
        let Some(node) = self.selected_node() else { return false };
        let before = self.folds.clone();
        match self.folds.collapse(&self.tree, node) {
            Some(parent) => {
                self.refold(parent);
                true
            }
            None if self.folds != before => {
                self.refold(node);
                true
            }
            None => false,
        }
    }

    /// `e` / Space.
    fn toggle_selected(&mut self) -> bool {
        self.fold_with(|f, t, n| f.toggle(t, n))
    }

    /// `→` / `l`: expand the selected node. On a node that is already open, or on `+N more`, show the overflow.
    fn expand_selected(&mut self) -> bool {
        if let Some(parent) = self.selected_more() {
            return self.show_more(parent);
        }
        let Some(node) = self.selected_node() else { return false };
        if self.fold_with(|f, t, n| f.expand(t, n)) {
            return true;
        }
        let id = match self.nodes.get(node).map(|n| &n.item) {
            Some(Item::Row { row }) => row.id.clone(),
            _ => return false,
        };
        let overflow = self.shown_children.get(&id).is_some_and(|k| k.len() > MAX_KIDS) && !self.kids_all.contains(&id);
        overflow && self.show_more(id)
    }

    /// The parent of the selected `+N more` row.
    fn selected_more(&self) -> Option<String> {
        match &self.nodes.get(self.selected_node()?)?.item {
            Item::More { parent, .. } => Some(parent.clone()),
            Item::Row { .. } => None,
        }
    }

    /// Shows every child of `parent` past the `MAX_KIDS` cut. Index clamping keeps the cursor on the first revealed child.
    fn show_more(&mut self, parent: String) -> bool {
        if !self.kids_all.insert(parent) {
            return false;
        }
        self.rebuild_ledger();
        true
    }

    /// Remote rows that pass the ledger filters; when browsing, grouped into roots and children.
    fn remote_view(&self, browse: bool) -> (Vec<SessionRow>, HashMap<String, Vec<SessionRow>>) {
        let tab = tab_source(self.source_tab);
        let q = self.query.to_lowercase();
        let host = self.host_filter.checked_sub(2).and_then(|i| self.hosts.get(i)).map(|t| remote::label(t).to_string());
        let rows: Vec<&SessionRow> = self
            .remote_rows
            .values()
            .flatten()
            .filter(|r| tab.is_none_or(|t| r.source == t))
            .filter(|r| host.as_deref().is_none_or(|h| r.host.as_deref() == Some(h)))
            .filter(|r| !self.errors_only || r.error_count > 0)
            .filter(|r| !self.active_only || r.status == Status::Active)
            .filter(|r| {
                q.is_empty()
                    || [Some(&r.title), r.label.as_ref(), r.cwd.as_ref(), r.model.as_ref()].into_iter().flatten().any(|f| f.to_lowercase().contains(&q))
            })
            .collect();
        if !browse {
            return (rows.into_iter().cloned().collect(), HashMap::new());
        }
        let ids: HashSet<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        let (mut roots, mut kids) = (Vec::new(), HashMap::<String, Vec<SessionRow>>::new());
        for r in rows {
            match r.parent_id.as_deref().filter(|p| ids.contains(p)) {
                Some(p) => kids.entry(p.to_string()).or_default().push(r.clone()),
                None => roots.push(r.clone()),
            }
        }
        for v in kids.values_mut() {
            v.sort_by_key(|c| c.started_at);
        }
        (roots, kids)
    }

    pub fn selected_session(&self) -> Option<&SessionRow> {
        match &self.nodes.get(self.selected_node()?)?.item {
            Item::Row { row } => Some(row),
            Item::More { .. } => None,
        }
    }

    /// Enter on `+N more` expands it; on anything else it does nothing here.
    fn expand_selected_more(&mut self) -> bool {
        match self.selected_more() {
            Some(parent) => self.show_more(parent),
            None => false,
        }
    }

    /// (active, failed, total) over everything loaded, children included.
    fn counts(&self) -> (usize, usize, usize) {
        let all = || self.shown_roots.iter().chain(self.shown_children.values().flatten());
        (all().filter(|s| s.status == Status::Active).count(), all().filter(|s| s.failed).count(), all().count())
    }

    pub fn finish_scan(&mut self, summary: String) {
        self.scanning = false;
        if !summary.is_empty() {
            self.set_status(summary);
        }
        self.refresh();
    }

    pub fn report_copy(&mut self, delivery: &crate::clipboard::CopyDelivery) {
        match (self.copy_note.take(), delivery) {
            (Some(cmd), crate::clipboard::CopyDelivery::Clipboard { .. }) => self.set_status(format!("copied: {cmd}")),
            _ => self.set_status(delivery.toast_message().into_owned()),
        }
    }

    fn open_selected(&mut self) {
        let Some(row) = self.selected_session().cloned() else { return };
        self.trail.clear();
        self.live_return = false;
        self.open_session(row);
    }

    fn open_session(&mut self, row: SessionRow) {
        self.close_peek();
        let remote = row.host.is_some();
        self.parent_name = match (&self.index, &row.parent_id) {
            _ if remote => row.parent_id.as_deref().and_then(|pid| self.remote_rows.values().flatten().find(|r| r.id == pid)).map(clean_name),
            (Some(i), Some(pid)) => i.session(pid).ok().flatten().map(|p| clean_name(&p)),
            _ => None,
        };
        self.session = Some(row);
        self.sel = None;
        self.follow = false;
        self.reveal = None;
        self.event_scroll = 0;
        self.anchor = None;
        self.bodies.clear();
        self.body_pending.clear();
        self.body_failed.clear();
        // Folds belong to one session: a new one starts with its defaults.
        self.ofolds = FoldState::new();
        self.oseen.clear();
        self.filter_seen = FilterSig::default();
        self.event_errors_only = false;
        self.event_query.clear();
        self.tool_filter = None;
        self.remote_all.clear();
        self.remote_loading = false;
        self.remote_ev = None;
        self.tools = match (&self.index, &self.session) {
            _ if remote => Vec::new(),
            (Some(i), Some(s)) => i.facets(&s.id).map(|f| f.tools.into_iter().map(|(t, _)| t).collect()).unwrap_or_default(),
            _ => Vec::new(),
        };
        self.mode = Mode::Trajectory;
        self.load_events();
    }

    /// Esc in a trajectory: back up the dispatch trail, else to the ledger.
    fn leave_trajectory(&mut self) {
        self.close_peek();
        if let Some(t) = self.trail.pop() {
            self.open_session(t.session);
            // Back where we left: the same folds and row. The row's ancestors open so it shows.
            self.ofolds = t.folds;
            self.oseen = t.seen;
            self.reveal = t.sel.clone();
            self.sel = t.sel;
            self.rebuild_outline();
        } else {
            self.mode = if self.live_return {
                Mode::Live
            } else {
                Mode::List
            };
            self.session = None;
            self.set_events(Vec::new());
            self.remote_loading = false;
            self.remote_ev = None;
            if std::mem::take(&mut self.live_return) {
                self.kick_fold(now_ms());
            }
        }
    }

    fn load_events(&mut self) {
        self.generation += 1;
        if self.session.as_ref().is_some_and(|s| s.host.is_some()) {
            return self.load_remote_events();
        }
        let loaded = {
            let (Some(index), Some(session)) = (&self.index, &self.session) else { return };
            query_trajectory(index, &session.id, &self.event_filter())
        };
        match loaded {
            Ok((rows, session, children)) => self.install_trajectory(rows, session, children),
            Err(e) => self.set_status(format!("query failed: {e}")),
        }
    }

    fn install_trajectory(&mut self, rows: Vec<EventRow>, session: Option<SessionRow>, children: Vec<SessionRow>) {
        self.set_events(rows);
        if session.is_some() {
            self.session = session;
        }
        self.session_children = children;
        self.dispatch_child = self.match_dispatches();
    }

    /// Keeps every loaded event; a tool result right after a call (unless noise is shown) is hidden
    /// and recorded on that call row as `folded`. Then rebuilds the outline.
    fn set_events(&mut self, rows: Vec<EventRow>) {
        let mut folded: Vec<Option<Fold>> = vec![None; rows.len()];
        let mut hidden = vec![false; rows.len()];
        let mut last: Option<usize> = None;
        for (i, e) in rows.iter().enumerate() {
            let call = last.filter(|&j| rows.get(j).is_some_and(is_call) && folded.get(j).is_some_and(Option::is_none));
            match call {
                Some(j) if !self.include_noise && is_result(e) => {
                    if let Some(f) = folded.get_mut(j) {
                        *f = Some(Fold { error: e.is_error, detail: e.detail.clone() });
                    }
                    if let Some(h) = hidden.get_mut(i) {
                        *h = true;
                    }
                }
                _ => last = Some(i),
            }
        }
        (self.events, self.folded, self.hidden) = (rows, folded, hidden);
        self.rebuild_outline();
    }

    // ---- trajectory outline: rows, folds, selection, bodies ----

    fn filtering(&self) -> bool {
        !self.event_query.is_empty() || self.event_errors_only || self.tool_filter.is_some()
    }

    fn key_at(&self, node: usize) -> Option<&OutlineKey> {
        self.outline.nodes.get(node).map(|n| &n.key)
    }

    /// Fold mark before an outline row: `› ` collapsed, `⌄ ` expanded, two spaces when there is nothing to fold.
    fn fold_mark(&self, node: usize) -> String {
        match self.key_at(node) {
            Some(key) if self.outline.has_children(node) => {
                let glyph = if self.ofolds.is_collapsed(key) { crate::glyphs::chevron() } else { crate::glyphs::chevron_down() };
                format!("{glyph} ")
            }
            _ => "  ".to_string(),
        }
    }

    /// Position of the selected row in `oshown`; the first row when nothing is selected.
    fn sel_pos(&self) -> usize {
        let Some(key) = self.sel.clone() else {
            return 0;
        };
        let key = self.canon(key);
        self.oshown
            .iter()
            .position(|&n| self.key_at(n) == Some(&key))
            .unwrap_or(0)
    }

    /// Selects the visible row at `pos`. A row of height 0 is not landable: search continues in the
    /// direction of travel, then the other way.
    fn goto(&mut self, pos: usize) {
        let pos = self.skip_unshown(pos);
        self.sel = self.oshown.get(pos).and_then(|&n| self.key_at(n)).cloned();
    }

    fn step(&mut self, delta: isize) {
        let mut pos = self.sel_pos();
        move_sel(&mut pos, self.oshown.len(), delta);
        self.goto(pos);
    }

    fn session_active(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.status == Status::Active)
    }

    /// Screen lines of shown row `pos` from the last render. Until that render matches `oshown`, every
    /// row counts as one line so a fold does not skip on stale heights.
    fn shown_height(&self, pos: usize) -> usize {
        if self.row_h.len() != self.oshown.len() {
            return 1;
        }
        self.row_h.get(pos).copied().unwrap_or(1)
    }

    /// `pos`, or the nearest shown row that draws at least one line.
    fn skip_unshown(&self, pos: usize) -> usize {
        let len = self.oshown.len();
        if len == 0 {
            return 0;
        }
        let pos = pos.min(len - 1);
        if self.shown_height(pos) > 0 {
            return pos;
        }
        let origin = self.sel_pos().min(len - 1);
        let dir: isize = if pos > origin { 1 } else { -1 };
        self.walk_shown(pos, dir)
            .or_else(|| self.walk_shown(pos, -dir))
            .unwrap_or(pos)
    }

    /// The next shown row in direction `dir` whose height is not 0.
    fn walk_shown(&self, start: usize, dir: isize) -> Option<usize> {
        let len = self.oshown.len() as isize;
        let mut i = start as isize + dir;
        while i >= 0 && i < len {
            if self.shown_height(i as usize) > 0 {
                return Some(i as usize);
            }
            i += dir;
        }
        None
    }

    /// Remembers `heights` for selection. A selection sitting on a height-0 row moves off it.
    fn note_heights(&mut self, heights: &[usize]) {
        self.row_h = heights.to_vec();
        let pos = self.sel_pos();
        if self.row_h.get(pos).copied() == Some(0) {
            self.goto(pos);
        }
    }

    /// Event index of the selected row: a turn's opener (its prompt rows too), a run's first call, or the row's own event.
    fn selected_event(&self) -> Option<usize> {
        match self.outline_items.get(*self.oshown.get(self.sel_pos())?)? {
            OutlineItem::Turn { event, .. } | OutlineItem::Run { first: event, .. } => Some(*event),
            OutlineItem::Event(i) | OutlineItem::Body(i) => Some(*i),
            OutlineItem::Prompt(i) | OutlineItem::PromptMore(i) | OutlineItem::PromptRest(i) => {
                Some(*i)
            }
        }
    }

    /// A visible event failed: its own `is_error`, or a tool result folded onto that call.
    /// Hidden rows are skipped; their failure is already on the call they fold into.
    fn event_failed(&self, i: usize) -> bool {
        if self.hidden.get(i).copied().unwrap_or(false) {
            return false;
        }
        let folded = self.folded.get(i).and_then(|f| f.as_ref()).is_some_and(|f| f.error);
        self.events.get(i).is_some_and(|e| e.is_error) || folded
    }

    /// `!` next error, `*` previous, wrapping. Status is `error K/N`, or `no errors` with no move.
    fn jump_error(&mut self, forward: bool) {
        let errors: Vec<usize> = (0..self.events.len()).filter(|&i| self.event_failed(i)).collect();
        let n = errors.len();
        if n == 0 {
            self.set_status("no errors".to_string());
            return;
        }
        let cur = self.selected_event();
        let at = if forward {
            let start = cur.unwrap_or(usize::MAX);
            errors.iter().position(|&i| i > start).unwrap_or(0)
        } else {
            let start = cur.unwrap_or(0);
            errors.iter().rposition(|&i| i < start).unwrap_or(n - 1)
        };
        let Some(&i) = errors.get(at) else { return };
        self.focus_event(i);
        self.set_status(format!("error {}/{n}", at + 1));
    }

    /// Reveals event `i` (expanding collapsed ancestors) and selects that outline row by key.
    fn focus_event(&mut self, i: usize) {
        let Some(node) = self
            .outline_items
            .iter()
            .position(|it| matches!(it, OutlineItem::Event(ev) if *ev == i))
        else {
            return;
        };
        let Some(key) = self.key_at(node).cloned() else { return };
        self.reveal(&key);
        self.reshow(node);
    }

    /// The key a stored selection means now: an opener's event row is folded into its turn row, and a
    /// prompt row that is no longer in the outline falls back to its turn.
    fn canon(&self, key: OutlineKey) -> OutlineKey {
        let line = match &key {
            OutlineKey::Ev(l) if self.opener_lines.contains(l) => return OutlineKey::Turn(*l),
            OutlineKey::Prompt(l) | OutlineKey::PromptMore(l) | OutlineKey::PromptRest(l)
                if self.opener_lines.contains(l) && !self.outline.nodes.iter().any(|n| n.key == key) =>
            {
                *l
            }
            _ => return key,
        };
        OutlineKey::Turn(line)
    }

    /// The turn row above outline node `node` (or the node itself): its event index and counts.
    fn owning_turn(&self, node: usize) -> Option<(usize, usize, usize, usize)> {
        let mut at = Some(node);
        while let Some(i) = at {
            if let Some(OutlineItem::Turn {
                event,
                tools,
                msgs,
                errors,
                ..
            }) = self.outline_items.get(i)
            {
                return Some((*event, *tools, *msgs, *errors));
            }
            at = self.outline.nodes.get(i).and_then(|x| x.parent);
        }
        None
    }

    /// Continuation rows of event `ev`'s prompt past its turn header, wrapped to `text_w` cells (the text
    /// cell, which starts at the turn's mark). Empty when the header holds the whole prompt, or the width
    /// is not measured yet. The full text is the cached body, else the index text.
    fn prompt_lines(
        &self,
        ev: usize,
        (tools, msgs, errors): (usize, usize, usize),
        text_w: usize,
    ) -> Vec<String> {
        let Some(e) = self.events.get(ev) else {
            return Vec::new();
        };
        if text_w == 0 {
            return Vec::new();
        }
        let (head, _) = turn_head(
            &one_line(index_text(e)),
            text_w,
            &turn_counts(tools, msgs, errors),
        );
        let rest = continuation(&head, &continuation_full(self.bodies.get(&e.line), index_text(e)));
        if rest.is_empty() {
            return Vec::new();
        }
        wrap_body(&rest, text_w.saturating_sub(MARK_W))
    }

    /// `prompt_lines` for the turn that owns outline node `node`.
    fn prompt_rows_at(&self, node: usize, text_w: usize) -> Vec<String> {
        match self.owning_turn(node) {
            Some((ev, tools, msgs, errors)) => self.prompt_lines(ev, (tools, msgs, errors), text_w),
            None => Vec::new(),
        }
    }

    /// The parts of event row `i` at outline depth `depth` on a trajectory `width` columns wide.
    fn event_head(&self, i: usize, depth: usize, width: usize) -> EventHead {
        let Some(e) = self.events.get(i) else {
            return EventHead::default();
        };
        let row_text = one_line(index_text(e));
        let (chip, age, tokens, summary_w) = event_cols(width, e, &row_text);
        let mut suffix = String::new();
        let mut failed = e.is_error;
        if let Some(c) = self.child_of(i) {
            suffix.push_str(&format!(
                "→ {} {}",
                truncate_str(&clean_name(c), 28),
                status_symbol(c)
            ));
        }
        if let Some(Some(f)) = self.folded.get(i) {
            failed |= f.error;
            if !suffix.is_empty() {
                suffix.push(' ');
            }
            suffix.push_str(if f.error { "✗" } else { "✓" });
            if f.error
                && let Some(d) = f.detail.as_deref().filter(|d| !d.is_empty())
            {
                suffix.push_str(&format!(" {}", truncate_str(&one_line(d), 24)));
            }
        }
        if echoed(&suffix, &row_text) {
            suffix.clear();
        }
        let suffix_w = suffix.width().min(summary_w / 2);
        let body_w = summary_w.saturating_sub(if suffix_w > 0 { suffix_w + 1 } else { 0 });
        let title = if is_call(e) && !text_has_title(&e.title, &row_text) {
            format!("{}  ", e.title)
        } else {
            String::new()
        };
        let head = first_line(
            &row_text,
            body_w.saturating_sub(2 * depth + MARK_W + title.width()),
        );
        EventHead {
            title,
            head,
            suffix,
            suffix_w,
            body_w,
            failed,
            chip,
            age,
            tokens,
        }
    }

    /// Full text of the turn a turn or prompt row belongs to (for `y`): the cached body, else the index text.
    fn turn_prompt_text(&self) -> Option<String> {
        let &n = self.oshown.get(self.sel_pos())?;
        let on_prompt = matches!(
            self.outline_items.get(n)?,
            OutlineItem::Turn { .. }
                | OutlineItem::Prompt(_)
                | OutlineItem::PromptMore(_)
                | OutlineItem::PromptRest(_)
        );
        if !on_prompt {
            return None;
        }
        let (ev, ..) = self.owning_turn(n)?;
        let e = self.events.get(ev)?;
        Some(
            self.bodies
                .get(&e.line)
                .cloned()
                .unwrap_or_else(|| index_text(e).to_string()),
        )
    }

    /// Rebuilds the outline from `events`. The selection stays on its key when that row is still
    /// shown, else on the row at the old position.
    fn rebuild_outline(&mut self) {
        let pos = self.sel_pos();
        let w = self.outline_w;
        let (tree, items) =
            traces_outline::outline(&self.events, &self.hidden, |ev, tools, msgs, errors| {
                self.prompt_lines(ev, (tools, msgs, errors), w).len()
            });
        (self.outline, self.outline_items) = (tree, items);
        self.opener_lines = self
            .outline_items
            .iter()
            .filter_map(|it| match it {
                OutlineItem::Turn {
                    event,
                    opener: true,
                    ..
                } => self.events.get(*event).map(|e| e.line),
                _ => None,
            })
            .collect();
        // An empty outline (remote events still loading, or no match) keeps the folds and selection for when rows return.
        if !self.outline.nodes.is_empty() {
            self.ofolds.retain(&self.outline);
            let live: HashSet<OutlineKey> = self.outline.nodes.iter().map(|n| n.key.clone()).collect();
            self.oseen.retain(|k| live.contains(k));
        }
        // Defaults on first sight: turns open, short runs open, long runs and event rows closed. Under a
        // filter, turns and runs open on first sight and again when the filter itself changes, so a manual
        // collapse survives live polls.
        let sig = self.filter_sig();
        let refilter = sig != self.filter_seen;
        self.filter_seen = sig;
        let filtering = self.filtering();
        for i in 0..self.outline.nodes.len() {
            let Some(key) = self.key_at(i).cloned() else { continue };
            if !self.outline.has_children(i) {
                continue;
            }
            let turn = matches!(key, OutlineKey::Turn(_));
            let run = matches!(key, OutlineKey::Run(_));
            let small_run = run
                && matches!(
                    self.outline_items.get(i),
                    Some(OutlineItem::Run { count, .. }) if *count <= RUN_OPEN_MAX_CALLS
                );
            let fresh = self.oseen.insert(key);
            if fresh && !turn && !(filtering && run) && !small_run {
                self.ofolds.collapse(&self.outline, i);
            }
            if filtering && (turn || run) && (fresh || refilter) {
                self.ofolds.expand(&self.outline, i);
            }
        }
        // A key not found while remote events are still loading stays pending for when they arrive.
        if let Some(key) = self.reveal.take()
            && !self.reveal(&key)
            && self.events.is_empty()
        {
            self.reveal = Some(key);
        }
        self.oshown = self.outline.visible(&self.ofolds);
        self.row_h.clear();
        self.place(self.sel.clone(), pos);
    }

    /// Expands every ancestor of the outline row `key` so it is shown. False when there is no such row.
    fn reveal(&mut self, key: &OutlineKey) -> bool {
        let key = self.canon(key.clone());
        let Some(i) = self.outline.nodes.iter().position(|n| n.key == key) else {
            return false;
        };
        let mut at = self.outline.nodes.get(i).and_then(|n| n.parent);
        while let Some(p) = at {
            self.ofolds.expand(&self.outline, p);
            at = self.outline.nodes.get(p).and_then(|n| n.parent);
        }
        true
    }

    fn filter_sig(&self) -> FilterSig {
        (self.event_query.clone(), self.event_errors_only, self.tool_filter)
    }

    /// The selected row is a run of tool calls.
    fn selected_run(&self) -> bool {
        self.oshown.get(self.sel_pos()).and_then(|&n| self.outline_items.get(n)).is_some_and(|item| matches!(item, OutlineItem::Run { .. }))
    }

    /// Selects `keep` if it is shown, else the row at `pos` (clamped). With no rows shown, the selection is left as it is.
    fn place(&mut self, keep: Option<OutlineKey>, pos: usize) {
        if self.oshown.is_empty() {
            return;
        }
        let keep = keep.map(|k| self.canon(k));
        let at = keep
            .as_ref()
            .and_then(|k| self.oshown.iter().position(|&n| self.key_at(n) == Some(k)))
            .unwrap_or_else(|| pos.min(self.oshown.len().saturating_sub(1)));
        self.goto(at);
    }

    /// Re-lists the outline after a fold change on `node`, keeping the selection on it or its nearest shown ancestor.
    fn reshow(&mut self, node: usize) {
        self.oshown = self.outline.visible(&self.ofolds);
        self.row_h.clear();
        let mut at = Some(node);
        while let Some(i) = at {
            if self.oshown.contains(&i) {
                break;
            }
            at = self.outline.nodes.get(i).and_then(|n| n.parent);
        }
        let keep = at.and_then(|i| self.key_at(i)).cloned();
        let pos = self.sel_pos();
        self.place(keep, pos);
    }

    /// Applies a fold op to the selected row. False when it changed nothing.
    fn outline_fold(&mut self, op: impl FnOnce(&mut FoldState<OutlineKey>, &FoldTree<OutlineKey>, usize)) -> bool {
        let Some(&node) = self.oshown.get(self.sel_pos()) else { return false };
        let before = self.ofolds.clone();
        op(&mut self.ofolds, &self.outline, node);
        if self.ofolds == before {
            return false;
        }
        self.reshow(node);
        true
    }

    /// `h` / `←`: collapse the selected row; on a leaf or a collapsed row, select its parent.
    fn outline_collapse(&mut self) -> bool {
        let Some(&node) = self.oshown.get(self.sel_pos()) else { return false };
        let before = self.ofolds.clone();
        match self.ofolds.collapse(&self.outline, node) {
            Some(parent) => {
                self.reshow(parent);
                true
            }
            None if self.ofolds != before => {
                self.reshow(node);
                true
            }
            None => false,
        }
    }

    /// Wrapped rows per shown row (`None` for rows that are one line), `text_w` cells wide.
    fn body_rows(&self, text_w: usize) -> Vec<Option<Vec<String>>> {
        self.oshown
            .iter()
            .map(|&n| {
                let depth = self.outline.nodes.get(n).map_or(0, |x| x.depth);
                match self.outline_items.get(n) {
                    Some(OutlineItem::Body(i)) => {
                        // The event row above holds the header; its text continues under it.
                        let head = self.event_head(*i, depth.saturating_sub(1), text_w + TEXT_COL).head;
                        Some(self.body_lines(*i, &head, text_w.saturating_sub(2 * depth)))
                    }
                    Some(OutlineItem::Prompt(_)) => Some(
                        self.prompt_rows_at(n, text_w)
                            .into_iter()
                            .take(PROMPT_LEAD)
                            .collect(),
                    ),
                    Some(OutlineItem::PromptRest(_)) => Some(
                        self.prompt_rows_at(n, text_w)
                            .into_iter()
                            .skip(PROMPT_LEAD)
                            .collect(),
                    ),
                    _ => None,
                }
            })
            .collect()
    }

    /// Loads the bodies of the rows shown at `oshown` positions `from..from + height` that are not cached:
    /// local rows now, remote rows on a worker (at most `MAX_BODY_FETCHES` in flight). True when a body
    /// was cached here, so the caller should measure the rows again.
    fn ensure_bodies(&mut self, from: usize, height: usize) -> bool {
        let Some(session) = self.session.clone() else { return false };
        // (event index, its line): the body is cached by the line of the event that owns it.
        let wanted: Vec<(usize, i64)> = self
            .oshown
            .iter()
            .skip(from)
            .take(height)
            .filter_map(|&n| match self.outline_items.get(n)? {
                OutlineItem::Body(i) => self.events.get(*i).map(|e| (*i, e.line)),
                OutlineItem::Turn {
                    event,
                    opener: true,
                    ..
                } => self.events.get(*event).map(|e| (*event, e.line)),
                _ => None,
            })
            .collect();
        let mut cached = false;
        for (i, line) in wanted {
            if self.bodies.contains_key(&line) || self.body_pending.contains(&line) || self.body_failed.contains(&line) {
                continue;
            }
            let result = self.result_of(i);
            if session.host.is_some() {
                let Some(target) = self.target_for(&session) else { continue };
                if self.body_pending.len() >= MAX_BODY_FETCHES {
                    break;
                }
                self.body_pending.insert(line);
                let tx = self.body_tx.clone();
                let sid = session.id.clone();
                std::thread::spawn(move || {
                    let fetch = |l: i64| remote::fetch_raw(&target, remote::remote_id(&target, &sid), l).map_err(|e| e.to_string());
                    let text = compose_body(fetch(line), result.map(|(l, error)| (fetch(l), error)));
                    let _ = tx.send(BodyMsg { session_id: sid, line, text });
                });
            } else {
                let text = match &self.index {
                    Some(index) => {
                        let fetch = |l: i64| index.raw_line(&session.id, l).map_err(|e| e.to_string());
                        compose_body(fetch(line), result.map(|(l, error)| (fetch(l), error)))
                    }
                    None => continue,
                };
                cached |= self.settle_body(line, text);
            }
        }
        cached
    }

    /// Takes a loaded body. A failure is not cached: the row shows nothing until `l` retries it.
    fn settle_body(&mut self, line: i64, text: Result<String, String>) -> bool {
        self.body_pending.remove(&line);
        match text {
            Ok(text) => {
                if self.opener_lines.contains(&line) {
                    self.dirty_outline = true;
                }
                self.bodies.insert(line, text);
                true
            }
            Err(e) => {
                self.body_failed.insert(line);
                self.set_status(format!("(could not read #{line}: {e}, press l to retry)"));
                false
            }
        }
    }

    /// `l` on a row that did not fold: forget failed body reads so the next render reads them again.
    fn retry_bodies(&mut self) -> bool {
        if self.body_failed.is_empty() {
            return false;
        }
        self.body_failed.clear();
        if self.status.as_deref().is_some_and(|s| s.starts_with("(could not read")) {
            self.clear_status();
        }
        true
    }

    /// The result folded into event `i`'s call row: its line and whether it failed.
    fn result_of(&self, i: usize) -> Option<(i64, bool)> {
        let fold = self.folded.get(i)?.as_ref()?;
        Some((self.events.get(i + 1)?.line, fold.error))
    }

    /// The body of event `ev` wrapped to `width`, minus what its header `head` already shows, then its folded
    /// result. Empty when the header already holds the text and there is no result. A placeholder until
    /// the body is read.
    fn body_lines(&self, ev: usize, head: &str, width: usize) -> Vec<String> {
        let Some(line) = self.events.get(ev).map(|e| e.line) else { return vec!["loading…".to_string()] };
        match self.bodies.get(&line) {
            Some(text) => {
                let (own, result) = split_result(text);
                let rest = continuation(head, &continuation_source(own));
                let mut rows = if rest.is_empty() { Vec::new() } else { wrap_body(&rest, width) };
                if !result.is_empty() {
                    rows.push(String::new());
                    rows.extend(wrap_body(result, width));
                }
                rows
            }
            None if self.body_failed.contains(&line) => vec!["(could not read, press l to retry)".to_string()],
            None => vec!["loading…".to_string()],
        }
    }

    /// The child session a dispatch row (Agent/Task tool call) started, if one matches.
    fn child_of(&self, i: usize) -> Option<&SessionRow> {
        self.session_children.get(self.dispatch_child.get(i).copied().flatten()?)
    }

    /// Per event, the index into `session_children` its dispatch started. Computed once per load.
    fn match_dispatches(&self) -> Vec<Option<usize>> {
        let mut out = vec![None; self.events.len()];
        if self.session_children.is_empty() {
            return out;
        }
        let dispatches: Vec<usize> = (0..self.events.len()).filter(|&j| is_dispatch(&self.events[j])).collect();
        // Without a label match, pair the nth dispatch with the nth child when the counts agree.
        let positional = self.event_query.is_empty() && !self.event_errors_only && dispatches.len() == self.session_children.len();
        for (n, &i) in dispatches.iter().enumerate() {
            let e = &self.events[i];
            let want = norm(e.detail.as_deref().unwrap_or(&e.preview));
            let by_label = (!want.is_empty())
                .then(|| {
                    self.session_children.iter().position(|c| {
                        let l = norm(c.label.as_deref().unwrap_or_default());
                        !l.is_empty() && (l == want || l.starts_with(&want) || want.starts_with(&l))
                    })
                })
                .flatten();
            out[i] = by_label.or(positional.then_some(n));
        }
        out
    }

    fn enter_child(&mut self, child: SessionRow) {
        if let Some(session) = self.session.clone() {
            self.trail.push(Trail { session, sel: self.sel.clone(), folds: self.ofolds.clone(), seen: self.oseen.clone() });
        }
        self.open_session(child);
    }

    fn open_inspector(&mut self) {
        if self.session.as_ref().is_some_and(|s| s.host.is_some()) {
            return self.open_remote_inspector();
        }
        let Some(line) = self.selected_event().and_then(|i| self.events.get(i)).map(|e| e.line) else { return };
        let (Some(index), Some(session)) = (&self.index, &self.session) else { return };
        self.raw = Some(pretty(index.raw_line(&session.id, line).map_err(|e| e.to_string())));
        let (call, shape) = self.payload_target().unwrap_or((line, Shape::Prose));
        let result = self.payload_result();
        let fetch = |l: i64| index.raw_line(&session.id, l).map_err(|e| e.to_string());
        let body = compose_body(fetch(call), result.map(|(l, error)| (fetch(l), error)));
        self.payload.open(Some(body), shape);
        self.raw_scroll = 0;
        self.mode = Mode::Inspector;
    }

    /// The line the payload view reads and its shape: the selected event, or the call whose folded result it is.
    fn payload_target(&self) -> Option<(i64, Shape)> {
        let i = self.selected_event()?;
        let e = self.events.get(i)?;
        if is_result(e) && i > 0 && self.result_of(i - 1).is_some_and(|(l, _)| l == e.line) {
            return Some((self.events.get(i - 1)?.line, Shape::Call));
        }
        Some((e.line, if is_call(e) { Shape::Call } else if is_result(e) { Shape::Result } else { Shape::Prose }))
    }

    /// The result folded into the payload's call, if any.
    fn payload_result(&self) -> Option<(i64, bool)> {
        let i = self.selected_event()?;
        let call = if self.events.get(i).is_some_and(is_result) && i > 0 { i - 1 } else { i };
        self.result_of(call)
    }

    /// The raw source line for the inspected event, unformatted.
    fn raw_source_line(&self) -> Option<String> {
        let line = self.events.get(self.selected_event()?)?.line;
        self.raw_at(line)
    }

    /// Raw source line for event `line` (inspector read, else a synthetic line).
    fn raw_at(&self, line: i64) -> Option<String> {
        if self.session.as_ref().is_some_and(|s| s.host.is_some()) {
            return self.raw_source.clone();
        }
        if let (Some(index), Some(session)) = (&self.index, &self.session)
            && let Some(text) = index.raw_line(&session.id, line).ok().flatten()
        {
            return Some(text);
        }
        self.raw_lines.get(&line).cloned()
    }

    fn selected_tool_call(&self) -> Option<usize> {
        let n = *self.oshown.get(self.sel_pos())?;
        match self.outline_items.get(n)? {
            OutlineItem::Event(i) | OutlineItem::Body(i) if self.events.get(*i).is_some_and(is_call) => Some(*i),
            _ => None,
        }
    }

    /// `p`: peek a tool call while its session is active, or once the call has a result.
    fn open_peek(&mut self) -> bool {
        let Some(i) = self.selected_tool_call() else { return false };
        let active = self.session.as_ref().is_some_and(|s| s.status == Status::Active);
        if !active && self.result_of(i).is_none() {
            return false;
        }
        self.peek = self.events.get(i).map(|e| e.line);
        self.raw_scroll = 0;
        self.refresh_peek();
        self.peek.is_some()
    }

    fn close_peek(&mut self) {
        self.peek = None;
        self.raw = None;
        self.raw_scroll = 0;
    }

    fn refresh_peek(&mut self) -> bool {
        let text = self.peek_text();
        let changed = self.raw.as_deref() != Some(text.as_str());
        self.raw = Some(text);
        changed
    }

    /// Tail of the call's output. A finished call reads its result line; a running one, its own.
    fn peek_text(&self) -> String {
        let Some(line) = self.peek else { return "no output yet".into() };
        let call = self.events.iter().position(|e| e.line == line && is_call(e));
        let src = call.and_then(|i| self.result_of(i).map(|(l, _)| l).or(Some(line))).and_then(|l| self.raw_at(l));
        src.as_deref().and_then(peek_output).filter(|t| !t.is_empty()).map(|t| tail_lines(&t, PEEK_TAIL)).unwrap_or_else(|| "no output yet".into())
    }

    fn resume(&mut self, row: Option<SessionRow>) -> InputOutcome {
        let Some(row) = row else { return InputOutcome::Unchanged };
        if let Some(host) = &row.host {
            return InputOutcome::Action(Action::TracesNotice { text: format!("remote session on {host}: resume it there") });
        }
        match resume_action(&row) {
            Some(ResumeAction::InPager(t)) => InputOutcome::Action(Action::TracesResume { source: t.source, native_id: t.native_id }),
            Some(ResumeAction::CopyCommand(text)) => {
                self.copy_note = Some(text.clone());
                InputOutcome::Action(Action::TracesCopy { text })
            }
            None => InputOutcome::Action(Action::TracesNotice { text: format!("no resume command for {}", row.source) }),
        }
    }

    // ---- remote hosts: every ssh call runs on its own thread; the 1s poll collects results ----

    fn target_for(&self, row: &SessionRow) -> Option<String> {
        self.hosts.iter().find(|t| Some(remote::label(t)) == row.host.as_deref()).cloned()
    }

    /// Collect finished fetches and start the next round when due. True when something changed.
    fn poll_remote(&mut self) -> bool {
        let mut redraw = false;
        let mut done = Vec::new();
        if let Some(rx) = &self.remote_rx {
            loop {
                match rx.try_recv() {
                    Ok(h) => done.push(h),
                    Err(mpsc::TryRecvError::Empty) => break,
                    // Every host thread has exited; one that panicked never answered, so stop waiting for it.
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.remote_pending = 0;
                        break;
                    }
                }
            }
        }
        for h in done {
            self.remote_pending = self.remote_pending.saturating_sub(1);
            self.apply_host(h);
            redraw = true;
        }
        if self.remote_pending == 0 {
            self.remote_rx = None;
            if !(self.mode == Mode::Live || self.live_return || self.hosts.is_empty()) && self.remote_at.is_none_or(|t| t.elapsed() >= REMOTE_EVERY) {
                self.remote_at = Some(Instant::now());
                let (tx, rx) = mpsc::channel();
                self.remote_rx = Some(rx);
                self.remote_pending = self.hosts.len();
                for target in self.hosts.clone() {
                    let tx = tx.clone();
                    std::thread::spawn(move || {
                        let result = remote::fetch_sessions(&target, &RemoteFilter { limit: 200, ..Default::default() });
                        let _ = tx.send(HostResult { target, result });
                    });
                }
            }
        }
        if let Some(rx) = &self.remote_ev
            && let Ok(ev) = rx.try_recv()
        {
            self.remote_ev = None;
            if self.mode != Mode::List && self.session.as_ref().is_some_and(|s| s.id == ev.session_id) {
                self.remote_loading = false;
                match ev.result {
                    Ok(rows) => {
                        self.remote_noise = ev.noise;
                        self.tools = tool_names(&rows);
                        self.remote_all = rows;
                        self.filter_remote();
                    }
                    Err(e) => self.set_status(e.to_string()),
                }
                redraw = true;
            }
        }
        while let Ok(m) = self.body_rx.try_recv() {
            if self.session.as_ref().is_some_and(|s| s.id == m.session_id) {
                self.settle_body(m.line, m.text);
                redraw = true;
            }
        }
        if let Some(rx) = &self.raw_rx
            && let Ok((text, body)) = rx.try_recv()
        {
            self.raw_rx = None;
            self.raw_source = Some(text.clone());
            self.raw = Some(pretty(Ok(Some(text))));
            self.payload.body = Some(body);
            redraw = true;
        }
        redraw
    }

    /// Merge one host's answer; on failure keep showing its last good rows and say so.
    fn apply_host(&mut self, h: HostResult) {
        let keep = self.selected_key();
        match h.result {
            Ok(Fetched { rows, note }) => {
                self.remote_err.remove(&h.target);
                self.remote_rows.insert(h.target, rows);
                if let Some(note) = note {
                    self.set_status(note);
                }
            }
            Err(e) => {
                let mut msg = e.to_string();
                if let Some((age, rows)) = remote::cached_sessions(&h.target) {
                    msg.push_str(&format!(" · cached {}s ago", age.as_secs()));
                    self.remote_rows.insert(h.target.clone(), rows);
                }
                self.remote_err.insert(h.target, msg);
            }
        }
        self.rebuild_ledger();
        self.keep_selection(keep);
    }

    /// Fetch the open remote session's events, or refilter the ones we have.
    fn load_remote_events(&mut self) {
        if !self.remote_all.is_empty() && self.remote_noise == self.include_noise {
            return self.filter_remote();
        }
        let Some(session) = self.session.clone() else { return };
        let Some(target) = self.target_for(&session) else {
            self.set_status("host is no longer configured");
            return;
        };
        self.remote_loading = true;
        self.install_trajectory(Vec::new(), None, Vec::new());
        let (tx, rx) = mpsc::channel();
        self.remote_ev = Some(rx);
        let noise = self.include_noise;
        std::thread::spawn(move || {
            let result = remote::fetch_events(&target, remote::remote_id(&target, &session.id), &EventQuery { include_noise: noise, limit: EVENT_PAGE });
            let _ = tx.send(RemoteEvents { session_id: session.id, noise, result });
        });
    }

    /// Remote events carry no server-side filters; apply the trajectory filters here.
    fn filter_remote(&mut self) {
        let q = self.event_query.to_lowercase();
        let tool = self.tool_filter.and_then(|i| self.tools.get(i)).cloned();
        let rows: Vec<EventRow> = self
            .remote_all
            .iter()
            .filter(|e| !self.event_errors_only || e.is_error)
            .filter(|e| tool.as_deref().is_none_or(|t| e.kind == "tool" && e.title == t))
            .filter(|e| {
                q.is_empty() || [Some(&e.title), Some(&e.preview), e.detail.as_ref(), e.intent.as_ref()].into_iter().flatten().any(|f| f.to_lowercase().contains(&q))
            })
            .cloned()
            .collect();
        let kids = self.session.as_ref().and_then(|s| self.shown_children.get(&s.id).cloned()).unwrap_or_default();
        self.install_trajectory(rows, None, kids);
    }

    fn open_remote_inspector(&mut self) {
        let Some(line) = self.selected_event().and_then(|i| self.events.get(i)).map(|e| e.line) else { return };
        let Some(session) = self.session.clone() else { return };
        let Some(target) = self.target_for(&session) else { return };
        let (tx, rx) = mpsc::channel();
        self.raw_rx = Some(rx);
        self.raw_source = None;
        self.raw = Some("fetching from the remote host…".into());
        let (call, shape) = self.payload_target().unwrap_or((line, Shape::Prose));
        let result = self.payload_result();
        self.payload.open(None, shape);
        self.raw_scroll = 0;
        self.mode = Mode::Inspector;
        std::thread::spawn(move || {
            let fetch = |l: i64| remote::fetch_raw(&target, remote::remote_id(&target, &session.id), l).map_err(|e| e.to_string());
            let raw = fetch(line);
            let own = if call == line { raw.clone() } else { fetch(call) };
            let body = compose_body(own, result.map(|(l, error)| (fetch(l), error)));
            let text = match raw {
                Ok(Some(text)) => text,
                Ok(None) => String::new(),
                Err(e) => format!("{{\"error\": \"{e}\"}}"),
            };
            let _ = tx.send((text, body));
        });
    }
}

// ---------------------------------------------------------------- live screen

const HOUR_MS: i64 = 3_600_000;
const HINTS: &str = " enter open   j/k   h/l fold   / search   q quit";
const INSTALL_LINE: &str =
    " install hooks: crates/codegen/xai-grok-traces/hooks/install.sh --apply";

/// The last answer from one remote host: rows as fetched, when, and whether the latest fetch succeeded.
struct RemoteLive {
    rows: Vec<LiveRow>,
    fetched_at: Instant,
    ok: bool,
    note: Option<String>,
    /// The last answer was an empty one with a note (schema gate): stale rows from before must not come back.
    gated: bool,
}

struct LiveHostResult {
    target: String,
    result: Result<Fetched<LiveRow>, remote::Error>,
}

/// A finished local fold; `generation` lets a stale one be dropped.
struct LiveFold {
    generation: u64,
    /// When the fold was requested: the instant its ages are measured at.
    at: Instant,
    rows: Result<Vec<LiveRow>, String>,
    /// Titles of parents that are not among the rows (for `↑ <parent>`).
    titles: HashMap<String, String>,
    /// Hook receipts in the last hour, for the empty state.
    receipts_hour: i64,
    /// Whether any receipt exists; asked only while the state does not know yet.
    receipts_any: Option<bool>,
}

struct FoldReq {
    now_ms: i64,
    at: Instant,
    generation: u64,
    need_total: bool,
}

/// One long-lived thread that folds on request; it opens the index once and again only after an error.
struct FoldWorker {
    tx: mpsc::Sender<FoldReq>,
    rx: mpsc::Receiver<LiveFold>,
}

fn spawn_fold_worker(path: std::path::PathBuf) -> FoldWorker {
    let (tx, req_rx) = mpsc::channel::<FoldReq>();
    let (out_tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut index: Option<TraceIndex> = None;
        while let Ok(req) = req_rx.recv() {
            let ix = match index.take() {
                Some(ix) => ix,
                None => match TraceIndex::open(&path) {
                    Ok(ix) => ix,
                    Err(e) => {
                        let f = LiveFold::failed(&req, e.to_string());
                        if out_tx.send(f).is_err() {
                            break;
                        }
                        continue;
                    }
                },
            };
            let f = compute_fold(&ix, &req);
            if f.rows.is_ok() {
                index = Some(ix);
            }
            if out_tx.send(f).is_err() {
                break;
            }
        }
    });
    FoldWorker { tx, rx }
}

impl LiveFold {
    fn failed(req: &FoldReq, e: String) -> Self {
        Self {
            generation: req.generation,
            at: req.at,
            rows: Err(e),
            titles: HashMap::new(),
            receipts_hour: 0,
            receipts_any: None,
        }
    }
}

/// One displayed row: ages already advanced, state rendered, title composed.
struct LiveLine {
    row: LiveRow,
    child: bool,
    glyph: &'static str,
    title: String,
    text: String,
    marks: Vec<&'static str>,
    /// Collapsed root: number of hidden children.
    hidden: usize,
}

struct LiveView {
    lines: Vec<LiveLine>,
    agents: usize,
    waiting: usize,
}

/// Short host name: up to the first dot; `local` when it cannot be read.
fn short_hostname() -> String {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        // SAFETY: `buf` is valid for `buf.len()` bytes and gethostname NUL-terminates within it on success.
        let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
        if rc == 0 {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            let name = String::from_utf8_lossy(&buf[..end]).into_owned();
            if let Some(short) = name.split('.').next().filter(|s| !s.is_empty()) {
                return short.to_string();
            }
        }
    }
    "local".into()
}

/// `row` as it will be at render time: `extra_ms` added to its age, time in state and quiet time.
fn advance(row: &LiveRow, extra_ms: i64) -> LiveRow {
    let mut r = row.clone();
    if extra_ms != 0 {
        r.age_ms += extra_ms;
        r.since_ms = Some(r.since_ms.unwrap_or(0) + extra_ms);
        r.quiet_ms += extra_ms;
    }
    r
}

fn pad_to(s: &str, w: usize) -> String {
    let sw = s.width();
    if sw >= w {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(w - sw))
    }
}

/// `s` cut to `w - 1` columns plus `…`, padded to exactly `w`.
fn ellipsize(s: &str, w: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for ch in s.chars() {
        let c = ch.width().unwrap_or(0);
        if used + c > w.saturating_sub(1) {
            break;
        }
        out.push(ch);
        used += c;
    }
    let mut out = out.trim_end().to_string();
    out.push('…');
    pad_to(&out, w)
}

/// Hard cut to exactly `w` columns, ending in `…` when text was dropped.
fn cut(s: &str, w: usize) -> String {
    if w == 0 {
        return String::new();
    }
    if s.width() <= w {
        pad_to(s, w)
    } else {
        ellipsize(s, w)
    }
}

/// Exactly `w` columns: the first wrapped line of `text`; `…` (inside `w`) when text was dropped.
/// A single word longer than `w` is hard-cut.
fn fit(text: &str, w: usize) -> String {
    if w == 0 {
        return String::new();
    }
    let t = text.replace(['\r', '\n', '\t'], " ");
    if t.width() <= w {
        return pad_to(&t, w);
    }
    let first = modal_window::word_wrap(&t, w)
        .first()
        .copied()
        .unwrap_or("");
    ellipsize(first, w)
}

struct LiveCols {
    source: bool,
    ctx: bool,
    marks: bool,
    title: usize,
    fill: usize,
}

fn live_cols(width: usize) -> LiveCols {
    let title = if width >= 100 {
        28
    } else if width >= 60 {
        20
    } else {
        14
    };
    let (source, ctx, marks) = (width >= 48, width >= 100, width >= 36);
    // indent, glyph, then `sep + cell` per column: title, [source], [ctx], text, age, [marks].
    let fixed = 1
        + 2
        + if source { 8 } else { 0 }
        + 1
        + title
        + if ctx { 14 } else { 0 }
        + 1
        + 5
        + if marks { 5 } else { 0 };
    LiveCols {
        source,
        ctx,
        marks,
        title,
        fill: width.saturating_sub(fixed),
    }
}

fn ctx_text(row: &LiveRow) -> String {
    let k = |n: i64| (n + 500) / 1000;
    match (row.context_tokens, row.context_window) {
        (Some(t), Some(w)) => format!("ctx {}k/{}k", k(t), k(w)),
        (Some(t), None) => format!("ctx {}k", k(t)),
        _ => String::new(),
    }
}

fn glyph_color(glyph: &str, theme: &Theme) -> Color {
    match glyph {
        "⚠" | "⚠?" => theme.accent_error,
        "▶" | "⏳" | "⟳" => theme.accent_tool,
        "●" => theme.accent_running,
        _ => theme.gray,
    }
}

/// A `SessionRow` for a remote live row, enough for the ledger's remote open path.
fn remote_session(row: &LiveRow) -> SessionRow {
    SessionRow {
        id: row.session_id.clone(),
        host: row.host.clone(),
        source: row.source.clone(),
        title: row.title.clone(),
        parent_id: row.parent_id.clone(),
        native_id: row.native_id.clone(),
        context_tokens: row.context_tokens,
        context_window: row.context_window,
        ..Default::default()
    }
}

fn compute_fold(index: &TraceIndex, req: &FoldReq) -> LiveFold {
    let alive = |pid: i64, start: Option<i64>| match start {
        Some(s) => live::alive(pid, s),
        None => live::alive_pid(pid),
    };
    match live::fold_all(index, req.now_ms, alive) {
        Ok(rows) => {
            let ids: HashSet<&str> = rows.iter().map(|r| r.session_id.as_str()).collect();
            let titles = rows
                .iter()
                .filter_map(|r| r.parent_id.as_deref())
                .filter(|p| !ids.contains(p))
                .filter_map(|p| {
                    Some((p.to_string(), clean_name(&index.session(p).ok().flatten()?)))
                })
                .collect();
            LiveFold {
                generation: req.generation,
                at: req.at,
                rows: Ok(rows),
                titles,
                receipts_hour: index
                    .receipt_count_since(req.now_ms - HOUR_MS)
                    .unwrap_or(0),
                receipts_any: req
                    .need_total
                    .then(|| index.receipt_count_since(0).unwrap_or(0) > 0),
            }
        }
        Err(e) => LiveFold::failed(req, e.to_string()),
    }
}

impl TracesModalState {
    /// `grok live`'s state: Live mode from the start, and the first fold under way.
    pub fn open_live(index: Option<TraceIndex>, hosts: Vec<String>, standalone: bool) -> Self {
        let mut s = Self::open_with_hosts(index, hosts);
        s.standalone = standalone;
        s.enter_live();
        s
    }

    /// True once the user asked to leave the standalone live screen (q, or Esc with no query).
    pub fn live_wants_exit(&self) -> bool {
        self.live_exit
    }

    /// Ask the standalone live screen to close (Ctrl-C).
    pub fn request_live_exit(&mut self) {
        self.live_exit = true;
    }

    fn enter_live(&mut self) {
        self.mode = Mode::Live;
        self.live_gen += 1;
        self.kick_fold(now_ms());
        self.poll_remote_live();
    }

    /// Ask the worker for a fold unless one is already out.
    fn kick_fold(&mut self, now_ms: i64) {
        if self.live_inflight {
            return;
        }
        let Some(index) = &self.index else { return };
        let worker = self
            .live_worker
            .get_or_insert_with(|| spawn_fold_worker(index.db_path().to_path_buf()));
        let req = FoldReq {
            now_ms,
            at: Instant::now(),
            generation: self.live_gen,
            need_total: !self.live_receipts_any,
        };
        if worker.tx.send(req).is_ok() {
            self.live_inflight = true;
        } else {
            self.live_worker = None;
            self.set_status("fold: worker stopped");
        }
    }

    fn apply_fold(&mut self, f: LiveFold) {
        match f.rows {
            Ok(rows) => {
                self.live_rows = rows;
                self.live_titles = f.titles;
                self.live_fold_at = f.at;
                self.live_folded = true;
                self.live_receipts_hour = f.receipts_hour;
                self.live_receipts_any |= f.receipts_any.unwrap_or(false);
                if self
                    .status
                    .as_deref()
                    .is_some_and(|s| s.starts_with("fold: "))
                {
                    self.clear_status();
                }
            }
            Err(e) => self.set_status(format!("fold: {e}")),
        }
    }

    /// Live mode's share of the 1s poll: take a finished fold, start the next, collect remote answers.
    /// Always true: ages move every second.
    fn poll_live_screen(&mut self, now_ms: i64) -> bool {
        if self.live_inflight
            && let Some(w) = &self.live_worker
        {
            match w.rx.try_recv() {
                Ok(f) => {
                    self.live_inflight = false;
                    if f.generation == self.live_gen {
                        self.apply_fold(f);
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.live_inflight = false;
                    self.live_worker = None;
                    self.set_status("fold: worker stopped");
                }
            }
        }
        self.kick_fold(now_ms);
        self.poll_remote_live();
        true
    }

    fn poll_remote_live(&mut self) {
        let mut done = Vec::new();
        if let Some(rx) = &self.live_rx {
            loop {
                match rx.try_recv() {
                    Ok(h) => done.push(h),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.live_pending = 0;
                        break;
                    }
                }
            }
        }
        for h in done {
            self.live_pending = self.live_pending.saturating_sub(1);
            self.apply_live_host(h);
        }
        if self.live_pending == 0 {
            self.live_rx = None;
            if !self.hosts.is_empty() && self.live_at.is_none_or(|t| t.elapsed() >= REMOTE_EVERY) {
                self.live_at = Some(Instant::now());
                let (tx, rx) = mpsc::channel();
                self.live_rx = Some(rx);
                self.live_pending = self.hosts.len();
                for target in self.hosts.clone() {
                    let tx = tx.clone();
                    std::thread::spawn(move || {
                        let result = remote::fetch_live(&target);
                        let _ = tx.send(LiveHostResult { target, result });
                    });
                }
            }
        }
    }

    /// Ok replaces rows and note. Err shows the fetch cache (ages keep advancing from the fetch time) and keeps the note.
    fn apply_live_host(&mut self, h: LiveHostResult) {
        let now = Instant::now();
        let old_note = self.remote_live.get(&h.target).and_then(|r| r.note.clone());
        let was_gated = self.remote_live.get(&h.target).is_some_and(|r| r.gated);
        let entry = match h.result {
            Ok(Fetched { rows, note }) => RemoteLive {
                gated: rows.is_empty() && note.is_some(),
                rows,
                fetched_at: now,
                ok: true,
                note,
            },
            Err(_) => match remote::cached_live(&h.target).filter(|_| !was_gated) {
                Some((elapsed, rows)) => RemoteLive {
                    rows,
                    fetched_at: now.checked_sub(elapsed).unwrap_or(now),
                    ok: false,
                    note: old_note,
                    gated: false,
                },
                None => RemoteLive {
                    rows: Vec::new(),
                    fetched_at: now,
                    ok: false,
                    note: old_note,
                    gated: was_gated,
                },
            },
        };
        self.remote_live.insert(h.target, entry);
    }

    #[cfg(test)]
    fn fold_now(&mut self, now_ms: i64) {
        let Some(index) = &self.index else { return };
        let req = FoldReq {
            now_ms,
            at: Instant::now(),
            generation: self.live_gen,
            need_total: !self.live_receipts_any,
        };
        let f = compute_fold(index, &req);
        self.apply_fold(f);
    }

    #[cfg(test)]
    fn set_live(
        &mut self,
        local: Vec<LiveRow>,
        remote: Vec<(&str, Vec<LiveRow>, Instant, bool, Option<&str>)>,
    ) {
        self.live_rows = local;
        self.live_fold_at = Instant::now();
        self.live_folded = true;
        self.hosts = remote.iter().map(|r| r.0.to_string()).collect();
        self.remote_live = remote
            .into_iter()
            .map(|(t, rows, fetched_at, ok, note)| {
                (
                    t.to_string(),
                    RemoteLive {
                        rows,
                        fetched_at,
                        ok,
                        note: note.map(String::from),
                        gated: false,
                    },
                )
            })
            .collect();
    }

    fn row_title(&self, row: &LiveRow, child: bool, orphan: bool) -> String {
        let base = if orphan {
            let p = row.parent_id.as_deref().unwrap_or_default();
            let bare = row
                .host
                .as_deref()
                .and_then(|h| p.strip_prefix(h).and_then(|r| r.strip_prefix(':')))
                .unwrap_or(p);
            let title = if row.host.is_some() {
                bare.to_string()
            } else {
                self.live_titles
                    .get(p)
                    .cloned()
                    .unwrap_or_else(|| bare.to_string())
            };
            format!("↑ {title}")
        } else if child {
            one_line(
                row.label
                    .as_deref()
                    .filter(|l| !l.trim().is_empty())
                    .unwrap_or(&row.title),
            )
        } else {
            clean_name(&SessionRow {
                title: row.title.clone(),
                source: row.source.clone(),
                ..Default::default()
            })
        };
        match &row.host {
            Some(h) => format!("{h}: {base}"),
            None => base,
        }
    }

    /// Local and remote rows merged at `now`, ordered, filtered by the search, then collapsed.
    fn live_view(&self, now: Instant) -> LiveView {
        let local_extra = now.saturating_duration_since(self.live_fold_at).as_millis() as i64;
        let mut all: Vec<LiveRow> = self
            .live_rows
            .iter()
            .map(|r| advance(r, local_extra))
            .collect();
        for target in &self.hosts {
            if let Some(rl) = self.remote_live.get(target) {
                let extra = now.saturating_duration_since(rl.fetched_at).as_millis() as i64;
                all.extend(rl.rows.iter().map(|r| advance(r, extra)));
            }
        }
        live::order(&mut all);
        let ids: HashSet<String> = all.iter().map(|r| r.session_id.clone()).collect();
        let mut lines: Vec<LiveLine> = all
            .into_iter()
            .map(|row| {
                let parent = row.parent_id.as_deref().filter(|p| **p != row.session_id);
                let child = parent.is_some_and(|p| ids.contains(p));
                let orphan = parent.is_some_and(|p| !ids.contains(p));
                let r = live::render(&row, 0);
                LiveLine {
                    child,
                    glyph: r.glyph,
                    title: self.row_title(&row, child, orphan),
                    text: r.text,
                    marks: r.marks,
                    hidden: 0,
                    row,
                }
            })
            .collect();
        let q = self.live_query.to_lowercase();
        if !q.is_empty() {
            let hit: Vec<bool> = lines
                .iter()
                .map(|l| l.title.to_lowercase().contains(&q) || l.text.to_lowercase().contains(&q))
                .collect();
            let mut keep = hit.clone();
            for (i, l) in lines.iter().enumerate() {
                if hit[i]
                    && l.child
                    && let Some(p) = lines
                        .iter()
                        .position(|x| Some(&x.row.session_id) == l.row.parent_id.as_ref())
                {
                    keep[p] = true;
                }
            }
            let mut i = 0;
            lines.retain(|_| {
                i += 1;
                keep[i - 1]
            });
        }
        let agents = lines.len();
        let waiting = lines
            .iter()
            .filter(|l| l.row.state == LiveState::Waiting)
            .count();
        let mut out: Vec<LiveLine> = Vec::new();
        let mut folded: Option<usize> = None;
        for l in lines {
            if !l.child {
                folded = self
                    .live_collapsed
                    .contains(&l.row.session_id)
                    .then_some(out.len());
                out.push(l);
            } else if let Some(r) = folded {
                out[r].hidden += 1;
                if l.row.state == LiveState::Waiting && out[r].row.state != LiveState::Waiting {
                    out[r].glyph = "⚠";
                }
            } else {
                out.push(l);
            }
        }
        LiveView {
            lines: out,
            agents,
            waiting,
        }
    }

    /// Index of the selected line; the selection follows its session id, else keeps its position.
    /// Only user moves change `live_sel`: a fallback row is shown selected without replacing it.
    fn live_pos(&mut self, lines: &[LiveLine]) -> Option<usize> {
        if lines.is_empty() {
            return None;
        }
        let found = self
            .live_sel
            .as_ref()
            .and_then(|id| lines.iter().position(|l| l.row.session_id == *id));
        if let Some(i) = found {
            self.live_sel_idx = i;
            return Some(i);
        }
        Some(if self.live_sel.is_some() {
            self.live_sel_idx.min(lines.len() - 1)
        } else {
            0
        })
    }

    fn live_move(&mut self, delta: isize) {
        let view = self.live_view(Instant::now());
        let Some(i) = self.live_pos(&view.lines) else {
            return;
        };
        let j = (i as isize + delta).clamp(0, view.lines.len() as isize - 1) as usize;
        self.live_sel = Some(view.lines[j].row.session_id.clone());
        self.live_sel_idx = j;
    }

    /// Mouse wheel on the live screen (three rows a notch, as in the other modals).
    pub fn live_wheel(&mut self, down: bool) {
        self.live_move(if down { 3 } else { -3 });
    }

    /// Selected root: its index, whether the selection is a child, whether it has children,
    /// whether it is collapsed, and its session id.
    ///
    /// A childless root's next line is not a child, or its `hidden` count is 0 while collapsed.
    fn live_fold_target(&mut self) -> Option<(usize, bool, bool, bool, String)> {
        let view = self.live_view(Instant::now());
        let i = self.live_pos(&view.lines)?;
        let on_child = view.lines[i].child;
        let root = if on_child {
            view.lines[..i].iter().rposition(|l| !l.child).unwrap_or(i)
        } else {
            i
        };
        let has_children =
            view.lines[root].hidden > 0 || view.lines.get(root + 1).is_some_and(|l| l.child);
        let id = view.lines[root].row.session_id.clone();
        let collapsed = self.live_collapsed.contains(&id);
        Some((root, on_child, has_children, collapsed, id))
    }

    /// `h` / `←`: collapse the selected root. From a child, that selects the root.
    /// A collapsed root, or a root with no children, is unchanged and is not recorded.
    fn live_collapse(&mut self) -> InputOutcome {
        let Some((root, _on_child, has_children, collapsed, id)) = self.live_fold_target() else {
            return InputOutcome::Unchanged;
        };
        if !has_children || collapsed {
            return InputOutcome::Unchanged;
        }
        self.live_collapsed.insert(id.clone());
        self.live_sel = Some(id);
        self.live_sel_idx = root;
        InputOutcome::Changed
    }

    /// `l` / `→`: expand a collapsed root. The selection stays. Anything else is unchanged.
    fn live_expand(&mut self) -> InputOutcome {
        let Some((_, on_child, has_children, collapsed, id)) = self.live_fold_target() else {
            return InputOutcome::Unchanged;
        };
        if on_child || !has_children || !collapsed {
            return InputOutcome::Unchanged;
        }
        self.live_collapsed.remove(&id);
        InputOutcome::Changed
    }

    /// `c`: collapse or expand the selected row's root. From a child, collapse moves the selection to the root.
    fn live_toggle(&mut self) -> InputOutcome {
        match self.live_collapse() {
            InputOutcome::Unchanged => self.live_expand(),
            other => other,
        }
    }

    fn live_open(&mut self) {
        let view = self.live_view(Instant::now());
        let Some(i) = self.live_pos(&view.lines) else {
            return;
        };
        let row = &view.lines[i].row;
        let session = if row.host.is_some() {
            Some(remote_session(row))
        } else {
            self.index
                .as_ref()
                .and_then(|ix| ix.session(&row.session_id).ok().flatten())
        };
        match session {
            Some(s) => {
                self.live_return = true;
                self.trail.clear();
                self.open_session(s);
            }
            None => self.set_status(format!("cannot open {}", row.session_id)),
        }
    }

    fn live_back(&mut self) {
        if self.standalone {
            self.live_exit = true;
        } else {
            self.live_query.clear();
            self.mode = Mode::List;
        }
    }
}

// ---- live render

/// The live screen into `area`, no modal chrome.
pub fn render_live(buf: &mut Buffer, area: Rect, state: &mut TracesModalState, theme: &Theme) {
    render_live_at(buf, area, state, theme, Instant::now());
}

pub fn render_live_at(
    buf: &mut Buffer,
    area: Rect,
    state: &mut TracesModalState,
    theme: &Theme,
    now: Instant,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let w = area.width as usize;
    let dim = Style::default().fg(theme.gray);
    let view = state.live_view(now);

    // Header.
    let health = state.index.as_ref().and_then(|i| i.health());
    let (left, left_style) = if health.is_some() {
        (
            " index tripped · grok-traces doctor --reset".to_string(),
            Style::default().fg(theme.accent_error),
        )
    } else {
        let noun = if view.agents == 1 { "agent" } else { "agents" };
        let mut t = format!(" grok live · {} {noun}", view.agents);
        if view.waiting > 0 {
            t.push_str(&format!(" · {} waiting", view.waiting));
        }
        (t, Style::default().fg(theme.text_primary))
    };
    let mut hosts: Vec<(String, &str, Color)> =
        vec![(state.local_host.clone(), "●", theme.accent_running)];
    for t in &state.hosts {
        let (mark, color) = match state.remote_live.get(t) {
            None => ("…", theme.gray),
            Some(r) if r.ok => ("●", theme.accent_running),
            Some(_) => ("✗", theme.accent_error),
        };
        hosts.push((remote::label(t).to_string(), mark, color));
    }
    let host_w = |n: usize| {
        hosts[..n]
            .iter()
            .map(|(l, m, _)| l.width() + 1 + m.width())
            .sum::<usize>()
            + n.saturating_sub(1)
    };
    // The local host always stays: remote hosts drop first, then the left text is cut.
    let fits = (1..=hosts.len())
        .rev()
        .find(|&n| left.width() + 1 + host_w(n) < w);
    let shown = fits.unwrap_or(1);
    let left = if fits.is_none() {
        fit(&left, w.saturating_sub(host_w(1) + 2))
            .trim_end()
            .to_string()
    } else {
        left
    };
    buf.set_stringn(area.x, area.y, &left, w, left_style);
    {
        let mut x = area.x + w.saturating_sub(1 + host_w(shown)) as u16;
        for (i, (label, mark, color)) in hosts[..shown].iter().enumerate() {
            if i > 0 {
                x += 1;
            }
            buf.set_stringn(
                x,
                area.y,
                label,
                w,
                Style::default().fg(theme.text_secondary),
            );
            x += label.width() as u16 + 1;
            buf.set_stringn(x, area.y, mark, w, Style::default().fg(*color));
            x += mark.width() as u16;
        }
    }

    // Notes (schema gate), then the last row: typing, status, the committed query or the hints.
    let notes: Vec<String> = state
        .hosts
        .iter()
        .filter_map(|t| state.remote_live.get(t)?.note.clone())
        .collect();
    let (foot, foot_style) = if state.typing == Typing::Live {
        (
            Some(format!(" /{}▏", state.live_query)),
            Style::default().fg(theme.text_primary),
        )
    } else if let Some(s) = &state.status {
        (
            Some(format!(" {s}")),
            Style::default().fg(theme.accent_error),
        )
    } else if !state.live_query.is_empty() {
        (
            Some(format!(" /{}   esc clear", state.live_query)),
            Style::default().fg(theme.fuzzy_accent),
        )
    } else if state.standalone {
        (Some(HINTS.to_string()), dim)
    } else {
        (None, dim)
    };
    let foot_rows = usize::from(foot.is_some());
    let top = 1 + notes
        .len()
        .min((area.height as usize).saturating_sub(1 + foot_rows));
    for (i, note) in notes.iter().enumerate().take(top - 1) {
        buf.set_stringn(area.x, area.y + 1 + i as u16, fit(note, w), w, dim);
    }
    let body_h = (area.height as usize).saturating_sub(top + foot_rows);
    if let Some(f) = &foot
        && area.height as usize > top
    {
        buf.set_stringn(area.x, area.y + area.height - 1, fit(f, w), w, foot_style);
    }

    // Body.
    let body = Rect {
        y: area.y + top as u16,
        height: body_h as u16,
        ..area
    };
    if view.lines.is_empty() {
        for (i, r) in live_empty(state).iter().enumerate().take(body_h) {
            buf.set_stringn(body.x, body.y + i as u16, fit(r, w), w, dim);
        }
        return;
    }
    let sel = state.live_pos(&view.lines).unwrap_or(0);
    keep_visible(sel, &mut state.live_scroll, body_h);
    state.live_scroll = state
        .live_scroll
        .min(view.lines.len().saturating_sub(body_h));
    let c = live_cols(w);
    for (i, l) in view
        .lines
        .iter()
        .enumerate()
        .skip(state.live_scroll)
        .take(body_h)
    {
        let y = body.y + (i - state.live_scroll) as u16;
        let base = if i == sel {
            Style::default().bg(theme.bg_highlight)
        } else {
            Style::default()
        };
        if i == sel {
            fill(buf, body, y, base);
        }
        draw_live_line(buf, body, y, l, &c, base, theme);
    }
}

fn live_empty(state: &TracesModalState) -> Vec<String> {
    if !state.live_query.is_empty() {
        return vec![" no match".into()];
    }
    if state.index.is_none() {
        return vec![" no live agents · no index".into()];
    }
    if !state.live_folded {
        return vec![" no live agents".into()];
    }
    let n = state.live_receipts_hour;
    let noun = if n == 1 { "receipt" } else { "receipts" };
    let mut rows = vec![format!(
        " no live agents · {n} hook {noun} in the last hour · grok-traces doctor"
    )];
    if !state.live_receipts_any {
        rows.push(INSTALL_LINE.into());
    }
    rows
}

fn draw_live_line(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    l: &LiveLine,
    c: &LiveCols,
    base: Style,
    theme: &Theme,
) {
    let indent = if l.child { 3 } else { 1 };
    let title_w = if l.child {
        c.title.saturating_sub(2).max(1)
    } else {
        c.title
    };
    // Title cell: title text (cut first), ` via <x>` in gray, then the ` +N` tag, all inside `title_w`.
    let tag = match l.hidden {
        0 => String::new(),
        n => format!(" +{n}"),
    };
    let via = match l.row.via.as_deref() {
        Some(v) if !v.eq_ignore_ascii_case(&l.row.source) => format!(" via {v}"),
        _ => String::new(),
    };
    let via = if via.width() + tag.width() < title_w { via } else { String::new() };
    let head = if tag.is_empty() && via.is_empty() {
        fit(&l.title, title_w)
    } else {
        let room = title_w.saturating_sub(via.width() + tag.width());
        fit(&l.title, room).trim_end().to_string()
    };
    let used = head.width() + via.width() + tag.width();
    let (head, tag) = if used > title_w {
        (fit(&format!("{head}{tag}"), title_w), String::new())
    } else {
        (head, tag)
    };
    let pad = title_w.saturating_sub(head.width() + via.width() + tag.width());
    let text_color = if l.row.state == LiveState::Prompt || l.glyph == "◌" {
        theme.gray
    } else {
        theme.text_secondary
    };
    let mut cells: Vec<(String, Style)> = vec![
        (" ".repeat(indent), base),
        (pad_to(l.glyph, 2), base.fg(glyph_color(l.glyph, theme))),
        (" ".into(), base),
    ];
    if c.source {
        cells.push((
            fit(&source_label(&l.row.source).to_lowercase(), 7),
            base.fg(source_color(&l.row.source, theme)),
        ));
        cells.push((" ".into(), base));
    }
    cells.push((head, base.fg(theme.text_primary)));
    if !via.is_empty() {
        cells.push((via, base.fg(theme.gray)));
    }
    if !tag.is_empty() {
        cells.push((tag, base.fg(theme.text_primary)));
    }
    cells.push((" ".repeat(pad), base));
    cells.push((" ".into(), base));
    if c.ctx {
        cells.push((fit(&ctx_text(&l.row), 13), base.fg(theme.gray)));
        cells.push((" ".into(), base));
    }
    cells.push((fit(&l.text, c.fill), base.fg(text_color)));
    cells.push((" ".into(), base));
    cells.push((
        format!("{:>4}", live::fmt_age(live::age_ms(&l.row, 0))),
        base.fg(theme.text_secondary),
    ));
    if c.marks {
        cells.push((" ".into(), base));
        let color = if l.marks.contains(&"✗") {
            theme.accent_error
        } else {
            theme.gray
        };
        cells.push((cut(&l.marks.join(" "), 4), base.fg(color)));
    }
    let right = area.x + area.width;
    let mut x = area.x;
    for (text, style) in cells {
        if x >= right {
            break;
        }
        buf.set_stringn(x, y, &text, (right - x) as usize, style);
        x = x.saturating_add(text.width() as u16);
    }
}

struct HostResult {
    target: String,
    result: Result<Fetched, remote::Error>,
}

struct RemoteEvents {
    session_id: String,
    noise: bool,
    result: Result<Vec<EventRow>, remote::Error>,
}

/// A body read for a trajectory row; `session_id` lets a stale one be dropped.
struct BodyMsg {
    session_id: String,
    line: i64,
    text: Result<String, String>,
}

/// Wraps `text` to `width` columns (one entry per screen line), capped at `BODY_LINES` plus a tail line.
fn wrap_body(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out: Vec<String> = Vec::new();
    for raw in text.lines() {
        let line = raw.replace('\t', "    ");
        let body = line.trim_start();
        if body.is_empty() {
            out.push(String::new());
        } else {
            let indent = line.get(..line.len() - body.len()).unwrap_or("");
            let indent_w = cols(indent);
            // An indent that leaves no room for text is dropped for this line.
            let (indent, avail) = if indent_w < width {
                (indent, width - indent_w)
            } else {
                ("", width)
            };
            out.extend(
                modal_window::word_wrap(body, avail)
                    .into_iter()
                    .map(|row| format!("{indent}{row}")),
            );
        }
        if out.len() > BODY_LINES {
            break;
        }
    }
    if out.len() > BODY_LINES {
        out.truncate(BODY_LINES);
        out.push("… Enter for inspector".to_string());
    }
    out
}

/// Display columns of `s`, measured per char as the body wrap always has.
fn cols(s: &str) -> usize {
    s.chars()
        .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
        .sum()
}

/// Scrolls so row `sel` sits fully inside `height` lines. Row `i` is `heights[i]` lines tall, and
/// `gaps[i]` is the blank lines before (`pre`) and after (`post`) it. The gap before a row and the
/// gap after the row above it count as one line. The pre-spacer of the first drawn row is left out:
/// nothing is painted above the first line on screen, and every later pre-spacer is counted, so the
/// selected row keeps the spacer above it when the rows above still fit.
fn fit_rows(sel: usize, scroll: &mut usize, heights: &[usize], gaps: &[(u16, u16)], height: usize) {
    if sel < *scroll {
        *scroll = sel;
    }
    while *scroll < sel && row_span(*scroll, sel, heights, gaps) > height {
        *scroll += 1;
    }
}

/// Scroll for the next paint. While the selection is the row painted last time, its header keeps
/// its screen line (folds and arriving bodies change heights around it); otherwise the selection
/// is brought on screen.
fn place_scroll(state: &mut TracesModalState, heights: &[usize], gaps: &[(u16, u16)], height: usize) {
    let sel = state.sel_pos();
    let key = state.oshown.get(sel).and_then(|&n| state.key_at(n));
    let anchored = state.anchor.as_ref().filter(|(k, _)| Some(k) == key).map(|&(_, y)| y);
    match anchored {
        Some(y) => state.event_scroll = anchor_scroll(y, sel, heights, gaps, height),
        None => fit_rows(sel, &mut state.event_scroll, heights, gaps, height),
    }
}

/// The first row to draw so row `sel`'s header lands on screen line `y` (clamped into `height`), or
/// as close above it as the rows allow. A block taller than the viewport keeps its header at `y`.
fn anchor_scroll(y: usize, sel: usize, heights: &[usize], gaps: &[(u16, u16)], height: usize) -> usize {
    let y = y.min(height.saturating_sub(1));
    let (mut scroll, mut top) = (sel, 0);
    while scroll > 0 {
        let above = heights.get(scroll - 1).copied().unwrap_or(0) + gap_before(gaps, scroll);
        if top + above > y {
            break;
        }
        top += above;
        scroll -= 1;
    }
    scroll
}

/// Screen lines from shown row `from` through `to`, both inclusive.
fn row_span(from: usize, to: usize, heights: &[usize], gaps: &[(u16, u16)]) -> usize {
    if from > to {
        return 0;
    }
    let mut total = heights.get(from).copied().unwrap_or(0);
    for i in from + 1..=to {
        total += gap_before(gaps, i) + heights.get(i).copied().unwrap_or(0);
    }
    total
}

/// The one blank line before shown row `i`: its pre-spacer, or the post-spacer of the row above.
fn gap_before(gaps: &[(u16, u16)], i: usize) -> usize {
    let pre = gaps.get(i).map(|g| g.0).unwrap_or(0);
    let post = i.checked_sub(1).and_then(|p| gaps.get(p)).map(|g| g.1).unwrap_or(0);
    usize::from(pre.max(post))
}

/// Blank lines before and after each shown row. A block is a depth-0 outline row plus the visible
/// rows under it. A block taller than one line gets one spacer above and one below, except the first
/// row of the list (nothing above the first drawn line) and the single shared spacer between two tall
/// blocks. A list shorter than 8 lines gets none: the rows stay dense.
fn row_gaps(state: &TracesModalState, heights: &[usize], list_height: usize) -> Vec<(u16, u16)> {
    let n = state.oshown.len();
    let mut gaps = vec![(0u16, 0u16); n];
    if list_height < 8 || n == 0 {
        return gaps;
    }
    let depth = |i: usize| {
        state
            .oshown
            .get(i)
            .and_then(|&node| state.outline.nodes.get(node))
            .map_or(0, |node| node.depth)
    };
    let mut i = 0;
    let mut prev_tall = false;
    while i < n {
        let start = i;
        let mut lines = heights.get(i).copied().unwrap_or(1);
        i += 1;
        while i < n && depth(i) > 0 {
            lines += heights.get(i).copied().unwrap_or(1);
            i += 1;
        }
        if lines <= 1 {
            prev_tall = false;
            continue;
        }
        if start > 0 && !prev_tall {
            if let Some(gap) = gaps.get_mut(start) {
                gap.0 = 1;
            }
        }
        if let Some(gap) = gaps.get_mut(i.saturating_sub(1)) {
            gap.1 = 1;
        }
        prev_tall = true;
    }
    gaps
}

/// Output strings in one raw line (`output`/`stdout`/`stderr`/`result`, and `content`/`text` inside a
/// tool result). `None` when the line has no output yet. A non-JSON line is itself the output.
fn peek_output(raw: &str) -> Option<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
        return (!raw.trim().is_empty()).then(|| raw.trim().to_string());
    };
    fn out_key(k: &str) -> bool {
        matches!(k, "output" | "result" | "stdout" | "stderr" | "partial_output")
    }
    fn resultish(t: &str) -> bool {
        matches!(t, "tool_result" | "toolResult" | "tool-result" | "function_call_output")
    }
    fn walk(v: &serde_json::Value, res: bool, out: &mut Vec<String>) {
        match v {
            serde_json::Value::Array(xs) => xs.iter().for_each(|x| walk(x, res, out)),
            serde_json::Value::Object(m) => {
                let res = res || m.get("type").and_then(|t| t.as_str()).is_some_and(resultish) || m.get("role").and_then(|r| r.as_str()) == Some("tool_result");
                for (k, x) in m {
                    if matches!(k.as_str(), "input" | "arguments" | "args" | "params") {
                        continue;
                    }
                    if let serde_json::Value::String(s) = x && (out_key(k) || (res && matches!(k.as_str(), "content" | "text"))) {
                        push_chunk(out, s);
                    } else {
                        walk(x, res, out);
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(&v, false, &mut out);
    (!out.is_empty()).then(|| out.join("\n"))
}

fn tail_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// The human text of one raw trajectory line: message text, thinking, tool inputs and tool output, in
/// order, found by key across Claude, Codex and Pi shapes. Falls back to `pretty()` when none is found.
fn body_text(raw: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) {
        collect_text(&v, &mut out);
    }
    if out.is_empty() {
        return pretty(Ok(Some(raw.to_string())));
    }
    out.join("\n\n")
}

fn collect_text(v: &serde_json::Value, out: &mut Vec<String>) {
    use serde_json::Value;
    match v {
        Value::Array(items) => items.iter().for_each(|x| collect_text(x, out)),
        Value::Object(map) => {
            for (key, x) in map {
                let key = key.as_str();
                match x {
                    Value::String(s) if TEXT_KEYS.contains(&key) || key == "content" || key == "message" => push_chunk(out, s),
                    Value::String(s) if INPUT_KEYS.contains(&key) => push_chunk(out, s),
                    _ if INPUT_KEYS.contains(&key) => collect_text(x, out),
                    _ => collect_text(x, out),
                }
            }
        }
        _ => {}
    }
}

/// Appends a non-blank chunk, skipping it when it repeats the one before.
fn push_chunk(out: &mut Vec<String>, s: &str) {
    let t = s.trim_end();
    if t.trim().is_empty() || out.last().is_some_and(|last| last.as_str() == t) {
        return;
    }
    out.push(t.to_string());
}

/// Body text of one raw line; a line that is gone reads as a placeholder, a failed read as its error.
fn body_of(raw: Result<Option<String>, String>) -> Result<String, String> {
    match raw {
        Ok(Some(line)) if !line.is_empty() => Ok(body_text(&line)),
        Ok(_) => Ok(pretty(Ok(None))),
        Err(e) => Err(e),
    }
}

/// A body row: the event's own text, then its folded result under a separator (`✗` when it failed).
/// Any read that failed fails the whole body, so nothing half-read gets cached.
pub(super) fn compose_body(own: Result<Option<String>, String>, result: Option<(Result<Option<String>, String>, bool)>) -> Result<String, String> {
    let text = body_of(own)?;
    match result {
        Some((raw, error)) => {
            let sep = if error { RESULT_ERR_SEP } else { RESULT_SEP };
            Ok(format!("{text}\n\n{sep}\n\n{}", body_of(raw)?))
        }
        None => Ok(text),
    }
}

/// Distinct tool names in an event list, most used first.
fn tool_names(rows: &[EventRow]) -> Vec<String> {
    let mut n: HashMap<&str, usize> = HashMap::new();
    for e in rows.iter().filter(|e| e.kind == "tool") {
        *n.entry(e.title.as_str()).or_default() += 1;
    }
    let mut v: Vec<_> = n.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    v.into_iter().map(|(t, _)| t.to_string()).collect()
}

fn sort_rows(rows: &mut [SessionRow], sort: Sort) {
    match sort {
        Sort::Updated => rows.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then_with(|| a.id.cmp(&b.id))),
        Sort::Started => rows.sort_by(|a, b| b.started_at.cmp(&a.started_at).then_with(|| a.id.cmp(&b.id))),
        Sort::Events => rows.sort_by(|a, b| b.event_count.cmp(&a.event_count).then_with(|| a.id.cmp(&b.id))),
        Sort::Duration => rows.sort_by(|a, b| b.duration_ms.cmp(&a.duration_ms).then_with(|| a.id.cmp(&b.id))),
        Sort::Errors => rows.sort_by(|a, b| b.error_count.cmp(&a.error_count).then_with(|| a.id.cmp(&b.id))),
    }
}

type Ledger = (Vec<SessionRow>, HashMap<String, Vec<SessionRow>>);
type Trajectory = (Vec<EventRow>, Option<SessionRow>, Vec<SessionRow>);

/// Roots (running ones first when sorted by update) and, when browsing, each root's children.
fn query_ledger(index: &TraceIndex, filter: &SessionFilter, browse: bool) -> Result<Ledger, xai_grok_traces::Error> {
    let mut sessions = index.list_sessions(filter)?;
    let mut children: HashMap<String, Vec<SessionRow>> = HashMap::new();
    if browse {
        // Children of children too, down to MAX_DEPTH; a parent already fetched is skipped, so cycles end.
        let mut open: Vec<(String, usize)> = sessions.iter().filter(|r| r.children > 0).map(|r| (r.id.clone(), 0)).collect();
        while let Some((id, depth)) = open.pop() {
            if depth >= MAX_DEPTH || children.contains_key(&id) {
                continue;
            }
            let Ok(kids) = index.children(&id) else { continue };
            open.extend(kids.iter().filter(|k| k.children > 0).map(|k| (k.id.clone(), depth + 1)));
            children.insert(id, kids);
        }
        if filter.sort == Sort::Updated {
            // A finished parent with a running child counts as running.
            let running: HashMap<String, bool> = sessions
                .iter()
                .map(|r| (r.id.clone(), r.status == Status::Active || children.get(&r.id).is_some_and(|k| k.iter().any(|c| c.status == Status::Active))))
                .collect();
            sessions.sort_by_key(|r| !running[&r.id]);
        }
    }
    Ok((sessions, children))
}

fn query_trajectory(index: &TraceIndex, id: &str, filter: &EventFilter) -> Result<Trajectory, xai_grok_traces::Error> {
    let rows = index.list_events(id, filter)?;
    let session = index.session(id).ok().flatten();
    let mut children = match &session {
        Some(s) if s.children > 0 => index.children(&s.id).unwrap_or_default(),
        _ => Vec::new(),
    };
    children.sort_by_key(|c| c.started_at);
    Ok((rows, session, children))
}

struct Request {
    generation: u64,
    sessions: SessionFilter,
    browse: bool,
    /// The open trajectory (session id, event count we have).
    open: Option<(String, i64)>,
    events: EventFilter,
}

struct Snapshot {
    generation: u64,
    ledger: Result<Ledger, xai_grok_traces::Error>,
    trajectory: Option<Loaded>,
}

struct Loaded {
    session_id: String,
    rows: Vec<EventRow>,
    session: Option<SessionRow>,
    children: Vec<SessionRow>,
}

fn run_request(index: &TraceIndex, req: Request) -> Snapshot {
    let trajectory = req.open.and_then(|(id, count)| {
        // Only requery the trajectory when the session grew.
        let grown = index.session(&id).ok().flatten().is_some_and(|s| s.event_count != count);
        if !grown {
            return None;
        }
        let (rows, session, children) = query_trajectory(index, &id, &req.events).ok()?;
        Some(Loaded { session_id: id, rows, session, children })
    });
    Snapshot { generation: req.generation, ledger: query_ledger(index, &req.sessions, req.browse), trajectory }
}

fn is_result(e: &EventRow) -> bool {
    e.role.as_deref() == Some("tool_result")
}

fn is_call(e: &EventRow) -> bool {
    e.kind == "tool" && e.role.as_deref() == Some("tool")
}

/// An Agent/Task tool call: a subagent dispatch.
fn is_dispatch(e: &EventRow) -> bool {
    is_call(e) && matches!(e.title.to_ascii_lowercase().as_str(), "agent" | "task")
}

fn norm(s: &str) -> String {
    one_line(s).to_lowercase()
}

fn move_sel(sel: &mut usize, len: usize, delta: isize) {
    if len == 0 {
        *sel = 0;
        return;
    }
    *sel = (*sel as isize + delta).clamp(0, len as isize - 1) as usize;
}

/// Returns true when the key was consumed by the line editor.
fn edit_line(text: &mut String, key: &KeyEvent) -> bool {
    match key.code {
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            text.push(c);
            true
        }
        KeyCode::Backspace => {
            text.pop();
            true
        }
        _ => false,
    }
}

/// Viewport recorded while drawing the ledger or the trajectory, so a later click can be mapped.
#[derive(Clone, Copy)]
struct HitList {
    rect: Rect,
    mode: Mode,
}

/// A shown row under a screen cell: `index` into the visible list, `top` the row's first line
/// (the line that draws the fold glyph).
struct RowHit {
    index: usize,
    top: u16,
}

/// Shown-row index at screen row `y`. Walks `heights` and `gaps` from `scroll` the same way
/// `render_trajectory` does: the first drawn row's pre-spacer is not painted, a height of 0 draws
/// nothing, and a click on a spacer hits nothing.
fn row_at_y(y: u16, list_y: u16, bottom: u16, scroll: usize, heights: &[usize], gaps: &[(u16, u16)]) -> Option<RowHit> {
    if list_y >= bottom || y < list_y || y >= bottom {
        return None;
    }
    let mut cy = list_y;
    for k in scroll..heights.len() {
        let h = heights.get(k).copied().unwrap_or(1) as u16;
        let (pre, post) = gaps.get(k).copied().unwrap_or((0, 0));
        if cy > list_y {
            for _ in 0..pre {
                if cy >= bottom || cy == y {
                    return None;
                }
                cy += 1;
            }
        }
        if cy >= bottom {
            return None;
        }
        if h == 0 {
            for _ in 0..post {
                if cy >= bottom || cy == y {
                    return None;
                }
                cy += 1;
            }
            continue;
        }
        let span = h.min(bottom.saturating_sub(cy));
        if (cy..cy + span).contains(&y) {
            return Some(RowHit { index: k, top: cy });
        }
        cy = cy.saturating_add(h);
        for _ in 0..post {
            if cy >= bottom || cy == y {
                return None;
            }
            cy += 1;
        }
    }
    None
}

/// Column of a ledger row's fold glyph. The name cell follows the glyph, source, and (when shown) host.
fn ledger_fold_col(list_x: u16, host: bool, guides: &[bool]) -> u16 {
    let name_x = list_x + if host { 19 } else { 10 };
    name_x + guide_prefix(guides).width() as u16
}

impl TracesModalState {
    fn list_rect(&self) -> Option<Rect> {
        self.hit_list.filter(|h| h.mode == self.mode).map(|h| h.rect)
    }

    fn hit_heights(&self) -> Vec<usize> {
        if self.mode == Mode::List {
            return vec![1; self.ledger.len()];
        }
        if self.row_h.len() == self.oshown.len() { self.row_h.clone() } else { vec![1; self.oshown.len()] }
    }

    fn row_hit(&self, y: u16, list: Rect) -> Option<RowHit> {
        let heights = self.hit_heights();
        let gaps = if self.mode == Mode::Trajectory { row_gaps(self, &heights, list.height as usize) } else { Vec::new() };
        let scroll = if self.mode == Mode::List { self.scroll } else { self.event_scroll };
        row_at_y(y, list.y, list.y.saturating_add(list.height), scroll, &heights, &gaps)
    }

    /// The click is on this row's fold glyph, on the row's first line, and the glyph was drawn.
    fn on_fold(&self, column: u16, row: u16, hit: &RowHit, list: Rect) -> bool {
        if row != hit.top {
            return false;
        }
        match self.mode {
            Mode::List => {
                let Some(&n) = self.ledger.get(hit.index) else { return false };
                if !self.tree.has_children(n) {
                    return false;
                }
                let host = !self.hosts.is_empty();
                let guides = self.nodes.get(n).map(|node| node.guides.as_slice()).unwrap_or(&[]);
                let fold_x = ledger_fold_col(list.x, host, guides);
                let name_w = ledger_cols(list.width as usize, host).name as u16;
                let name_x = list.x + if host { 19 } else { 10 };
                column == fold_x && fold_x < name_x.saturating_add(name_w)
            }
            Mode::Trajectory => {
                let Some(&n) = self.oshown.get(hit.index) else { return false };
                if !self.outline.has_children(n) {
                    return false;
                }
                let depth = self.outline.nodes.get(n).map_or(0, |node| node.depth);
                let fold_x = list.x + TEXT_COL as u16 + (depth as u16) * MARK_W as u16;
                column == fold_x && fold_x < list.x.saturating_add(list.width)
            }
            Mode::Inspector | Mode::Live => false,
        }
    }

    fn select_at(&mut self, pos: usize) {
        match self.mode {
            Mode::List => self.selected = pos,
            Mode::Trajectory => self.goto(pos),
            Mode::Inspector | Mode::Live => {}
        }
    }

    fn reveal_selection(&mut self) {
        match self.mode {
            Mode::List => {
                let height = self.list_rect().map_or(0, |r| r.height as usize);
                keep_visible(self.selected, &mut self.scroll, height);
            }
            Mode::Trajectory => {
                let Some(list) = self.list_rect() else { return };
                let heights = self.hit_heights();
                let gaps = row_gaps(self, &heights, list.height as usize);
                fit_rows(self.sel_pos(), &mut self.event_scroll, &heights, &gaps, list.height as usize);
            }
            Mode::Inspector | Mode::Live => {}
        }
    }

    fn wheel(&mut self, delta: isize) -> InputOutcome {
        match self.mode {
            Mode::List => {
                let before = (self.selected, self.scroll);
                move_sel(&mut self.selected, self.ledger.len(), delta);
                self.reveal_selection();
                if before == (self.selected, self.scroll) { InputOutcome::Unchanged } else { InputOutcome::Changed }
            }
            Mode::Trajectory => {
                let before = (self.sel_pos(), self.event_scroll);
                self.step(delta);
                self.reveal_selection();
                if before == (self.sel_pos(), self.event_scroll) { InputOutcome::Unchanged } else { InputOutcome::Changed }
            }
            Mode::Inspector | Mode::Live => InputOutcome::Unchanged,
        }
    }

    fn click(&mut self, column: u16, row: u16) -> InputOutcome {
        let Some(list) = self.list_rect() else { return InputOutcome::Unchanged };
        if column < list.x || column >= list.x.saturating_add(list.width) {
            return InputOutcome::Unchanged;
        }
        let Some(hit) = self.row_hit(row, list) else { return InputOutcome::Unchanged };
        // Fold wins over Enter. A second click on the selected row is Enter (a double-click lands here too).
        if self.on_fold(column, row, &hit, list) {
            self.select_at(hit.index);
            return handle_traces_key(self, &KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        }
        let already = match self.mode {
            Mode::List => hit.index == self.selected,
            Mode::Trajectory => hit.index == self.sel_pos(),
            Mode::Inspector | Mode::Live => false,
        };
        if already {
            return handle_traces_key(self, &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        }
        self.select_at(hit.index);
        InputOutcome::Changed
    }
}

/// Esc in a sub-view or an editor is handled here; only the ledger's Esc closes the modal.
pub fn wants_esc(state: &TracesModalState) -> bool {
    state.typing != Typing::No || state.mode != Mode::List
}

/// Mouse in the list and the trajectory. A click selects; a click on the selected row (a double-click)
/// is Enter; the wheel moves by [`WHEEL`] rows; a click on a fold glyph toggles that fold.
/// The inspector ignores the mouse.
pub fn handle_traces_mouse(state: &mut TracesModalState, mouse: &MouseEvent) -> InputOutcome {
    match state.mode {
        Mode::Inspector => InputOutcome::Unchanged,
        Mode::Live => match mouse.kind {
            MouseEventKind::ScrollDown => {
                state.live_wheel(true);
                InputOutcome::Changed
            }
            MouseEventKind::ScrollUp => {
                state.live_wheel(false);
                InputOutcome::Changed
            }
            _ => InputOutcome::Unchanged,
        },
        Mode::List | Mode::Trajectory => match mouse.kind {
            MouseEventKind::ScrollDown => state.wheel(WHEEL),
            MouseEventKind::ScrollUp => state.wheel(-WHEEL),
            MouseEventKind::Down(MouseButton::Left) => state.click(mouse.column, mouse.row),
            _ => InputOutcome::Unchanged,
        },
    }
}

pub fn handle_traces_key(state: &mut TracesModalState, key: &KeyEvent) -> InputOutcome {
    let expired = state.expire_status();
    match traces_key(state, key) {
        InputOutcome::Unchanged if expired => InputOutcome::Changed,
        other => other,
    }
}

fn traces_key(state: &mut TracesModalState, key: &KeyEvent) -> InputOutcome {
    if key.kind == KeyEventKind::Release {
        return InputOutcome::Unchanged;
    }
    match state.typing {
        Typing::Ledger => {
            match key.code {
                KeyCode::Esc => {
                    state.typing = Typing::No;
                    state.query.clear();
                    state.deep = false;
                }
                KeyCode::Enter => {
                    state.typing = Typing::No;
                    state.deep = true;
                }
                _ if edit_line(&mut state.query, key) => state.deep = false,
                _ => return InputOutcome::Unchanged,
            }
            state.selected = 0;
            state.refresh();
            return InputOutcome::Changed;
        }
        Typing::Events => {
            match key.code {
                KeyCode::Esc => {
                    state.typing = Typing::No;
                    state.event_query.clear();
                }
                KeyCode::Enter => state.typing = Typing::No,
                _ if edit_line(&mut state.event_query, key) => {}
                _ => return InputOutcome::Unchanged,
            }
            state.load_events();
            return InputOutcome::Changed;
        }
        Typing::Live => {
            match key.code {
                KeyCode::Esc => {
                    state.typing = Typing::No;
                    state.live_query.clear();
                }
                KeyCode::Enter => state.typing = Typing::No,
                _ if edit_line(&mut state.live_query, key) => {}
                _ => return InputOutcome::Unchanged,
            }
            return InputOutcome::Changed;
        }
        Typing::No => {}
    }
    // The payload scrolls a page on Ctrl-U/D.
    if state.mode == Mode::Inspector && key.modifiers == KeyModifiers::CONTROL && matches!(key.code, KeyCode::Char('u' | 'd')) {
        state.raw_scroll = if key.code == KeyCode::Char('d') { state.raw_scroll + 10 } else { state.raw_scroll.saturating_sub(10) };
        return InputOutcome::Changed;
    }
    // Plain-letter nav keys ignore Ctrl/Alt chords (Ctrl+E is not `e`, Ctrl+R is not `r`).
    if matches!(key.code, KeyCode::Char(_)) && key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
        return InputOutcome::Unchanged;
    }
    let page = 10;
    match state.mode {
        Mode::List => {
            let len = state.ledger.len();
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => move_sel(&mut state.selected, len, 1),
                KeyCode::Char('k') | KeyCode::Up => move_sel(&mut state.selected, len, -1),
                KeyCode::PageDown => move_sel(&mut state.selected, len, page),
                KeyCode::PageUp => move_sel(&mut state.selected, len, -page),
                KeyCode::Char('g') | KeyCode::Home => state.selected = 0,
                KeyCode::Char('G') | KeyCode::End => move_sel(&mut state.selected, len, isize::MAX / 2),
                KeyCode::Tab | KeyCode::BackTab => {
                    let n = SOURCES.len() + 1;
                    state.source_tab =
                        if key.code == KeyCode::Tab { (state.source_tab + 1) % n } else { (state.source_tab + n - 1) % n };
                    state.selected = 0;
                    state.refresh();
                }
                KeyCode::Char('/') => state.typing = Typing::Ledger,
                KeyCode::Char('L') => state.enter_live(),
                KeyCode::Char('x') => {
                    state.errors_only = !state.errors_only;
                    state.refresh();
                }
                KeyCode::Char('a') => {
                    state.show_all = !state.show_all;
                    state.selected = 0;
                    state.refresh();
                }
                KeyCode::Char('H') if !state.hosts.is_empty() => {
                    state.host_filter = (state.host_filter + 1) % (state.hosts.len() + 2);
                    state.selected = 0;
                    state.rebuild_ledger();
                }
                KeyCode::Char('A') => {
                    state.active_only = !state.active_only;
                    state.refresh();
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    if !state.expand_selected() {
                        return InputOutcome::Unchanged;
                    }
                }
                KeyCode::Left | KeyCode::Char('h') => {
                    if !state.collapse_selected() {
                        return InputOutcome::Unchanged;
                    }
                }
                KeyCode::Char('e' | ' ') => {
                    if !state.toggle_selected() {
                        return InputOutcome::Unchanged;
                    }
                }
                KeyCode::Char('E') => {
                    if !state.fold_with(|f, t, _| f.toggle_all(t)) {
                        return InputOutcome::Unchanged;
                    }
                }
                KeyCode::Char('>') => {
                    if !state.fold_with(|f, t, n| f.expand_subtree(t, n)) {
                        return InputOutcome::Unchanged;
                    }
                }
                KeyCode::Char('<') => {
                    if !state.fold_with(|f, t, n| f.collapse_subtree(t, n)) {
                        return InputOutcome::Unchanged;
                    }
                }
                KeyCode::Char('s') => {
                    state.sort = match state.sort {
                        Sort::Updated => Sort::Started,
                        Sort::Started => Sort::Events,
                        Sort::Events => Sort::Duration,
                        Sort::Duration => Sort::Errors,
                        Sort::Errors => Sort::Updated,
                    };
                    state.refresh();
                }
                KeyCode::Char('R') => {
                    state.scanning = true;
                    return InputOutcome::Action(Action::TracesRescan);
                }
                KeyCode::Char('r') => return state.resume(state.selected_session().cloned()),
                KeyCode::Enter => {
                    if !state.expand_selected_more() {
                        state.open_selected();
                    }
                }
                _ => return InputOutcome::Unchanged,
            }
        }
        Mode::Live => {
            state.clear_status();
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => state.live_move(1),
                KeyCode::Char('k') | KeyCode::Up => state.live_move(-1),
                KeyCode::PageDown => state.live_move(page),
                KeyCode::PageUp => state.live_move(-page),
                KeyCode::Char('g') | KeyCode::Home => state.live_move(isize::MIN / 2),
                KeyCode::Char('G') | KeyCode::End => state.live_move(isize::MAX / 2),
                KeyCode::Left | KeyCode::Char('h') => return state.live_collapse(),
                KeyCode::Right | KeyCode::Char('l') => return state.live_expand(),
                KeyCode::Char('c') => return state.live_toggle(),
                KeyCode::Char('o') | KeyCode::Enter => state.live_open(),
                KeyCode::Char('/') => state.typing = Typing::Live,
                KeyCode::Char('q') => state.live_back(),
                KeyCode::Esc if !state.live_query.is_empty() => state.live_query.clear(),
                KeyCode::Esc => state.live_back(),
                _ => return InputOutcome::Unchanged,
            }
        }
        Mode::Trajectory if state.peek.is_some() => match key.code {
            KeyCode::Esc => state.close_peek(),
            KeyCode::Char('j') | KeyCode::Down => state.raw_scroll += 1,
            KeyCode::Char('k') | KeyCode::Up => state.raw_scroll = state.raw_scroll.saturating_sub(1),
            _ => return InputOutcome::Unchanged,
        },
        Mode::Trajectory => {
            let len = state.oshown.len();
            let nav = matches!(
                key.code,
                KeyCode::Char('j' | 'k' | 'g' | 'G') | KeyCode::Down | KeyCode::Up | KeyCode::PageDown | KeyCode::PageUp | KeyCode::Home | KeyCode::End
            );
            match key.code {
                KeyCode::Esc => state.leave_trajectory(),
                KeyCode::Char('j') | KeyCode::Down => state.step(1),
                KeyCode::Char('k') | KeyCode::Up => state.step(-1),
                KeyCode::PageDown => state.step(page),
                KeyCode::PageUp => state.step(-page),
                KeyCode::Char('g') | KeyCode::Home => state.goto(0),
                KeyCode::Char('G') | KeyCode::End => state.goto(len.saturating_sub(1)),
                KeyCode::Char('f') if state.session_active() => {
                    state.follow = !state.follow;
                    state.set_status(if state.follow { "follow on" } else { "follow off" });
                    if state.follow {
                        state.goto(len.saturating_sub(1));
                    }
                }
                KeyCode::Char('e' | ' ') => {
                    if !state.outline_fold(|f, t, n| f.toggle(t, n)) {
                        return InputOutcome::Unchanged;
                    }
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    // On a row that cannot expand, `l` retries any body that failed to read.
                    if !state.outline_fold(|f, t, n| f.expand(t, n)) && !state.retry_bodies() {
                        return InputOutcome::Unchanged;
                    }
                }
                KeyCode::Left | KeyCode::Char('h') => {
                    if !state.outline_collapse() {
                        return InputOutcome::Unchanged;
                    }
                }
                KeyCode::Char('E') => {
                    if !state.outline_fold(|f, t, _| f.toggle_all(t)) {
                        return InputOutcome::Unchanged;
                    }
                }
                KeyCode::Char('>') => {
                    if !state.outline_fold(|f, t, n| f.expand_subtree(t, n)) {
                        return InputOutcome::Unchanged;
                    }
                }
                KeyCode::Char('<') => {
                    if !state.outline_fold(|f, t, n| f.collapse_subtree(t, n)) {
                        return InputOutcome::Unchanged;
                    }
                }
                KeyCode::Char('x') => {
                    state.event_errors_only = !state.event_errors_only;
                    state.load_events();
                }
                KeyCode::Char('!') => state.jump_error(true),
                KeyCode::Char('*') => state.jump_error(false),
                KeyCode::Char('n') => {
                    state.include_noise = !state.include_noise;
                    state.load_events();
                }
                KeyCode::Char('t') => {
                    state.tool_filter = match state.tool_filter {
                        _ if state.tools.is_empty() => None,
                        None => Some(0),
                        Some(i) if i + 1 < state.tools.len() => Some(i + 1),
                        Some(_) => None,
                    };
                    state.load_events();
                }
                KeyCode::Char('/') => state.typing = Typing::Events,
                KeyCode::Char('r') => return state.resume(state.session.clone()),
                KeyCode::Char('y') => match state.turn_prompt_text() {
                    Some(text) => return InputOutcome::Action(Action::TracesCopy { text }),
                    None => return InputOutcome::Unchanged,
                },
                KeyCode::Char('p') => {
                    if !state.open_peek() {
                        return InputOutcome::Unchanged;
                    }
                }
                // A run's Enter expands or collapses it; on a dispatch row, Enter opens the child; otherwise the inspector.
                KeyCode::Enter if state.selected_run() => {
                    if !state.outline_fold(|f, t, n| f.toggle(t, n)) {
                        return InputOutcome::Unchanged;
                    }
                }
                KeyCode::Enter => match state.selected_event().and_then(|i| state.child_of(i)).cloned() {
                    Some(child) => state.enter_child(child),
                    None => state.open_inspector(),
                },
                _ => return InputOutcome::Unchanged,
            }
            if nav && state.follow {
                state.follow = false;
                state.set_status("follow off");
            }
        }
        Mode::Inspector => match key.code {
            KeyCode::Esc if state.payload.meta => {
                state.payload.meta = false;
                state.raw_scroll = 0;
            }
            KeyCode::Esc => {
                state.mode = Mode::Trajectory;
                state.raw = None;
            }
            KeyCode::Char('j') | KeyCode::Down => state.raw_scroll += 1,
            KeyCode::Char('k') | KeyCode::Up => state.raw_scroll = state.raw_scroll.saturating_sub(1),
            KeyCode::PageDown => state.raw_scroll += page as usize,
            KeyCode::PageUp => state.raw_scroll = state.raw_scroll.saturating_sub(page as usize),
            KeyCode::Char('g') | KeyCode::Home => state.raw_scroll = 0,
            KeyCode::Char('G') | KeyCode::End => state.raw_scroll = usize::MAX / 2,
            KeyCode::Char('m') => {
                state.payload.meta = !state.payload.meta;
                state.raw_scroll = 0;
            }
            KeyCode::Char('w') => state.payload.wrap = !state.payload.wrap,
            KeyCode::Char('r') if state.payload.failed() => state.open_inspector(),
            KeyCode::Char('y') => match state.payload.copy_text() {
                Some(text) => return InputOutcome::Action(Action::TracesCopy { text }),
                None => return InputOutcome::Unchanged,
            },
            KeyCode::Char('Y') => match state.raw_source_line() {
                Some(text) => return InputOutcome::Action(Action::TracesCopy { text }),
                None => return InputOutcome::Unchanged,
            },
            _ => return InputOutcome::Unchanged,
        },
    }
    InputOutcome::Changed
}

// ---------------------------------------------------------------- render

fn source_color(source: &str, theme: &Theme) -> Color {
    match source {
        "claude" => theme.accent_assistant,
        "codex" => theme.accent_success,
        "pi" => theme.accent_thinking,
        "omp" => theme.accent_plan,
        "dsh" => theme.accent_user,
        "grok" => theme.accent_model,
        "cursor" => theme.accent_user,
        _ => theme.gray_bright,
    }
}

pub fn fmt_duration(ms: Option<i64>) -> String {
    match ms {
        None => "-".into(),
        Some(ms) if ms < 1000 => format!("{ms}ms"),
        Some(ms) if ms < 60_000 => format!("{:.1}s", ms as f64 / 1000.0),
        Some(ms) if ms < 3_600_000 => format!("{}m{:02}s", ms / 60_000, (ms / 1000) % 60),
        Some(ms) => format!("{}h{:02}m", ms / 3_600_000, (ms / 60_000) % 60),
    }
}

pub fn fmt_tokens(n: Option<i64>) -> String {
    match n {
        None => "-".into(),
        Some(n) if n < 1000 => n.to_string(),
        Some(n) if n < 1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        Some(n) => format!("{:.1}m", n as f64 / 1e6),
    }
}

fn fmt_time(ms: Option<i64>, with_date: bool) -> String {
    let Some(dt) = ms.and_then(chrono::DateTime::from_timestamp_millis) else { return "-".into() };
    let local = dt.with_timezone(&chrono::Local);
    local.format(if with_date { "%b %d %H:%M" } else { "%H:%M:%S" }).to_string()
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Draws `cells` left to right, each truncated to its width, separated by one space.
fn put_row(buf: &mut Buffer, area: Rect, y: u16, cells: &[(String, usize, Style)]) {
    let mut x = area.x;
    let right = area.x + area.width;
    for (text, width, style) in cells {
        if x >= right {
            break;
        }
        let w = (*width).min((right - x) as usize);
        let t = truncate_str(text, w);
        buf.set_span(x, y, &Span::styled(t.as_str(), *style), w as u16);
        x = x.saturating_add(w as u16 + 1);
    }
}

fn fill(buf: &mut Buffer, area: Rect, y: u16, style: Style) {
    for x in area.x..area.x + area.width {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_style(style);
        }
    }
}

fn keep_visible(sel: usize, scroll: &mut usize, height: usize) {
    if height == 0 {
        return;
    }
    if sel < *scroll {
        *scroll = sel;
    } else if sel >= *scroll + height {
        *scroll = sel + 1 - height;
    }
}

/// Footer width the shortcut bar actually gets, matching `modal_window`'s frame math.
fn footer_hint_width(area: Rect, sizing: &ModalSizing) -> u16 {
    let max_width = area.width.saturating_sub(4).min(sizing.max_width);
    let preferred = (area.width as f32 * sizing.width_pct) as u16;
    let modal_width = preferred.min(max_width).max(sizing.min_width).min(area.width);
    modal_width.saturating_sub(2).saturating_sub(sizing.h_pad * 2)
}

/// Hints for keys that do something in `state`. `width` is the footer in columns;
/// past two lines, drop `E`, `</>`, `n`, then `x` until the bar fits.
fn shortcuts(state: &TracesModalState, width: u16) -> Vec<Shortcut<'static>> {
    let plain = |label: &'static str| Shortcut { label, clickable: false, id: 0 };
    if state.typing == Typing::Live {
        return vec![plain("type to filter"), plain("Enter keep"), plain("Esc clear")];
    }
    if state.typing != Typing::No {
        return vec![plain("type to filter"), plain("Enter search events"), plain("Esc clear")];
    }
    if state.peek.is_some() {
        return vec![plain("j/k scroll"), plain("Esc back")];
    }
    let resume = |row: Option<&SessionRow>| row.is_some_and(|row| resume_action(row).is_some());
    let mut hints = match state.mode {
        Mode::List => {
            let mut v = vec![
                plain("j/k move"),
                plain("g/G ends"),
                plain("Tab source"),
                plain("/ search"),
                plain("a all"),
                plain("A active"),
                plain("x errors"),
            ];
            if !state.hosts.is_empty() {
                v.push(plain("H host"));
            }
            v.extend([
                plain("s sort"),
                plain("Enter open"),
                plain("e toggle"),
                plain("E all"),
                plain("</> more/less"),
                plain("h/l fold"),
            ]);
            if resume(state.selected_session()) {
                v.push(plain("r resume"));
            }
            v.extend([plain("R rescan"), plain("L live"), plain("Esc close")]);
            v
        }
        Mode::Trajectory => {
            let mut v = vec![
                plain("j/k move"),
                plain("g/G ends"),
                plain("h/l fold"),
                plain("Enter inspect/open agent"),
                plain("e fold"),
                plain("E all"),
                plain("</> more/less"),
                plain("n noise"),
                plain("x errors"),
                plain("!/* errors"),
                plain("t tool"),
                plain("/ search"),
                plain("y copy"),
            ];
            if state.session_active() {
                v.insert(2, plain("f follow"));
            }
            if resume(state.session.as_ref()) {
                v.push(plain("r resume"));
            }
            v.push(plain("Esc back"));
            v
        }
        Mode::Live => vec![
            plain("j/k move"),
            plain("Enter open"),
            plain("o open"),
            plain("h/l fold"),
            plain("c fold"),
            plain("/ search"),
            plain("Esc back"),
        ],
        Mode::Inspector => {
            let mut v = vec![plain("j/k scroll"), plain("g/G ends"), plain("^U/^D page"), plain("w wrap"), plain(if state.payload.meta { "m payload" } else { "m metadata" }), plain("y copy"), plain("Y copy json")];
            if state.payload.failed() {
                v.push(plain("r retry"));
            }
            v.push(plain("Esc back"));
            v
        }
    };
    for label in ["E all", "</> more/less", "n noise", "x errors"] {
        if modal_window::shortcuts_rows_needed(&hints, width) <= 2 {
            break;
        }
        hints.retain(|hint| hint.label != label);
    }
    hints
}

pub fn render_traces_modal(buf: &mut Buffer, full_area: Rect, state: &mut TracesModalState, compact: bool) {
    state.hit_list = None;
    let theme = Theme::current();
    let sizing = ModalSizing { width_pct: 1.0, max_width: u16::MAX, min_width: 44, v_margin: 0, h_pad: 2, v_pad: 0, footer_lines: 2 }
        .with_compact(compact);
    let shortcuts = shortcuts(state, footer_hint_width(full_area, &sizing));
    let config = ModalWindowConfig {
        title: TITLE,
        tabs: None,
        shortcuts: &shortcuts,
        sizing,
        fold_info: None,
    };
    let Some(ModalContentArea { content, .. }) = modal_window::render_modal_window(buf, full_area, &mut state.window, &config, &theme)
    else {
        return;
    };
    if content.height < 4 || content.width < 20 {
        return;
    }
    match state.mode {
        Mode::List => render_ledger(buf, content, state, &theme),
        Mode::Trajectory => render_trajectory(buf, content, state, &theme),
        Mode::Inspector => render_inspector(buf, content, state, &theme),
        Mode::Live => render_live(buf, content, state, &theme),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// "now", "42s", "3m", "2h", "5d".
pub fn fmt_age(ago_ms: i64) -> String {
    let s = ago_ms.max(0) / 1000;
    match s {
        0..10 => "now".into(),
        10..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        3600..86_400 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86_400),
    }
}

fn age_of(s: &SessionRow, now: i64) -> String {
    s.last_event_at.or(s.updated_at).map_or("-".into(), |t| fmt_age(now - t))
}

/// Dispatch description (else title) without leading markup or a bare file name.
pub fn clean_name(s: &SessionRow) -> String {
    let raw = s.label.as_deref().filter(|l| !l.trim().is_empty()).unwrap_or(&s.title);
    let mut t = one_line(raw);
    loop {
        let before = t.len();
        t = t.trim_start_matches(|c: char| matches!(c, '#' | '*' | '>' | '"' | '\'' | '`') || c.is_whitespace()).to_string();
        // A wrapper tag such as `<command-message>`: drop it and its closing twin.
        if t.starts_with('<')
            && let Some(end) = t.find('>')
        {
            let tag = t[1..end].to_string();
            if !tag.is_empty() && !tag.starts_with('/') && tag.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
                t = t[end + 1..].replacen(&format!("</{tag}>"), " ", 1);
            }
        }
        if t.len() == before {
            break;
        }
    }
    t = t.trim().to_string();
    if t.is_empty() {
        t = one_line(&s.title);
    }
    if !t.contains(' ') && (t.ends_with(".jsonl") || t.ends_with(".json")) {
        let dir = s.cwd.as_deref().and_then(|c| c.trim_end_matches('/').rsplit('/').next()).filter(|d| !d.is_empty());
        t = format!("{} session", dir.unwrap_or(source_label(&s.source)));
    }
    t
}

/// ● running, ✗ failed, ✓ done, ○ idle.
fn status_symbol(s: &SessionRow) -> &'static str {
    match s.status {
        Status::Active => "●",
        _ if s.failed => "✗",
        Status::Completed => "✓",
        _ => "○",
    }
}

fn status_glyph(s: &SessionRow, theme: &Theme) -> (&'static str, Color) {
    let color = match s.status {
        Status::Active => theme.accent_running,
        _ if s.failed => theme.accent_error,
        Status::Completed => theme.gray,
        _ => theme.gray_dim,
    };
    (status_symbol(s), color)
}

struct Cols {
    name: usize,
    now: usize,
    counts: bool,
    cwd: bool,
    tokens: bool,
}

/// Fixed columns by priority; NAME and NOW share what is left.
fn ledger_cols(width: usize, host: bool) -> Cols {
    let (counts, cwd, tokens) = (width >= 100, width >= 130, width >= 160);
    // glyph, source, age, tools (+1 separator each), plus the NAME and NOW separators.
    let mut fixed = (1 + 1) + (7 + 1) + (4 + 1) + (5 + 1) + 2 + if host { 9 } else { 0 };
    if counts {
        fixed += (5 + 1) + (4 + 1) + (6 + 1);
    }
    if cwd {
        fixed += 22 + 1;
    }
    if tokens {
        fixed += 12 + 1;
    }
    let flex = width.saturating_sub(fixed);
    let name = (flex * 2 / 5).max(12).min(flex);
    Cols { name, now: flex - name, counts, cwd, tokens }
}

fn render_ledger(buf: &mut Buffer, area: Rect, state: &mut TracesModalState, theme: &Theme) {
    let dim = Style::default().fg(theme.gray);
    // Tabs.
    let mut x = area.x;
    for tab in 0..=SOURCES.len() {
        let label = tab_source(tab).map_or("All", source_label);
        let style = if tab == state.source_tab {
            Style::default().fg(theme.bg_base).bg(tab_source(tab).map_or(theme.accent_user, |s| source_color(s, theme))).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_secondary)
        };
        let text = format!(" {label} ");
        if x + text.width() as u16 > area.x + area.width {
            break;
        }
        buf.set_span(x, area.y, &Span::styled(text.as_str(), style), text.width() as u16);
        x += text.width() as u16 + 1;
    }
    let mut flags = Vec::new();
    if state.live {
        flags.push("● live".to_string());
    }
    if state.scanning {
        flags.push("indexing…".to_string());
    }
    if state.errors_only {
        flags.push("errors".into());
    }
    if state.active_only {
        flags.push("active only".into());
    }
    if state.show_all {
        flags.push("all".into());
    }
    match state.host_filter {
        0 => {}
        1 => flags.push("local only".into()),
        n => flags.push(format!("host:{}", state.hosts.get(n - 2).map_or("?", |t| remote::label(t)))),
    }
    if state.sort != Sort::Updated {
        flags.push(format!("sort:{:?}", state.sort).to_lowercase());
    }
    let (active, failed, total) = state.counts();
    let mut counts = Vec::new();
    if active > 0 {
        counts.push(format!("{active} active"));
    }
    if failed > 0 {
        counts.push(format!("{failed} failed"));
    }
    counts.push(format!("{total} sessions"));
    flags.push(counts.join(" · "));
    let right = flags.join(" · ");
    let rw = right.width() as u16;
    if area.width > rw && x < area.x + area.width - rw {
        buf.set_span(area.x + area.width - rw, area.y, &Span::styled(right.as_str(), dim), rw);
    }

    // Search line.
    let search_y = area.y + 1;
    let (text, style) = match (state.typing == Typing::Ledger, state.query.is_empty()) {
        (true, _) => (format!("/{}▏", state.query), Style::default().fg(theme.text_primary)),
        (false, true) => ("/ to search titles, cwd, model · Enter searches events too".into(), dim),
        (false, false) => (format!("/{}{}", state.query, if state.deep { "  (events)" } else { "" }), Style::default().fg(theme.fuzzy_accent)),
    };
    // A red health line takes the right of the search row; the search hint yields to it.
    let remote_alert = (!state.remote_err.is_empty()).then(|| {
        let mut v: Vec<&String> = state.remote_err.values().collect();
        v.sort();
        v.into_iter().cloned().collect::<Vec<_>>().join(" · ")
    });
    let alert = state.health.clone().or(remote_alert);
    let health_w = alert.as_deref().map_or(0, |h| h.width().min(area.width as usize * 2 / 3)) as u16;
    if let Some(health) = &alert {
        let red = Style::default().fg(theme.accent_error).add_modifier(Modifier::BOLD);
        buf.set_span(area.x + area.width - health_w, search_y, &Span::styled(truncate_str(health, health_w as usize).as_str(), red), health_w);
    }
    let search_w = area.width.saturating_sub(health_w + u16::from(health_w > 0));
    buf.set_span(area.x, search_y, &Span::styled(truncate_str(&text, search_w as usize).as_str(), style), search_w);

    let status_rows = u16::from(state.status.is_some());
    let list = Rect { y: area.y + 3, height: area.height.saturating_sub(3 + status_rows), ..area };
    state.hit_list = Some(HitList { rect: list, mode: Mode::List });
    let show_host = !state.hosts.is_empty();
    let c = ledger_cols(area.width as usize, show_host);
    let mut head: Vec<(String, usize, Style)> = vec![
        (" ".into(), 1, dim),
        ("SOURCE".into(), 7, dim),
        ("NAME".into(), c.name, dim),
        ("NOW".into(), c.now, dim),
        ("AGE".into(), 4, dim),
        ("TOOLS".into(), 5, dim),
    ];
    if show_host {
        head.insert(2, ("HOST".into(), 8, dim));
    }
    if c.counts {
        head.extend([("  EVT".into(), 5, dim), (" ERR".into(), 4, dim), ("   DUR".into(), 6, dim)]);
    }
    if c.cwd {
        head.push(("CWD".into(), 22, dim));
    }
    if c.tokens {
        head.push(("   IN/OUT".into(), 12, dim));
    }
    put_row(buf, area, area.y + 2, &head);
    if state.ledger.is_empty() {
        let msg = if state.scanning { "Indexing local sessions…" } else { "No sessions match. (a shows every session)" };
        buf.set_span(list.x, list.y, &Span::styled(msg, dim), list.width);
    }
    keep_visible(state.selected, &mut state.scroll, list.height as usize);
    let now = now_ms();
    for (i, &n) in state.ledger.iter().enumerate().skip(state.scroll).take(list.height as usize) {
        let y = list.y + (i - state.scroll) as u16;
        let selected = i == state.selected;
        let base = if selected { Style::default().bg(theme.bg_highlight).fg(theme.text_primary) } else { Style::default().fg(theme.text_primary) };
        if selected {
            fill(buf, list, y, base);
        }
        let Some(node) = state.nodes.get(n) else { continue };
        match &node.item {
            Item::More { hidden, .. } => put_row(buf, list, y, &{
                let mut v = vec![(" ".into(), 1, base), ("".into(), 7, base)];
                if show_host {
                    v.push(("".into(), 8, base));
                }
                v.push((format!("{}  +{hidden} more (l)", guide_prefix(&node.guides)), c.name, base.fg(theme.gray)));
                v
            }),
            Item::Row { row: s } => {
                let (glyph, glyph_color) = status_glyph(s, theme);
                let foldable = state.tree.has_children(n);
                let collapsed = foldable && state.tree.nodes.get(n).is_some_and(|x| state.folds.is_collapsed(&x.key));
                let fold = match (foldable, collapsed) {
                    (false, _) => "  ".to_string(),
                    (true, true) => format!("{} ", crate::glyphs::chevron()),
                    (true, false) => format!("{} ", crate::glyphs::chevron_down()),
                };
                let mut name = format!("{}{fold}{}", guide_prefix(&node.guides), clean_name(s));
                if collapsed {
                    // The hidden count stays visible; the name gives way to it.
                    let tag = format!(" (+{})", node.hidden);
                    name = format!("{}{tag}", truncate_str(&name, c.name.saturating_sub(tag.width())).as_str());
                }
                let running = s.status == Status::Active;
                let now_text = one_line(s.now.as_deref().unwrap_or_default());
                let mut v = vec![
                    (glyph.to_string(), 1, base.fg(glyph_color)),
                    (source_label(&s.source).to_string(), 7, base.fg(source_color(&s.source, theme))),
                    (name, c.name, base),
                    (now_text, c.now, base.fg(if running { theme.text_secondary } else { theme.gray })),
                    (format!("{:>4}", age_of(s, now)), 4, base.fg(theme.text_secondary)),
                    (format!("{:>5}", s.tool_count), 5, base.fg(theme.accent_tool)),
                ];
                if show_host {
                    v.insert(2, (s.host.clone().unwrap_or_default(), 8, base.fg(theme.accent_user)));
                }
                if c.counts {
                    v.push((format!("{:>5}", s.event_count), 5, base.fg(theme.text_secondary)));
                    v.push((if s.error_count > 0 { format!("{:>4}", s.error_count) } else { String::new() }, 4, base.fg(theme.accent_error)));
                    v.push((format!("{:>6}", fmt_duration(s.duration_ms)), 6, base.fg(theme.text_secondary)));
                }
                if c.cwd {
                    v.push((s.cwd.clone().or_else(|| s.model.clone()).unwrap_or_default(), 22, base.fg(theme.path)));
                }
                if c.tokens {
                    v.push((format!("{:>5}/{:<6}", fmt_tokens(s.input_tokens), fmt_tokens(s.output_tokens)), 12, base.fg(theme.gray_bright)));
                }
                put_row(buf, list, y, &v);
            }
        }
    }
    render_status(buf, area, state, theme);
}

fn render_status(buf: &mut Buffer, area: Rect, state: &TracesModalState, theme: &Theme) {
    if let Some(status) = &state.status {
        let y = area.y + area.height - 1;
        buf.set_span(
            area.x,
            y,
            &Span::styled(truncate_str(status, area.width as usize).as_str(), Style::default().fg(theme.accent_user).add_modifier(Modifier::BOLD)),
            area.width,
        );
    }
}

fn role_chip(e: &EventRow) -> &'static str {
    match (e.kind.as_str(), e.role.as_deref()) {
        (_, Some("tool_result")) => "RESULT",
        ("tool", _) => "TOOL",
        (_, Some("user")) => "USER",
        (_, Some("assistant")) => "ASSIST",
        ("reasoning", _) => "THINK",
        ("system", _) => "SYSTEM",
        ("step", _) => "STEP",
        ("turn", _) => "TURN",
        ("telemetry", _) => "TOKENS",
        _ => "EVENT",
    }
}

fn role_glyph(e: &EventRow) -> &'static str {
    match (e.is_error, e.kind.as_str(), e.role.as_deref()) {
        (true, _, _) => "✗",
        (_, _, Some("tool_result")) => "✓",
        (_, "tool", _) => "▸",
        (_, _, Some("assistant")) => "◆",
        (_, _, Some("user")) => "›",
        (_, "reasoning", _) => "?",
        (_, "telemetry", _) => "$",
        (_, "system", _) => "*",
        _ => "·",
    }
}

fn fmt_token_pair(e: &EventRow) -> String {
    format!("{:>5}/{:<6}", fmt_tokens(e.input_tokens), fmt_tokens(e.output_tokens))
}

fn echoed(column: &str, row_text: &str) -> bool {
    let c = column.trim();
    c.chars().any(|ch| ch != '-' && ch != '/') && row_text.to_ascii_lowercase().contains(&c.to_ascii_lowercase())
}

fn text_has_title(title: &str, text: &str) -> bool {
    let title = title.trim();
    let text = text.trim_start();
    text.get(..title.len()).is_some_and(|h| !title.is_empty() && h.eq_ignore_ascii_case(title))
        && text.get(title.len()..).is_some_and(|r| r.chars().next().is_none_or(|c| c.is_whitespace()))
}

fn event_cols(width: usize, e: &EventRow, row_text: &str) -> (bool, bool, bool, usize) {
    let chip = width >= 100 && !echoed(role_chip(e), row_text);
    let age = width >= 120 && !echoed(&fmt_time(e.timestamp, false), row_text);
    let has_tokens = e.input_tokens.is_some() || e.output_tokens.is_some();
    let tokens = width >= 120 && has_tokens && !echoed(&fmt_token_pair(e), row_text);
    let used = 9 + usize::from(chip) * 7 + usize::from(age) * 9 + usize::from(tokens) * 13;
    (chip, age, tokens, width.saturating_sub(used))
}

fn chip_color(e: &EventRow, theme: &Theme) -> Color {
    if e.is_error {
        theme.accent_error
    } else if e.kind == "tool" {
        theme.accent_tool
    } else if e.role.as_deref() == Some("user") || e.kind == "system" || e.kind == "reasoning" {
        theme.accent_user
    } else {
        theme.accent_assistant
    }
}

const SHADES: [char; 5] = [' ', '░', '▒', '▓', '█'];

/// Bucket of event `i` out of `n` given the time span `(lo, hi)`: by time when it has one, else by position.
fn bucket_of(events: &[EventRow], i: usize, n: usize, span: (Option<i64>, Option<i64>)) -> usize {
    let frac = match (events[i].timestamp, span) {
        (Some(t), (Some(a), Some(b))) if b > a => (t - a) as f64 / (b - a) as f64,
        _ => i as f64 / (events.len().max(2) - 1) as f64,
    };
    ((frac * (n - 1) as f64).round() as usize).min(n - 1)
}

fn time_span(events: &[EventRow]) -> (Option<i64>, Option<i64>) {
    let stamps = events.iter().filter_map(|e| e.timestamp);
    (stamps.clone().min(), stamps.max())
}

/// Per-bucket (event count, has error, turn starts here) over `n` buckets.
fn bucket_events(events: &[EventRow], n: usize) -> Vec<(u32, bool, bool)> {
    let mut out = vec![(0u32, false, false); n];
    if n == 0 {
        return out;
    }
    let span = time_span(events);
    let mut prev_turn = None;
    for (i, e) in events.iter().enumerate() {
        let b = &mut out[bucket_of(events, i, n, span)];
        b.0 += 1;
        b.1 |= e.is_error;
        if e.turn.is_some() && e.turn != prev_turn {
            b.2 = true;
        }
        prev_turn = e.turn.or(prev_turn);
    }
    out
}

/// Two lines: shaded activity per bucket (red where something failed), then turn starts `│` and the cursor `▲`.
fn render_strip(buf: &mut Buffer, area: Rect, events: &[EventRow], selected: usize, theme: &Theme) {
    let gutter = 9u16;
    let n = area.width.saturating_sub(gutter) as usize;
    buf.set_span(area.x, area.y, &Span::styled("ACTIVITY", Style::default().fg(theme.gray)), gutter);
    if n == 0 || events.is_empty() {
        return;
    }
    let buckets = bucket_events(events, n);
    let max = buckets.iter().map(|b| b.0).max().unwrap_or(0).max(1) as usize;
    for (i, (count, err, turn)) in buckets.iter().enumerate() {
        let x = area.x + gutter + i as u16;
        if *count > 0 {
            let level = (*count as usize * 4).div_ceil(max).clamp(1, 4);
            let fg = if *err { theme.accent_error } else { theme.accent_assistant };
            buf.set_span(x, area.y, &Span::styled(SHADES[level].to_string().as_str(), Style::default().fg(fg)), 1);
        }
        if *turn {
            buf.set_span(x, area.y + 1, &Span::styled("│", Style::default().fg(theme.gray_dim)), 1);
        }
    }
    if selected < events.len() {
        let at = area.x + gutter + bucket_of(events, selected, n, time_span(events)) as u16;
        buf.set_span(at, area.y + 1, &Span::styled("▲", Style::default().fg(theme.text_primary).add_modifier(Modifier::BOLD)), 1);
    }
}

/// Rows in the inspector session header: chip, meta, activity strip, cwd or filter.
const FULL_HEADER_ROWS: u16 = 5;

fn ctx_pct(session: &SessionRow) -> String {
    match (session.context_tokens, session.context_window) {
        (Some(used), Some(window)) if window > 0 => format!("ctx {}%", (used.max(0) as i128 * 100 / window as i128).clamp(0, 100)),
        _ => "ctx -".into(),
    }
}

/// One muted row: `source · title · N events · K tools · ctx%`. The title clips so the counts stay.
fn render_compact_header(buf: &mut Buffer, area: Rect, state: &TracesModalState, theme: &Theme) {
    let Some(session) = state.session.clone() else { return };
    if area.width == 0 { return; }
    let mut tail = format!(" · {} · {} · {}", count_of(session.event_count.max(0) as usize, "event", "events"), count_of(session.tool_count.max(0) as usize, "tool", "tools"), ctx_pct(&session));
    if state.follow { tail.push_str(" · FOL"); }
    if let Some(p) = &state.parent_name { tail.push_str(&format!(" · ↑ {p}")); }
    if state.events.len() >= EVENT_PAGE && session.event_count > state.events.len() as i64 && !state.filtering() && !(session.host.is_some() && state.remote_loading) {
        tail.push_str(&format!(" · showing {}/{}", state.events.len(), session.event_count));
    }
    let mut marks: Vec<String> = Vec::new();
    if state.event_errors_only { marks.push("errors only".into()); }
    if state.include_noise { marks.push("noise shown".into()); }
    if let Some(t) = state.tool_filter.and_then(|i| state.tools.get(i)) { marks.push(format!("tool:{t}")); }
    match state.typing {
        Typing::Events => marks.push(format!("/{}▏", state.event_query)),
        _ if !state.event_query.is_empty() => marks.push(format!("/{}", state.event_query)),
        _ => {}
    }
    if !marks.is_empty() {
        tail.push_str(&format!(" · {} · {} matching", marks.join(" · "), state.hidden.iter().filter(|h| !**h).count()));
    }
    let width = area.width as usize;
    let source = source_label(&session.source);
    let prefix = format!("{source} · ");
    let name = clean_name(&session);
    let budget = width.saturating_sub(prefix.width() + tail.width());
    let name = if budget == 0 || name.is_empty() { String::new() } else if name.width() > budget { ellipsize(&name, budget).trim_end().to_string() } else { name };
    let line = if name.is_empty() { format!("{source}{tail}") } else { format!("{prefix}{name}{tail}") };
    buf.set_span(area.x, area.y, &Span::styled(truncate_str(&line, width).as_str(), Style::default().fg(theme.gray)), area.width);
}

fn render_trajectory(buf: &mut Buffer, area: Rect, state: &mut TracesModalState, theme: &Theme) {
    if state.peek.is_some() {
        render_inspector(buf, area, state, theme);
        return;
    }
    let Some(session) = state.session.clone() else { return };
    let dim = Style::default().fg(theme.gray);
    // Inspector keeps the block below. Trajectory draws one muted row and skips it.
    if state.mode != Mode::Inspector {
        render_compact_header(buf, area, state, theme);
    } else {
        let chip = format!(" {} ", source_label(&session.source));
        let cw = chip.width() as u16;
        buf.set_span(area.x, area.y, &Span::styled(chip.as_str(), Style::default().fg(theme.bg_base).bg(source_color(&session.source, theme)).add_modifier(Modifier::BOLD)), cw);
        let title_w = area.width.saturating_sub(cw + 1) as usize;
        buf.set_span(area.x + cw + 1, area.y, &Span::styled(truncate_str(&clean_name(&session), title_w).as_str(), Style::default().fg(theme.text_primary).add_modifier(Modifier::BOLD)), title_w as u16);

        // Meta line: status, agent type / model, age, counts, parent.
        let (glyph, glyph_color) = status_glyph(&session, theme);
        let mut segs: Vec<(String, Style)> = vec![(format!("{glyph} {}", if session.failed && session.status != Status::Active { "failed" } else { session.status.as_str() }), Style::default().fg(glyph_color))];
        for t in [session.agent_type.clone(), session.model.clone()].into_iter().flatten() {
            segs.push((t, dim));
        }
        let age = age_of(&session, now_ms());
        segs.push((if age == "now" || age == "-" { age } else { format!("{age} ago") }, dim));
        segs.push((format!("{} evt · {} tools · {}", session.event_count, session.tool_count, fmt_duration(session.duration_ms)), dim));
        if let Some(p) = &state.parent_name {
            segs.push((format!("↑ {p}"), Style::default().fg(theme.accent_user)));
        }
        // A full page with more events on the session is a silent cap. A filter hides rows on purpose,
        // and a remote fetch still in flight has not settled on this page.
        if state.events.len() >= EVENT_PAGE
            && session.event_count > state.events.len() as i64
            && !state.filtering()
            && !(session.host.is_some() && state.remote_loading)
        {
            segs.push((format!("showing {}/{}", state.events.len(), session.event_count), Style::default().fg(theme.accent_user)));
        }
        let mut x = area.x;
        let right = area.x + area.width;
        for (i, (text, style)) in segs.iter().enumerate() {
            if i > 0 {
                if x + 3 >= right {
                    break;
                }
                buf.set_span(x, area.y + 1, &Span::styled(" · ", dim), 3);
                x += 3;
            }
            let w = (text.width() as u16).min(right - x);
            buf.set_span(x, area.y + 1, &Span::styled(truncate_str(text, w as usize).as_str(), *style), w);
            x += w;
        }
        render_strip(buf, Rect { y: area.y + 2, height: 2, ..area }, &state.events, state.selected_event().unwrap_or(0), theme);

        let mut filters = Vec::new();
        if state.event_errors_only {
            filters.push("errors only".to_string());
        }
        if state.include_noise {
            filters.push("noise shown".to_string());
        }
        if let Some(t) = state.tool_filter.and_then(|i| state.tools.get(i)) {
            filters.push(format!("tool:{t}"));
        }
        let search = match state.typing {
            Typing::Events => format!("/{}▏", state.event_query),
            _ if !state.event_query.is_empty() => format!("/{}", state.event_query),
            _ => String::new(),
        };
        if !search.is_empty() {
            filters.push(search);
        }
        let line = if filters.is_empty() {
            session.cwd.clone().unwrap_or_else(|| session.file_path.display().to_string())
        } else {
            format!("{} · {} matching", filters.join(" · "), state.hidden.iter().filter(|h| !**h).count())
        };
        buf.set_span(area.x, area.y + 4, &Span::styled(truncate_str(&line, area.width as usize).as_str(), Style::default().fg(theme.fuzzy_accent)), area.width);
        return;
    }

    let status_rows = u16::from(state.status.is_some());
    let list = Rect { y: area.y + 1, height: area.height.saturating_sub(1 + status_rows), ..area };
    state.hit_list = Some(HitList { rect: list, mode: Mode::Trajectory });
    // Outline rows are built for this text width. A resize, or an opener body that changed its prompt,
    // rebuilds them here, before anything is measured.
    let text_w = (list.width as usize).saturating_sub(TEXT_COL);
    if text_w != state.outline_w || state.dirty_outline {
        state.outline_w = text_w;
        state.dirty_outline = false;
        state.rebuild_outline();
    }
    if state.oshown.is_empty() {
        let msg = if state.remote_loading { "Fetching events over ssh…" } else { "No events match." };
        buf.set_span(list.x, list.y, &Span::styled(msg, dim), list.width);
    }
    // Body rows are as tall as their wrapped text, so scrolling works in lines, not rows.
    // An expanded body with nothing past the header is height 0: not drawn, not selectable.
    // `gaps` is render-only: spacers never enter the outline, so selection still steps by row.
    let height = list.height as usize;
    let mut bodies = state.body_rows(text_w);
    let mut heights = row_heights(&bodies);
    state.note_heights(&heights);
    let mut gaps = row_gaps(state, &heights, height);
    place_scroll(state, &heights, &gaps, height);
    // Load the bodies on screen; one cached right here changes row heights, so measure again.
    if state.ensure_bodies(state.event_scroll, height) {
        if state.dirty_outline {
            state.dirty_outline = false;
            state.rebuild_outline();
        }
        bodies = state.body_rows(text_w);
        heights = row_heights(&bodies);
        state.note_heights(&heights);
        gaps = row_gaps(state, &heights, height);
        place_scroll(state, &heights, &gaps, height);
    }

    let bottom = list.y + list.height;
    state.anchor = None;
    let sel = state.sel_pos();
    let mut y = list.y;
    for (k, &n) in state.oshown.iter().enumerate().skip(state.event_scroll) {
        let h = heights.get(k).copied().unwrap_or(1) as u16;
        let (pre, post) = gaps.get(k).copied().unwrap_or((0, 0));
        // A spacer above the first drawn line would be a blank at the top of the list.
        if y > list.y {
            skip_lines(&mut y, pre, bottom);
        }
        if y >= bottom {
            break;
        }
        // Nothing to draw. Keep a spacer that belongs to this row (a block's trailing gap sits on
        // its last row, which may itself be empty).
        if h == 0 {
            skip_lines(&mut y, post, bottom);
            continue;
        }
        let span = h.min(bottom - y);
        let selected = k == sel;
        if selected {
            state.anchor = state.outline.nodes.get(n).map(|x| (x.key.clone(), usize::from(y - list.y)));
        }
        let base = if selected { Style::default().bg(theme.bg_highlight).fg(theme.text_primary) } else { Style::default().fg(theme.text_primary) };
        if selected {
            for dy in 0..span {
                fill(buf, list, y + dy, base);
            }
        }
        let depth = state.outline.nodes.get(n).map_or(0, |x| x.depth);
        let indent = "  ".repeat(depth);
        let mark = state.fold_mark(n);
        match state.outline_items.get(n) {
            Some(OutlineItem::Turn {
                event,
                tools,
                msgs,
                errors,
                ..
            }) => {
                let Some(e) = state.events.get(*event) else {
                    y += h;
                    skip_lines(&mut y, post, bottom);
                    continue;
                };
                // The header is the prompt's first line; the counts follow it, in every state.
                let (head, suffix) = turn_head(
                    &one_line(index_text(e)),
                    text_w,
                    &turn_counts(*tools, *msgs, *errors),
                );
                let text = format!("{indent}{mark}{head}{suffix}");
                put_row(
                    buf,
                    list,
                    y,
                    &[
                        (
                            format!("{:>6}", format!("#{}", e.line)),
                            6,
                            base.fg(theme.gray),
                        ),
                        (
                            role_chip(e).to_string(),
                            6,
                            base.fg(chip_color(e, theme)).add_modifier(Modifier::BOLD),
                        ),
                        (text, text_w, base.add_modifier(Modifier::BOLD)),
                    ],
                );
            }
            Some(OutlineItem::Run { count, errors, .. }) => {
                let mut text = format!("{indent}{mark}{}", count_of(*count, "tool call", "tool calls"));
                if *errors > 0 {
                    text.push_str(&format!("  · ✗{errors}"));
                }
                put_row(buf, list, y, &[(String::new(), 6, base), (String::new(), 6, base), (text, text_w, base.fg(theme.accent_tool))]);
            }
            Some(OutlineItem::Event(i)) => {
                let Some(e) = state.events.get(*i) else {
                    y += h;
                    skip_lines(&mut y, post, bottom);
                    continue;
                };
                // Noise (only present when `n` is on) is dimmed.
                let (text_style, chip_fg) = if e.noise { (base.fg(theme.gray_dim), theme.gray_dim) } else { (base, chip_color(e, theme)) };
                let eh = state.event_head(*i, depth, list.width as usize);
                let body_style = if is_result(e) && e.is_error { text_style.fg(theme.accent_error) } else { text_style };
                let mut cells = vec![
                    (format!("{:>6}", format!("#{}", e.line)), 6, base.fg(theme.gray)),
                    (role_glyph(e).to_string(), 1, base.fg(chip_fg).add_modifier(Modifier::BOLD)),
                ];
                if eh.chip {
                    cells.push((role_chip(e).to_string(), 6, base.fg(chip_fg).add_modifier(Modifier::BOLD)));
                }
                cells.push((format!("{indent}{mark}{}{}", eh.title, eh.head), eh.body_w, body_style));
                if eh.suffix_w > 0 {
                    cells.push((eh.suffix, eh.suffix_w, base.fg(if eh.failed { theme.accent_error } else { theme.accent_success })));
                }
                if eh.age {
                    cells.push((fmt_time(e.timestamp, false), 8, base.fg(theme.text_secondary)));
                }
                if eh.tokens {
                    cells.push((fmt_token_pair(e), 12, base.fg(theme.gray_bright)));
                }
                put_row(buf, list, y, &cells);
            }
            Some(OutlineItem::Body(_)) => {
                let indent_w = (2 * depth).min(text_w);
                // A failed call's result section (after its separator) is drawn in the error colour.
                let failed = matches!(state.outline_items.get(n), Some(OutlineItem::Body(i)) if state.folded.get(*i).and_then(Option::as_ref).is_some_and(|f| f.error));
                let mut in_result = false;
                for (dy, line) in bodies
                    .get(k)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .take(span as usize)
                    .enumerate()
                {
                    in_result |= line.as_str() == RESULT_SEP || line.as_str() == RESULT_ERR_SEP;
                    let fg = if in_result && failed { theme.accent_error } else { theme.text_secondary };
                    let text = format!("{}{line}", " ".repeat(indent_w));
                    put_row(buf, list, y + dy as u16, &[(String::new(), 6, base), (String::new(), 6, base), (text, text_w, base.fg(fg))]);
                }
            }
            Some(OutlineItem::Prompt(_) | OutlineItem::PromptRest(_)) => {
                // Continuation rows: under the turn's header text, in its style.
                let pad = " ".repeat(MARK_W);
                for (dy, line) in bodies
                    .get(k)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .take(span as usize)
                    .enumerate()
                {
                    let text = format!("{pad}{line}");
                    put_row(
                        buf,
                        list,
                        y + dy as u16,
                        &[
                            (String::new(), 6, base),
                            (String::new(), 6, base),
                            (text, text_w, base.add_modifier(Modifier::BOLD)),
                        ],
                    );
                }
            }
            Some(OutlineItem::PromptMore(_)) => {
                let total = state.prompt_rows_at(n, text_w).len();
                let text = format!("{mark}{}", more_label(total));
                put_row(
                    buf,
                    list,
                    y,
                    &[
                        (String::new(), 6, base),
                        (String::new(), 6, base),
                        (text, text_w, base.fg(theme.gray)),
                    ],
                );
            }
            None => {}
        }
        y += h;
        skip_lines(&mut y, post, bottom);
    }
    render_status(buf, area, state, theme);
}

/// Moves `y` down `n` lines, stopping at `bottom`. The cells stay untouched, so a spacer is blank
/// and carries neither the selection fill nor a result colour.
fn skip_lines(y: &mut u16, n: u16, bottom: u16) {
    for _ in 0..n {
        if *y >= bottom {
            break;
        }
        *y += 1;
    }
}

/// Lines per shown row: a body is as tall as its wrapped text (`Some([])` is 0), every other row is one line.
fn row_heights(bodies: &[Option<Vec<String>>]) -> Vec<usize> {
    bodies.iter().map(|b| b.as_ref().map_or(1, Vec::len)).collect()
}

/// `1 tool` / `3 tools`.
fn count_of(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// A turn's counts after its header text: `  · 3 tools · 1 msg · ✗1`. Empty when there are none.
fn turn_counts(tools: usize, msgs: usize, errors: usize) -> String {
    let mut parts = Vec::new();
    if tools > 0 {
        parts.push(count_of(tools, "tool", "tools"));
    }
    if msgs > 0 {
        parts.push(count_of(msgs, "msg", "msgs"));
    }
    if errors > 0 {
        parts.push(format!("✗{errors}"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("  · {}", parts.join(" · "))
    }
}

/// A turn row's header at `text_w` cells: the first line of its prompt `src` (one line already), and the
/// counts `suffix`. The suffix goes when the prompt would keep fewer than `MIN_HEAD_W` cells beside it.
fn turn_head(src: &str, text_w: usize, suffix: &str) -> (String, String) {
    let avail = text_w.saturating_sub(MARK_W);
    let sw = suffix.width();
    if sw > 0 && avail.saturating_sub(sw) >= MIN_HEAD_W {
        (first_line(src, avail - sw), suffix.to_string())
    } else {
        (first_line(src, avail), String::new())
    }
}

/// The first line `word_wrap` makes of `src` at `w` cells; empty for empty text.
fn first_line(src: &str, w: usize) -> String {
    modal_window::word_wrap(src, w.max(1))
        .first()
        .map(|l| l.to_string())
        .unwrap_or_default()
}

/// The text an event's header and prompt are drawn from. A message row shows its preview (its text, not a
/// tool call's summary), else its detail; other rows show their intent, else their detail, else their preview.
fn index_text(e: &EventRow) -> &str {
    let detail = e.intent.as_deref().filter(|d| !d.is_empty()).or(e.detail.as_deref().filter(|d| !d.is_empty()));
    let preview = (!e.preview.is_empty()).then_some(e.preview.as_str());
    if traces_outline::is_message(e) {
        preview.or(detail).unwrap_or_default()
    } else {
        detail.or(preview).unwrap_or_default()
    }
}

/// The text a header's continuation is cut from: `body` with its harness wrappers stripped, when that
/// leaves text; else `body` as it is.
fn continuation_source(body: &str) -> String {
    let clean = xai_grok_traces::normalize::clean_text(body);
    if clean.is_empty() { body.to_string() } else { clean }
}

/// The text a header's continuation is cut from when the event's body is `cached`, else `index`: the index
/// text's preview ellipsis is a truncation marker, not text, so it is dropped.
fn continuation_full(cached: Option<&String>, index: &str) -> String {
    match cached {
        Some(body) => continuation_source(body),
        None => index.strip_suffix('…').unwrap_or(index).to_string(),
    }
}

/// `… N more lines` for a prompt of `total` continuation rows; past the `wrap_body` cap, `N+`.
fn more_label(total: usize) -> String {
    if total > BODY_LINES {
        format!("… {}+ more lines", BODY_LINES - PROMPT_LEAD)
    } else {
        format!("… {} more lines", total.saturating_sub(PROMPT_LEAD))
    }
}

/// Splits a composed body at its result separator: the event's own text, then the result section (empty
/// when there is none).
pub(super) fn split_result(text: &str) -> (&str, &str) {
    match [RESULT_SEP, RESULT_ERR_SEP]
        .iter()
        .filter_map(|sep| text.find(*sep))
        .min()
    {
        Some(at) => (
            text.get(..at).unwrap_or(text).trim_end(),
            text.get(at..).unwrap_or_default(),
        ),
        None => (text, ""),
    }
}

/// Byte offset where `want` ends inside `full`, matching from the start with whitespace runs equal. `None`
/// when they disagree or `full` runs out first. The offset is past the last matched non-space char.
fn match_end(want: &str, full: &str) -> Option<usize> {
    let want: Vec<char> = want.chars().collect();
    let mut matched = 0;
    let mut end = 0;
    let mut chars = full.char_indices().peekable();
    while let Some(&w) = want.get(matched) {
        let (at, c) = chars.next()?;
        if c.is_whitespace() {
            if matched == 0 {
                continue;
            }
            if w != ' ' {
                return None;
            }
            matched += 1;
            while chars.peek().is_some_and(|&(_, d)| d.is_whitespace()) {
                chars.next();
            }
        } else {
            if w != c {
                return None;
            }
            matched += 1;
            end = at + c.len_utf8();
        }
    }
    Some(end)
}

/// Where a cut at `end` lands on a word: when `full` continues mid-token there, the start of that token,
/// so the continuation begins with the whole word. Otherwise `end`.
fn whole_token_at(full: &str, end: usize) -> usize {
    let mid_token = full
        .get(end..)
        .and_then(|s| s.chars().next())
        .is_some_and(|c| !c.is_whitespace());
    if !mid_token {
        return end;
    }
    full.get(..end)
        .unwrap_or_default()
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map_or(0, |(i, c)| i + c.len_utf8())
}

/// What `full` has past `header`, the first line of its header text (`header` is the same one-line source,
/// wrapped). Whitespace runs compare equal, and a header cut mid-token still matches. The matched text and
/// the whitespace after it are dropped; the rest keeps its own lines. A trailing `…` on the header is a
/// preview cut, not text: when the header as-is does not match, it is retried without the `…`, and the
/// continuation then starts at the whole word the cut landed in. `full` comes back unchanged when nothing
/// matches, so no text is lost to a mismatch.
fn continuation(header: &str, full: &str) -> String {
    let header = one_line(header);
    let end = match match_end(&header, full) {
        Some(end) => end,
        None => match header
            .strip_suffix('…')
            .and_then(|bare| match_end(bare, full))
        {
            Some(end) => whole_token_at(full, end),
            None => return full.to_string(),
        },
    };
    full.get(end..).unwrap_or_default().trim_start().to_string()
}

fn render_inspector(buf: &mut Buffer, area: Rect, state: &mut TracesModalState, theme: &Theme) {
    if state.peek.is_none() {
        render_trajectory(buf, area, state, theme);
    }
    let chrome = if state.session.is_some() && state.peek.is_none() { FULL_HEADER_ROWS } else { 0 };
    let pane = Rect { y: area.y + chrome, height: area.height.saturating_sub(chrome), ..area };
    if pane.height == 0 {
        return;
    }
    let dim = Style::default().fg(theme.gray);
    if state.peek.is_none() && !state.payload.meta {
        let running = state.session.as_ref().is_some_and(|s| s.status == Status::Active);
        let head = match state.selected_event().and_then(|i| state.events.get(i)) {
            Some(e) => traces_payload::Head {
                title: if e.title.is_empty() { role_chip(e).to_string() } else { e.title.clone() },
                running,
                duration: fmt_duration(e.duration_ms),
                time: fmt_time(e.timestamp, true),
            },
            None => traces_payload::Head { title: String::new(), running, duration: String::new(), time: String::new() },
        };
        traces_payload::render(buf, pane, &state.payload, &head, &mut state.raw_scroll, theme);
        render_status(buf, area, state, theme);
        return;
    }
    if let Some(e) = state.selected_event().and_then(|i| state.events.get(i)) {
        let head = format!("{} · line {} · {} · {}", e.event_type, e.line, fmt_time(e.timestamp, true), fmt_duration(e.duration_ms));
        buf.set_span(pane.x, pane.y, &Span::styled(truncate_str(&head, pane.width as usize).as_str(), Style::default().fg(theme.text_primary).add_modifier(Modifier::BOLD)), pane.width);
    }
    let Some(raw) = &state.raw else { return };
    let wrap = state.payload.wrap && state.peek.is_none();
    let lines: Vec<String> = raw.lines().flat_map(|l| if wrap { traces_payload::hard_wrap(l, pane.width as usize) } else { vec![l.to_string()] }).collect();
    let status_rows = u16::from(state.status.is_some());
    let body = Rect { y: pane.y + 2, height: pane.height.saturating_sub(2 + status_rows), ..pane };
    state.raw_scroll = state.raw_scroll.min(lines.len().saturating_sub(body.height as usize));
    for (i, line) in lines.iter().skip(state.raw_scroll).take(body.height as usize).enumerate() {
        let style = if line.trim_start().starts_with('"') { Style::default().fg(theme.text_primary) } else { dim };
        buf.set_span(body.x, body.y + i as u16, &Span::styled(truncate_str(line, body.width as usize).as_str(), style), body.width);
    }
    render_status(buf, area, state, theme);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(source: &str, path: &str) -> SessionRow {
        SessionRow {
            id: "x".into(),
            source: source.into(),
            title: "t".into(),
            file_path: path.into(),
            model: None,
            cwd: None,
            started_at: None,
            updated_at: None,
            event_count: 0,
            user_count: 0,
            assistant_count: 0,
            tool_count: 0,
            error_count: 0,
            input_tokens: None,
            output_tokens: None,
            duration_ms: None,
            status: Status::Unknown,
            parent_id: None,
            label: None,
            agent_type: None,
            now: None,
            last_event_at: None,
            children: 0,
            failed: false,
            host: None,
            ..Default::default()
        }
    }

    #[test]
    fn resume_targets() {
        let c = resume_target(&row("claude", "/h/.claude/projects/-w/3a680ac6-2551-4c67-88d3-a18d8b3bbe73.jsonl")).unwrap();
        assert_eq!(c.native_id, "3a680ac6-2551-4c67-88d3-a18d8b3bbe73");
        let x = resume_target(&row("codex", "/h/.codex/sessions/2026/06/06/rollout-2026-06-06T16-07-29-019e9f31-031e-7fb0-9562-b793939a558a.jsonl")).unwrap();
        assert_eq!(x.native_id, "019e9f31-031e-7fb0-9562-b793939a558a");
        let g = resume_target(&row("grok", "/h/.grok/sessions/%2Fw/01a10b35-4b73-7903-a589-181aba7b40f5/chat_history.jsonl")).unwrap();
        assert_eq!(g.native_id, "01a10b35-4b73-7903-a589-181aba7b40f5");
        assert_eq!(resume_target(&row("dsh", "/h/.dsh/sessions/a/b/session.v2.jsonl.zstd")), None);
        assert_eq!(resume_target(&row("bb", "/h/.bb/bb.db#thr_x")), None);
        assert_eq!(source_label("bb"), "bb");
    }

    #[test]
    fn resume_actions_copy_commands() {
        let cmd = |source: &str, path: &str, cwd: Option<&str>| {
            let mut r = row(source, path);
            r.cwd = cwd.map(Into::into);
            resume_action(&r)
        };
        let copy = |s: &str| Some(ResumeAction::CopyCommand(s.into()));
        assert_eq!(cmd("pi", "/h/.pi/s/a.jsonl", Some("/w")), copy("cd /w && pi --session /h/.pi/s/a.jsonl"));
        assert_eq!(cmd("omp", "/h/.omp/s/a.jsonl", Some("/w")), copy("cd /w && omp --resume /h/.omp/s/a.jsonl"));
        assert_eq!(cmd("pi", "/h/.pi/s/a.jsonl", None), copy("pi --session /h/.pi/s/a.jsonl"));
        assert_eq!(cmd("pi", "/h/my s/a.jsonl", Some("/w x")), copy("cd '/w x' && pi --session '/h/my s/a.jsonl'"));
        assert_eq!(
            cmd("cursor", "/h/.cursor/projects/p/agent-transcripts/c1/c1.jsonl", Some("/w")),
            copy("cursor-agent --resume c1")
        );
        assert!(matches!(cmd("claude", CLAUDE_PATH, None), Some(ResumeAction::InPager(_))));
        assert_eq!(cmd("dsh", DSH_PATH, None), None);
        assert_eq!(cmd("bb", "/h/.bb/bb.db#thr_x", None), None);
    }

    #[test]
    fn r_copies_pi_command_and_reports_it() {
        let mut state = list_state("pi", PI_PATH, vec![]);
        state.sessions[0].cwd = Some("/w".into());
        state.rebuild_ledger();
        let want = "cd /w && pi --session /h/.pi/sessions/synthetic-pi.jsonl";
        match handle_traces_key(&mut state, &KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)) {
            InputOutcome::Action(Action::TracesCopy { text }) => assert_eq!(text, want),
            _ => panic!("r on a pi row must copy"),
        }
        state.report_copy(&crate::clipboard::CopyDelivery::Clipboard {
            result: crate::clipboard::CopyResult {
                message: "Copied",
                message_lead: "Copied",
                ticks: 30,
                delivery: crate::clipboard::ClipboardDelivery::Confirmed,
            },
            file: None,
        });
        assert_eq!(state.status.as_deref(), Some(&*format!("copied: {want}")));
    }

    #[test]
    fn r_without_action_says_so_and_hides_hint() {
        let mut state = list_state("dsh", DSH_PATH, vec![]);
        match handle_traces_key(&mut state, &KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)) {
            InputOutcome::Action(Action::TracesNotice { text }) => assert_eq!(text, "no resume command for dsh"),
            _ => panic!("r on dsh must notice"),
        }
        assert!(!hint_labels(&state, u16::MAX).contains(&"r resume"));
    }

    fn hint_labels(state: &TracesModalState, width: u16) -> Vec<&'static str> {
        shortcuts(state, width).iter().map(|sc| sc.label).collect()
    }

    fn list_state(source: &str, path: &str, hosts: Vec<String>) -> TracesModalState {
        let mut state = TracesModalState::open_with_hosts(None, hosts);
        state.mode = Mode::List;
        if !source.is_empty() {
            state.sessions = vec![row(source, path)];
            state.rebuild_ledger();
        }
        state
    }

    /// Synthetic paths only. Claude and grok resolve a resume target; pi and dsh do not.
    const CLAUDE_PATH: &str = "/h/.claude/projects/-w/3a680ac6-2551-4c67-88d3-a18d8b3bbe73.jsonl";
    const GROK_PATH: &str = "/h/.grok/sessions/%2Fw/01a10b35-4b73-7903-a589-181aba7b40f5/chat_history.jsonl";
    const PI_PATH: &str = "/h/.pi/sessions/synthetic-pi.jsonl";
    const DSH_PATH: &str = "/h/.dsh/sessions/a/b/session.v2.jsonl.zstd";

    #[test]
    fn hint_bar_lists_only_keys_that_work() {
        let has = |state: &TracesModalState, label: &'static str| hint_labels(state, u16::MAX).contains(&label);

        let empty = list_state("", "", vec![]);
        assert!(!has(&empty, "H host"), "no hosts configured");
        assert!(!has(&empty, "r resume"), "nothing selected");
        assert!(has(&empty, "g/G ends"));

        let hosts_only = list_state("", "", vec!["user@synthetic".into()]);
        assert!(has(&hosts_only, "H host"));
        assert!(!has(&hosts_only, "r resume"));

        let pi = list_state("pi", PI_PATH, vec!["user@synthetic".into()]);
        assert!(has(&pi, "H host"));
        assert!(has(&pi, "r resume"), "pi copies a command");
        assert!(has(&pi, "g/G ends"));

        let claude = list_state("claude", CLAUDE_PATH, vec![]);
        assert!(!has(&claude, "H host"));
        assert!(has(&claude, "r resume"));

        let claude_hosts = list_state("claude", CLAUDE_PATH, vec!["user@synthetic".into()]);
        assert!(has(&claude_hosts, "H host"));
        assert!(has(&claude_hosts, "r resume"));
        assert!(has(&claude_hosts, "g/G ends"));

        let mut dsh = TracesModalState::open_with_hosts(None, vec![]);
        dsh.mode = Mode::Trajectory;
        dsh.session = Some(row("dsh", DSH_PATH));
        assert!(!has(&dsh, "r resume"), "dsh has no resume target");
        assert!(has(&dsh, "g/G ends"));

        let mut grok = TracesModalState::open_with_hosts(None, vec![]);
        grok.mode = Mode::Trajectory;
        grok.session = Some(row("grok", GROK_PATH));
        assert!(has(&grok, "r resume"));
        assert!(has(&grok, "g/G ends"));

        let prompt = trajectory(vec![ev(1, "message", "user", "synthetic prompt")]);
        assert!(prompt.turn_prompt_text().is_some(), "y copies only from a prompt row");
        assert!(has(&prompt, "y copy"));

        let mut live = TracesModalState::open_with_hosts(None, vec![]);
        live.mode = Mode::Live;
        live.set_live(
            vec![
                LiveRow {
                    session_id: "root".into(),
                    source: "claude".into(),
                    title: "parent".into(),
                    state: LiveState::Working,
                    text: "synthetic".into(),
                    age_ms: 1_000,
                    ..Default::default()
                },
                LiveRow {
                    session_id: "kid".into(),
                    parent_id: Some("root".into()),
                    source: "claude".into(),
                    title: "child".into(),
                    label: Some("child".into()),
                    state: LiveState::Working,
                    text: "synthetic".into(),
                    age_ms: 500,
                    ..Default::default()
                },
            ],
            vec![],
        );
        live.live_sel = Some("root".into());
        let folded = handle_traces_key(&mut live, &KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert!(matches!(folded, InputOutcome::Changed), "c folds the open root");
        live.live_sel = Some("root".into());
        assert!(has(&live, "c fold"));
        assert!(has(&live, "o open"));
        assert!(!has(&live, "q back"), "q stays unlisted");
    }

    #[test]
    fn hint_bar_drops_overflow_keys_in_order() {
        let mut state = tree_state(&[("r", None), ("c", Some("r"))]);
        state.hosts = vec!["user@synthetic".into()];
        let wide = hint_labels(&state, 400);
        assert!(wide.contains(&"E all") && wide.contains(&"</> more/less") && wide.contains(&"x errors"));
        assert!(modal_window::shortcuts_rows_needed(&shortcuts(&state, 400), 400) <= 2);

        let mid = hint_labels(&state, 110);
        assert!(!mid.contains(&"E all"), "E drops first");
        assert!(mid.contains(&"</> more/less"));
        assert!(mid.contains(&"x errors"));
        assert!(mid.contains(&"H host") && mid.contains(&"r resume") && mid.contains(&"g/G ends"));
        assert!(modal_window::shortcuts_rows_needed(&shortcuts(&state, 110), 110) <= 2);

        let tight = hint_labels(&state, 100);
        assert!(!tight.contains(&"E all") && !tight.contains(&"</> more/less"));
        assert!(tight.contains(&"x errors"), "x drops after E and </>");
        assert!(modal_window::shortcuts_rows_needed(&shortcuts(&state, 100), 100) <= 2);

        let mut grok = TracesModalState::open_with_hosts(None, vec![]);
        grok.mode = Mode::Trajectory;
        grok.session = Some(row("grok", GROK_PATH));
        let traj = hint_labels(&grok, 90);
        assert!(!traj.contains(&"E all") && !traj.contains(&"</> more/less"));
        assert!(traj.contains(&"n noise"), "n drops after </>");
        assert!(traj.contains(&"g/G ends") && traj.contains(&"r resume"));
        assert!(modal_window::shortcuts_rows_needed(&shortcuts(&grok, 90), 90) <= 2);
    }

    fn line(ts: &str, text: &str) -> String {
        format!("{{\"type\":\"message\",\"timestamp\":\"{ts}\",\"message\":{{\"role\":\"user\",\"content\":\"{text}\"}}}}\n")
    }

    /// Three pi sessions (a < b < c by time), indexed, with a modal and a writer handle on one db.
    fn live_fixture() -> (tempfile::TempDir, [xai_grok_traces::Root; 1], TraceIndex, TracesModalState) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("pi");
        std::fs::create_dir_all(&dir).unwrap();
        for (n, ts) in [("a", "2026-10-01T00:00:01Z"), ("b", "2026-10-01T00:00:02Z"), ("c", "2026-10-01T00:00:03Z")] {
            std::fs::write(dir.join(format!("{n}.jsonl")), line(ts, &format!("task {n}"))).unwrap();
        }
        let roots = [xai_grok_traces::Root {
            id: "pi".into(),
            source: "pi".into(),
            path: dir,
            format: xai_grok_traces::Format::Jsonl,
            file_name: None,
        }];
        let db = tmp.path().join("idx/t.sqlite");
        let mut writer = TraceIndex::open(&db).unwrap();
        writer.scan(&roots).unwrap();
        let mut state = TracesModalState::open_with_hosts(Some(TraceIndex::open(&db).unwrap()), vec![]);
        // One-line fixtures count as trivial; this fixture is about ordering and live updates.
        state.show_all = true;
        state.refresh();
        (tmp, roots, writer, state)
    }

    fn hook(writer: &mut TraceIndex, roots: &[xai_grok_traces::Root], file: &std::path::Path, append: &str) {
        use std::io::Write;
        std::fs::OpenOptions::new().append(true).open(file).unwrap().write_all(append.as_bytes()).unwrap();
        let payload = format!(r#"{{"hook_event_name":"PostToolUse","transcript_path":"{}"}}"#, file.display());
        xai_grok_traces::hook::ingest(writer, roots, xai_grok_traces::hook::extract("pi", payload.as_bytes())).unwrap();
    }

    fn titles(state: &TracesModalState) -> Vec<&str> {
        state.sessions.iter().map(|s| s.title.as_str()).collect()
    }

    /// Polls until the worker's snapshot (if one was started) has been applied; true if anything redrew.
    fn settle(state: &mut TracesModalState) -> bool {
        let mut redraw = state.poll_live();
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while state.inflight.is_some() && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(5));
            redraw |= state.poll_live();
        }
        redraw
    }

    #[test]
    fn live_poll_keeps_selection_by_session_id() {
        let (_tmp, roots, mut writer, mut state) = live_fixture();
        assert_eq!(titles(&state), ["task c", "task b", "task a"]);
        assert!(!state.live);
        state.selected = 1;
        assert!(!state.poll_live(), "nothing changed");

        let a = roots[0].path.join("a.jsonl");
        hook(&mut writer, &roots, &a, &line("2026-10-01T00:00:09Z", "later"));
        assert!(settle(&mut state));
        assert_eq!(titles(&state), ["task a", "task c", "task b"], "a moved to the top");
        assert_eq!((state.selected, state.selected_session().unwrap().title.as_str()), (2, "task b"));
        assert!(state.live, "fresh receipt shows the live marker");
        assert!(state.poll_live_at(i64::MAX / 2), "marker flips off after the window");
        assert!(!state.live);
    }

    #[test]
    fn live_poll_appends_events_and_follows_the_bottom() {
        let (_tmp, roots, mut writer, mut state) = live_fixture();
        let a = roots[0].path.join("a.jsonl");
        state.selected = 2; // task a
        state.open_selected();
        assert_eq!(state.events.len(), 1);
        state.goto(state.oshown.len() - 1);

        hook(&mut writer, &roots, &a, &line("2026-10-01T00:00:05Z", "second"));
        assert!(settle(&mut state));
        assert_eq!((state.events.len(), state.selected_event()), (2, Some(1)), "at the bottom: follows");

        state.goto(0);
        hook(&mut writer, &roots, &a, &line("2026-10-01T00:00:06Z", "third"));
        assert!(settle(&mut state));
        assert_eq!((state.events.len(), state.selected_event()), (3, Some(0)), "mid-list: stays put");
    }

    /// `TRACES_DB=<copy of traces.sqlite> cargo test -p xai-grok-pager --lib traces_perf -- --ignored --nocapture`
    #[test]
    #[ignore = "profiling against a real index"]
    fn traces_perf() {
        let Some(db) = std::env::var_os("TRACES_DB") else { return };
        let t = |label: &str, f: &mut dyn FnMut()| {
            let start = std::time::Instant::now();
            f();
            eprintln!("{label:<28} {:>8.2} ms", start.elapsed().as_secs_f64() * 1e3);
        };
        if std::env::var_os("TRACES_SCAN").is_some() {
            let mut w = TraceIndex::open(std::path::Path::new(&db)).unwrap();
            t("scan (background thread)", &mut || {
                w.scan(&xai_grok_traces::user_roots()).unwrap();
            });
        }
        let mut state = None;
        t("open (ledger query)", &mut || state = Some(TracesModalState::open_with_hosts(Some(TraceIndex::open(std::path::Path::new(&db)).unwrap()), vec![])));
        let mut state = state.unwrap();
        eprintln!("ledger items {} roots {}", state.ledger.len(), state.sessions.len());
        t("render ledger (warm theme)", &mut || {
            draw(&mut state, 200, 50);
        });
        t("poll, nothing changed", &mut || {
            state.poll_live();
        });
        t("render ledger 200x50", &mut || {
            draw(&mut state, 200, 50);
        });
        t("reload_sessions (sync)", &mut || state.reload_sessions());
        state.watermark = -1;
        t("poll, index changed (UI)", &mut || {
            state.poll_live();
        });
        t("  worker settle", &mut || {
            settle(&mut state);
        });
        // The biggest session by events.
        let big = state.index.as_ref().unwrap().list_sessions(&SessionFilter { sort: Sort::Events, limit: 1, ..Default::default() }).unwrap();
        let Some(big) = big.into_iter().next() else { return };
        eprintln!("biggest: {} events", big.event_count);
        t("open_session (big)", &mut || state.open_session(big.clone()));
        eprintln!("trajectory events {}", state.events.len());
        t("render trajectory 200x50", &mut || {
            draw(&mut state, 200, 50);
        });
        t("load_events (poll path)", &mut || state.load_events());
    }

    #[test]
    fn formats() {
        assert_eq!(fmt_duration(Some(61_000)), "1m01s");
        assert_eq!(fmt_tokens(Some(12_345)), "12.3k");
        assert_eq!(
            [3_000, 42_000, 180_000, 7_200_000, 5 * 86_400_000].map(fmt_age),
            ["now", "42s", "3m", "2h", "5d"]
        );
    }

    #[test]
    fn names_lose_wrapper_noise() {
        let mut r = row("claude", "/p/a.jsonl");
        r.title = "<command-message>## Fix the index</command-message>".into();
        assert_eq!(clean_name(&r), "Fix the index");
        r.label = Some("  Explore index code ".into());
        assert_eq!(clean_name(&r), "Explore index code", "dispatch label beats title");
        r.label = None;
        r.title = "chat_history.jsonl".into();
        r.cwd = Some("/w/proj/".into());
        assert_eq!(clean_name(&r), "proj session");
    }

    const PID: &str = "0b7e1c2a-0000-4000-8000-0000000000aa";

    fn jl(lines: &[&str]) -> String {
        lines.join("\n") + "\n"
    }

    /// A Claude parent that dispatched one Agent, its subagent transcript and meta, indexed; a modal on the same db.
    fn claude_fixture() -> (tempfile::TempDir, TracesModalState) {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("claude/-w");
        let subs = proj.join(PID).join("subagents");
        std::fs::create_dir_all(&subs).unwrap();
        std::fs::write(
            proj.join(format!("{PID}.jsonl")),
            jl(&[
                r#"{"type":"file-history-snapshot","timestamp":"2026-10-01T00:00:00Z","snapshot":{}}"#,
                r#"{"type":"user","timestamp":"2026-10-01T00:00:01Z","cwd":"/w","message":{"role":"user","content":"index everything please"}}"#,
                r#"{"type":"assistant","timestamp":"2026-10-01T00:00:02Z","message":{"role":"assistant","model":"claude-opus-5-5","content":[{"type":"tool_use","name":"Agent","input":{"description":"Explore index code","subagent_type":"Explore","prompt":"look at the index"}}]}}"#,
                r#"{"type":"user","timestamp":"2026-10-01T00:00:09Z","message":{"role":"user","content":[{"type":"tool_result","content":"found it"}]}}"#,
                r#"{"type":"assistant","timestamp":"2026-10-01T00:00:10Z","message":{"role":"assistant","content":"all done here"}}"#,
            ]),
        )
        .unwrap();
        std::fs::write(
            subs.join("agent-x.jsonl"),
            jl(&[
                r#"{"type":"user","timestamp":"2026-10-01T00:00:03Z","message":{"role":"user","content":"look at the index and report"}}"#,
                r#"{"type":"assistant","timestamp":"2026-10-01T00:00:04Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash","input":{"command":"rg index"}}]}}"#,
                r#"{"type":"user","timestamp":"2026-10-01T00:00:05Z","message":{"role":"user","content":[{"type":"tool_result","content":"ok"}]}}"#,
                r#"{"type":"assistant","timestamp":"2026-10-01T00:00:08Z","message":{"role":"assistant","content":"the index lives in index.rs"}}"#,
            ]),
        )
        .unwrap();
        std::fs::write(subs.join("agent-x.meta.json"), r#"{"agentType":"Explore","description":"Explore index code"}"#).unwrap();
        let roots = [xai_grok_traces::Root {
            id: "claude-projects".into(),
            source: "claude".into(),
            path: tmp.path().join("claude"),
            format: xai_grok_traces::Format::Jsonl,
            file_name: None,
        }];
        let db = tmp.path().join("idx/t.sqlite");
        TraceIndex::open(&db).unwrap().scan(&roots).unwrap();
        let state = TracesModalState::open_with_hosts(Some(TraceIndex::open(&db).unwrap()), vec![]);
        (tmp, state)
    }

    fn press(state: &mut TracesModalState, code: KeyCode) {
        handle_traces_key(state, &KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn draw(state: &mut TracesModalState, w: u16, h: u16) -> String {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let theme = Theme::current();
        match state.mode {
            Mode::Trajectory => render_trajectory(&mut buf, area, state, &theme),
            Mode::Inspector => render_inspector(&mut buf, area, state, &theme),
            _ => render_ledger(&mut buf, area, state, &theme),
        }
        (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect::<Vec<_>>().join("\n")
    }

    /// Right-aligned flag text on the ledger header (tabs sit on the left, padded by spaces).
    fn header_flags(screen: &str) -> String {
        let line = screen.lines().next().unwrap_or("").trim_end();
        match line.rfind("   ") {
            Some(i) => line[i..].trim().to_string(),
            None => line.trim().to_string(),
        }
    }

    #[test]
    fn ledger_flags_omit_default_sort() {
        let mut state = TracesModalState::open_with_hosts(None, vec![]);
        state.scanning = false;
        let flags = header_flags(&draw(&mut state, 160, 12));
        assert!(flags.contains("0 sessions"), "{flags}");
        assert!(!flags.contains("sort:"), "{flags}");

        state.sort = Sort::Events;
        let flags = header_flags(&draw(&mut state, 160, 12));
        assert!(flags.contains("sort:events"), "{flags}");

        state.errors_only = true;
        let flags = header_flags(&draw(&mut state, 160, 12));
        let errors_at = flags.find("errors").unwrap_or_else(|| panic!("{flags}"));
        let sort_at = flags.find("sort:events").unwrap_or_else(|| panic!("{flags}"));
        let counts_at = flags.rfind("0 sessions").unwrap_or_else(|| panic!("{flags}"));
        assert!(errors_at < sort_at && sort_at < counts_at, "{flags}");
        assert!(flags.ends_with("0 sessions"), "{flags}");
    }

    #[test]
    fn ledger_nests_subagents_under_their_parent() {
        let (_tmp, mut state) = claude_fixture();
        assert_eq!(state.sessions.len(), 1, "roots only by default");
        assert_eq!(state.ledger.len(), 2, "parent then its child");
        assert!(matches!(&state.nodes[state.ledger[1]].item, Item::Row { row } if row.parent_id.is_some()));
        let screen = draw(&mut state, 140, 20);
        assert!(screen.lines().any(|l| l.contains("└─") && l.contains("Explore index code")), "{screen}");
        assert!(screen.contains("index everything please"), "{screen}");
        assert!(screen.contains("2 sessions"), "{screen}");

        // `a` flattens: no tree prefixes, both sessions are plain rows.
        press(&mut state, KeyCode::Char('a'));
        assert!(state.show_all);
        assert_eq!(state.sessions.len(), 2);
        assert!(!draw(&mut state, 140, 20).contains("└─"));
    }

    #[test]
    fn ctrl_and_alt_chords_do_not_fire_nav_letters() {
        let (_tmp, mut state) = claude_fixture();
        let before = state.show_all;
        handle_traces_key(&mut state, &KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        handle_traces_key(&mut state, &KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT));
        assert_eq!(state.show_all, before);
        press(&mut state, KeyCode::Char('x'));
        assert!(state.errors_only);
        handle_traces_key(&mut state, &KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
        assert!(state.errors_only, "Ctrl+X must not toggle the errors filter");
    }

    #[test]
    fn many_children_collapse_until_expanded() {
        let (_tmp, mut state) = claude_fixture();
        let kid = match &state.nodes[state.ledger[1]].item {
            Item::Row { row } => row.clone(),
            _ => unreachable!(),
        };
        let parent = state.sessions[0].id.clone();
        state.children.insert(parent.clone(), (0..9).map(|i| SessionRow { id: format!("k{i}"), ..kid.clone() }).collect());
        state.rebuild_ledger();
        assert_eq!(state.ledger.len(), 1 + MAX_KIDS + 1);
        assert!(matches!(state.nodes[*state.ledger.last().unwrap()].item, Item::More { hidden: 3, .. }));
        assert!(draw(&mut state, 140, 20).contains("+3 more"));
        state.selected = state.ledger.len() - 1;
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.mode, Mode::List, "Enter on +N more expands instead of opening");
        assert_eq!(state.ledger.len(), 1 + 9);
    }

    /// A ledger from `(id, parent)` edges; parents must come before their children.
    fn tree_state(edges: &[(&str, Option<&str>)]) -> TracesModalState {
        let mut state = TracesModalState::open_with_hosts(None, vec![]);
        let mut children: HashMap<String, Vec<SessionRow>> = HashMap::new();
        for &(id, parent) in edges {
            let r = SessionRow { id: id.into(), title: format!("task {id}"), ..row("claude", "/h/x.jsonl") };
            match parent {
                None => state.sessions.push(r),
                Some(p) => children.entry(p.into()).or_default().push(r),
            }
        }
        state.children = children;
        state.rebuild_ledger();
        state
    }

    /// Session ids of the visible ledger lines, in order.
    fn shown(state: &TracesModalState) -> Vec<String> {
        state.ledger.iter().map(|&n| state.tree.nodes[n].key.clone()).collect()
    }

    fn select(state: &mut TracesModalState, id: &str) {
        state.selected = state.position_of(id).expect("row is shown");
    }

    #[test]
    fn nested_tree_renders_and_folds() {
        // r ─┬─ c1 ── g1 ── gg1
        //    └─ c2
        let mut state = tree_state(&[("r", None), ("c1", Some("r")), ("g1", Some("c1")), ("gg1", Some("g1")), ("c2", Some("r"))]);
        assert_eq!(shown(&state), ["r", "c1", "c2"], "roots open, deeper parents start folded");
        let screen = draw(&mut state, 140, 20);
        assert!(screen.lines().any(|l| l.contains("├─ ›") && l.contains("(+2)")), "{screen}");

        select(&mut state, "c1");
        press(&mut state, KeyCode::Char('l'));
        assert_eq!(shown(&state), ["r", "c1", "g1", "c2"]);
        select(&mut state, "g1");
        press(&mut state, KeyCode::Right);
        assert_eq!(shown(&state), ["r", "c1", "g1", "gg1", "c2"]);
        let screen = draw(&mut state, 140, 20);
        assert!(screen.lines().any(|l| l.contains("│     └─") && l.contains("task gg1")), "{screen}");

        // `h` on a leaf selects its parent; on an open parent it folds; on a folded one it moves up.
        select(&mut state, "gg1");
        press(&mut state, KeyCode::Char('h'));
        assert_eq!(state.selected_session().unwrap().id, "g1");
        assert_eq!(shown(&state), ["r", "c1", "g1", "gg1", "c2"], "parent stays open");
        press(&mut state, KeyCode::Char('h'));
        assert_eq!(shown(&state), ["r", "c1", "g1", "c2"], "open parent folds");
        assert_eq!(state.selected_session().unwrap().id, "g1");
        press(&mut state, KeyCode::Char('h'));
        assert_eq!(state.selected_session().unwrap().id, "c1", "folded parent: selection moves up");
        press(&mut state, KeyCode::Char('e'));
        assert_eq!(shown(&state), ["r", "c1", "c2"], "e folds the open node");
        let screen = draw(&mut state, 140, 20);
        assert!(screen.lines().any(|l| l.contains("├─ ›") && l.contains("task c1") && l.contains("(+2)")), "{screen}");
    }

    #[test]
    fn angle_brackets_fold_a_subtree_and_leave_siblings_alone() {
        let mut state = tree_state(&[
            ("r", None),
            ("c1", Some("r")),
            ("g1", Some("c1")),
            ("gg1", Some("g1")),
            ("c2", Some("r")),
            ("r2", None),
            ("s1", Some("r2")),
            ("s1a", Some("s1")),
        ]);
        assert_eq!(shown(&state), ["r", "c1", "c2", "r2", "s1"]);
        press(&mut state, KeyCode::Char('E'));
        assert_eq!(shown(&state).len(), 8, "E expands everything when anything is folded");

        select(&mut state, "c1");
        press(&mut state, KeyCode::Char('<'));
        assert_eq!(shown(&state), ["r", "c1", "c2", "r2", "s1", "s1a"], "< folds c1 and g1, nothing else");

        select(&mut state, "r2");
        press(&mut state, KeyCode::Char('<'));
        assert_eq!(shown(&state), ["r", "c1", "c2", "r2"], "< on r2 folds its branch only");

        select(&mut state, "r");
        press(&mut state, KeyCode::Char('>'));
        assert_eq!(shown(&state), ["r", "c1", "g1", "gg1", "c2", "r2"], "> opens r's branch; r2 stays folded");

        press(&mut state, KeyCode::Char('E'));
        assert_eq!(shown(&state).len(), 8, "E expands everything since r2 is folded");
    }

    #[test]
    fn selection_and_folds_survive_a_live_requery() {
        let mut state = tree_state(&[("r", None), ("c1", Some("r")), ("g1", Some("c1")), ("c2", Some("r"))]);
        select(&mut state, "c1");
        press(&mut state, KeyCode::Char('e'));
        assert_eq!(shown(&state), ["r", "c1", "g1", "c2"]);
        select(&mut state, "c2");

        // A requery that adds a root in front of the others.
        let mut sessions = vec![SessionRow { id: "new".into(), ..state.sessions[0].clone() }];
        sessions.extend(state.sessions.clone());
        let snap = Snapshot { generation: state.generation, ledger: Ok((sessions, state.children.clone())), trajectory: None };
        assert!(state.apply(snap));
        assert_eq!(state.selected_session().unwrap().id, "c2", "selection follows its session id");
        assert_eq!(shown(&state), ["new", "r", "c1", "g1", "c2"], "c1 stays expanded");
    }

    #[test]
    fn noise_is_hidden_until_n() {
        let (_tmp, mut state) = claude_fixture();
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.mode, Mode::Trajectory);
        assert!(state.events.iter().all(|e| !e.noise), "no noise rows by default");
        let quiet = state.events.len();
        press(&mut state, KeyCode::Char('n'));
        assert!(state.include_noise);
        assert!(state.events.len() > quiet && state.events.iter().any(|e| e.noise), "n shows the snapshot row");
        assert!(draw(&mut state, 140, 24).contains("noise shown"));
        press(&mut state, KeyCode::Char('n'));
        assert_eq!(state.events.len(), quiet);
    }

    #[test]
    fn results_fold_into_their_call_and_agent_rows_open_the_child() {
        let (_tmp, mut state) = claude_fixture();
        let parent = state.selected_session().unwrap().id.clone();
        press(&mut state, KeyCode::Enter);
        assert!(state.hidden.iter().zip(&state.events).all(|(&h, e)| h || !is_result(e)), "adjacent results are hidden");
        assert!(shown_events(&state).iter().all(|&i| !is_result(&state.events[i])), "and get no row of their own");
        let at = state.events.iter().position(is_dispatch).expect("an Agent row");
        assert!(matches!(&state.folded[at], Some(Fold { error: false, .. })));
        state.sel = Some(OutlineKey::Ev(state.events[at].line));
        let screen = draw(&mut state, 140, 24);
        assert!(screen.contains("Agent  Explore index code"), "{screen}");
        assert!(screen.contains("→ Explore index code") && screen.contains("✓"), "{screen}");

        press(&mut state, KeyCode::Enter);
        let child = state.session.clone().unwrap();
        assert_eq!((state.mode, child.parent_id.as_deref()), (Mode::Trajectory, Some(parent.as_str())));
        let screen = draw(&mut state, 140, 24);
        assert!(screen.contains("rg index") && screen.contains("↑ "), "child shows its tool and its parent\n{screen}");

        press(&mut state, KeyCode::Esc);
        assert_eq!(state.session.as_ref().unwrap().id, parent, "Esc returns to the parent");
        assert_eq!(state.selected_event(), Some(at), "at the row we left");
        press(&mut state, KeyCode::Esc);
        assert_eq!(state.mode, Mode::List);
    }

    #[test]
    fn remote_rows_merge_and_render_with_a_host() {
        let (_tmp, mut state) = claude_fixture();
        state.hosts = vec!["sab@sab-air".into()];
        let mut r = row("codex", "/remote/x.jsonl");
        r.id = "sab-air:codex:abc".into();
        r.title = "remote refactor job".into();
        r.host = Some("sab-air".into());
        r.updated_at = Some(i64::MAX / 4);
        r.status = Status::Active;
        let mut kid = r.clone();
        kid.id = "sab-air:codex:kid".into();
        kid.title = "remote kid".into();
        kid.parent_id = Some(r.id.clone());
        state.apply_host(HostResult { target: "sab@sab-air".into(), result: Ok(Fetched { rows: vec![r, kid], note: None }) });
        assert_eq!(state.shown_roots.len(), 2, "local root + remote root; the remote child nests");
        assert_eq!(state.shown_roots[0].host.as_deref(), Some("sab-air"), "newest first across hosts");
        let screen = draw(&mut state, 140, 20);
        assert!(screen.contains("HOST") && screen.contains("sab-air") && screen.contains("remote refactor job"), "{screen}");
        assert!(screen.lines().any(|l| l.contains("└─") && l.contains("remote kid")), "{screen}");

        // Resume is refused for remote rows.
        state.selected = 0;
        assert!(matches!(state.resume(state.selected_session().cloned()), InputOutcome::Action(Action::TracesNotice { .. })));

        // An unreachable host keeps its rows and says so in the header.
        state.apply_host(HostResult {
            target: "sab@sab-air".into(),
            result: Err(remote::Error::Unreachable { host: "sab@sab-air".into(), detail: "timeout".into() }),
        });
        assert_eq!(state.shown_roots.len(), 2);
        assert!(draw(&mut state, 160, 20).contains("sab-air unreachable"));

        // H filters to this machine only.
        press(&mut state, KeyCode::Char('H'));
        assert_eq!(state.shown_roots.len(), 1);
        assert!(state.shown_roots[0].host.is_none());
    }

    /// Outline rows as `Turn(1)`-style keys, in order.
    fn shown_rows(state: &TracesModalState) -> Vec<String> {
        state.oshown.iter().filter_map(|&n| state.key_at(n)).map(|k| format!("{k:?}")).collect()
    }

    /// Event indices of the visible event rows.
    fn shown_events(state: &TracesModalState) -> Vec<usize> {
        state
            .oshown
            .iter()
            .filter_map(|&n| match state.outline_items.get(n)? {
                OutlineItem::Event(i) => Some(*i),
                _ => None,
            })
            .collect()
    }

    fn select_call(state: &mut TracesModalState, line: i64) {
        let pos = state.oshown.iter().position(|&n| matches!(state.key_at(n), Some(OutlineKey::Ev(l)) if *l == line)).expect("call row");
        state.goto(pos);
    }

    fn ev(line: i64, kind: &str, role: &str, text: &str) -> EventRow {
        EventRow {
            line,
            event_type: "x".into(),
            kind: kind.into(),
            role: Some(role.into()),
            title: if kind == "tool" { "Bash".into() } else { String::new() },
            preview: text.into(),
            timestamp: None,
            duration_ms: None,
            input_tokens: None,
            output_tokens: None,
            turn: None,
            step: None,
            depth: 0,
            model: None,
            is_error: false,
            noise: false,
            detail: None,
            intent: None,
        }
    }

    /// A trajectory over `rows` with no index behind it.
    fn trajectory(rows: Vec<EventRow>) -> TracesModalState {
        let mut state = TracesModalState::open_with_hosts(None, vec![]);
        state.session = Some(row("claude", "/h/x.jsonl"));
        state.mode = Mode::Trajectory;
        state.install_trajectory(rows, None, vec![]);
        state
    }

    /// Bookkeeping rows: not user turns, so the outline stays one synthetic turn.
    fn synthetic_events(n: usize) -> Vec<EventRow> {
        (1..=n).map(|i| ev(i as i64, "note", "system", "synthetic")).collect()
    }

    /// The trajectory header (source, title, counts), wide enough that a notice is not cut off.
    fn meta_line(state: &mut TracesModalState) -> String {
        rows_at(state, 160, 12).first().cloned().unwrap_or_default()
    }

    #[test]
    fn trajectory_event_row_is_within_the_first_three_rows() {
        let mut state = trajectory(vec![ev(1, "message", "user", "first event text")]);
        let session = state.session.as_mut().unwrap();
        session.event_count = 4;
        session.tool_count = 2;
        session.context_tokens = Some(50);
        session.context_window = Some(200);
        session.title = "narrow terminal title that has to clip before the counts".into();
        let rows = rows_at(&mut state, 80, 24);
        let at = rows.iter().position(|l| l.contains("#1")).expect("event row 1");
        assert!(at < 3, "event row 1 within the first 3 content rows, at {at}:\n{}", rows.join("\n"));
        assert!(rows[0].contains("4 events") && rows[0].contains("2 tools") && rows[0].contains("ctx 25%"), "{}", rows[0]);
        state.mode = Mode::Inspector;
        assert!(rows_at(&mut state, 80, 24).iter().any(|l| l.contains("ACTIVITY")), "inspector keeps the multi-row header");
    }

    #[test]
    fn meta_line_names_a_truncated_event_page() {
        let mut state = trajectory(synthetic_events(EVENT_PAGE));
        state.session.as_mut().unwrap().event_count = 6000;
        let meta = meta_line(&mut state);
        assert!(meta.contains("showing 5000/6000"), "{meta}");

        state.session.as_mut().unwrap().event_count = 5000;
        let meta = meta_line(&mut state);
        assert!(!meta.contains("showing"), "{meta}");

        state.session.as_mut().unwrap().event_count = 6000;
        state.event_query = "synthetic".into();
        let meta = meta_line(&mut state);
        assert!(!meta.contains("showing"), "{meta}");

        state.event_query.clear();
        state.session.as_mut().unwrap().host = Some("box".into());
        state.remote_loading = true;
        let meta = meta_line(&mut state);
        assert!(!meta.contains("showing"), "remote fetch still in flight: {meta}");
    }

    /// A user prompt, three Bash calls with their results (lines 2, 4, 6), and a reply.
    fn three_calls() -> Vec<EventRow> {
        vec![
            ev(1, "message", "user", "fix it"),
            ev(2, "tool", "tool", "ls"),
            ev(3, "tool", "tool_result", "a"),
            ev(4, "tool", "tool", "pwd"),
            ev(5, "tool", "tool_result", "b"),
            ev(6, "tool", "tool", "date"),
            ev(7, "tool", "tool_result", "c"),
            ev(8, "message", "assistant", "done"),
        ]
    }

    /// A user prompt, `n` Bash calls with their results, and a reply. Call `i` is line `2 + 2i`.
    fn n_calls(n: usize) -> Vec<EventRow> {
        let mut rows = vec![ev(1, "message", "user", "fix it")];
        let mut line = 2;
        for _ in 0..n {
            rows.push(ev(line, "tool", "tool", "ls"));
            line += 1;
            rows.push(ev(line, "tool", "tool_result", "a"));
            line += 1;
        }
        rows.push(ev(line, "message", "assistant", "done"));
        rows
    }

    #[test]
    fn turn_collapses_to_a_summary_and_back() {
        let mut state = trajectory(three_calls());
        assert_eq!(
            shown_rows(&state),
            ["Turn(1)", "Run(2)", "Ev(2)", "Ev(4)", "Ev(6)", "Ev(8)"],
            "turn open, short run open"
        );
        assert!(draw(&mut state, 140, 20).contains("3 tool calls"));

        state.goto(0);
        press(&mut state, KeyCode::Char('h'));
        assert_eq!(shown_rows(&state), ["Turn(1)"]);
        assert!(draw(&mut state, 140, 20).contains("3 tools · 1 msg"), "{}", draw(&mut state, 140, 20));

        press(&mut state, KeyCode::Char('l'));
        assert_eq!(shown_rows(&state).len(), 6);
    }

    #[test]
    fn small_runs_start_open_long_runs_stay_closed() {
        let mut state = trajectory(three_calls());
        assert_eq!(
            shown_rows(&state),
            ["Turn(1)", "Run(2)", "Ev(2)", "Ev(4)", "Ev(6)", "Ev(8)"],
            "a 3-call run shows its calls on first render"
        );
        assert!(draw(&mut state, 140, 24).contains("Bash  ls"), "{}", draw(&mut state, 140, 24));

        let mut long = trajectory(n_calls(12));
        assert_eq!(
            shown_rows(&long),
            ["Turn(1)", "Run(2)", "Ev(26)"],
            "a 12-call run stays collapsed"
        );
        let screen = draw(&mut long, 140, 24);
        assert!(screen.contains("12 tool calls"), "{screen}");
        assert!(!screen.contains("Bash"), "{screen}");

        state.goto(1);
        press(&mut state, KeyCode::Char('h'));
        assert!(!shown_rows(&state).contains(&"Ev(2)".to_string()), "h collapses the short run");

        let (tx, rx) = mpsc::channel();
        tx.send(Snapshot {
            generation: state.generation,
            ledger: Ok((state.sessions.clone(), state.children.clone())),
            trajectory: Some(Loaded {
                session_id: "x".into(),
                rows: three_calls(),
                session: None,
                children: vec![],
            }),
        })
        .unwrap();
        state.inflight = Some(rx);
        assert!(state.poll_live_at(1), "the queued snapshot is a requery");
        assert!(
            !shown_rows(&state).contains(&"Ev(2)".to_string()),
            "a poll_live_at requery keeps the manual collapse"
        );
    }

    fn queue_trajectory(state: &mut TracesModalState, rows: Vec<EventRow>) {
        let (tx, rx) = mpsc::channel();
        tx.send(Snapshot {
            generation: state.generation,
            ledger: Ok((Vec::new(), HashMap::new())),
            trajectory: Some(Loaded { session_id: "x".into(), rows, session: None, children: Vec::new() }),
        })
        .unwrap();
        state.inflight = Some(rx);
    }

    #[test]
    fn follow_mode_pins_new_rows_until_a_move_key() {
        let mut idle = trajectory(three_calls());
        assert!(!hint_labels(&idle, u16::MAX).contains(&"f follow"));
        press(&mut idle, KeyCode::Char('f'));
        assert!(!idle.follow);

        let mut state = trajectory(three_calls());
        state.session.as_mut().unwrap().status = Status::Active;
        assert!(hint_labels(&state, u16::MAX).contains(&"f follow"));
        press(&mut state, KeyCode::Char('f'));
        assert_eq!(state.status.as_deref(), Some("follow on"));
        assert!(draw(&mut state, 80, 12).lines().next().is_some_and(|l| l.contains("FOL")));
        state.goto(0);

        let was = state.sel_pos();
        let mut rows = three_calls();
        rows.push(ev(9, "message", "assistant", "later"));
        queue_trajectory(&mut state, rows);
        assert!(state.poll_live_at(1));
        assert_eq!(state.sel_pos(), state.oshown.len() - 1);
        assert!(state.sel_pos() > was);

        press(&mut state, KeyCode::Char('k'));
        assert!(!state.follow);
        assert_eq!(state.status.as_deref(), Some("follow off"));
        let stayed = state.sel.clone();
        let mut rows = three_calls();
        rows.push(ev(9, "message", "assistant", "later"));
        rows.push(ev(10, "message", "assistant", "again"));
        queue_trajectory(&mut state, rows);
        assert!(state.poll_live_at(1));
        assert_eq!(state.sel, stayed);

        press(&mut state, KeyCode::Char('f'));
        assert_eq!(state.status.as_deref(), Some("follow on"));
        press(&mut state, KeyCode::Char('f'));
        assert_eq!(state.status.as_deref(), Some("follow off"));
        assert!(!draw(&mut state, 80, 12).lines().next().is_some_and(|l| l.contains("FOL")));
    }

    #[test]
    fn bang_and_star_jump_errors_and_wrap() {
        let mut rows = Vec::new();
        for i in 0..10 {
            let role = if i == 0 { "user" } else { "assistant" };
            let mut e = ev(i, "message", role, "synthetic");
            e.is_error = matches!(i, 2 | 7 | 9);
            rows.push(e);
        }
        let mut state = trajectory(rows);
        assert_eq!(state.selected_event(), Some(0));
        let jumps = [(2, "error 1/3"), (7, "error 2/3"), (9, "error 3/3"), (2, "error 1/3")];
        for (idx, status) in jumps {
            press(&mut state, KeyCode::Char('!'));
            assert_eq!(state.selected_event(), Some(idx));
            assert_eq!(state.status.as_deref(), Some(status));
        }
        press(&mut state, KeyCode::Char('*'));
        assert_eq!(state.selected_event(), Some(9), "`*` from 2 wraps to the last error");
        assert_eq!(state.status.as_deref(), Some("error 3/3"));
    }

    #[test]
    fn bang_reveals_an_error_inside_a_collapsed_run() {
        let mut failed = ev(3, "tool", "tool_result", "synthetic fail");
        failed.is_error = true;
        let rows = vec![
            ev(1, "message", "user", "synthetic"),
            ev(2, "tool", "tool", "call-a"),
            failed,
            ev(4, "tool", "tool", "call-b"),
            ev(5, "tool", "tool_result", "synthetic ok"),
            ev(6, "tool", "tool", "call-c"),
            ev(7, "tool", "tool_result", "synthetic ok"),
            ev(8, "tool", "tool", "call-d"),
            ev(9, "tool", "tool_result", "synthetic ok"),
            ev(10, "tool", "tool", "call-e"),
            ev(11, "tool", "tool_result", "synthetic ok"),
            ev(12, "tool", "tool", "call-f"),
            ev(13, "tool", "tool_result", "synthetic ok"),
        ];
        let mut state = trajectory(rows);
        assert!(!shown_events(&state).contains(&1), "the run starts collapsed");
        press(&mut state, KeyCode::Char('!'));
        assert_eq!(state.selected_event(), Some(1));
        assert!(shown_events(&state).contains(&1), "the folded error row is revealed");
        assert_eq!(state.status.as_deref(), Some("error 1/1"));
    }

    #[test]
    fn bang_with_no_errors_keeps_the_selection() {
        let mut state = trajectory(three_calls());
        let before = state.sel.clone();
        let event = state.selected_event();
        press(&mut state, KeyCode::Char('!'));
        assert_eq!(state.selected_event(), event);
        assert_eq!(state.sel, before);
        assert_eq!(state.status.as_deref(), Some("no errors"));
        press(&mut state, KeyCode::Char('*'));
        assert_eq!(state.selected_event(), event);
        assert_eq!(state.sel, before);
        assert_eq!(state.status.as_deref(), Some("no errors"));
    }

    #[test]
    fn peek_shows_partial_then_final_and_esc_keeps_selection() {
        let mut state = trajectory(vec![ev(1, "message", "user", "go"), ev(2, "tool", "tool", "ls")]);
        state.session.as_mut().unwrap().status = Status::Active;
        select_call(&mut state, 2);
        let sel = state.sel.clone();
        press(&mut state, KeyCode::Char('p'));
        assert!(draw(&mut state, 80, 16).contains("no output yet"));
        state.raw_lines.insert(2, r#"{"input":{"command":"ls"},"output":"partial text"}"#.into());
        assert!(state.poll_live_at(1), "peek re-reads on the poll");
        assert!(draw(&mut state, 80, 16).contains("partial text"));
        let many = (1..=25).map(|n| format!("row{n:02}")).collect::<Vec<_>>().join("\n");
        state.raw_lines.insert(2, serde_json::json!({ "output": many }).to_string());
        assert!(state.poll_live_at(1));
        let screen = draw(&mut state, 40, 30);
        assert!(screen.contains("row06") && screen.contains("row25") && !screen.contains("row05"), "{screen}");
        press(&mut state, KeyCode::Char('j'));
        press(&mut state, KeyCode::Esc);
        assert_eq!((state.mode, state.peek, state.sel), (Mode::Trajectory, None, sel));

        let mut done = trajectory(vec![ev(1, "message", "user", "go"), ev(2, "tool", "tool", "ls"), ev(3, "tool", "tool_result", "ok")]);
        done.session.as_mut().unwrap().status = Status::Active;
        done.raw_lines.insert(2, r#"{"output":"partial text"}"#.into());
        done.raw_lines.insert(3, r#"{"type":"tool_result","content":"final output"}"#.into());
        select_call(&mut done, 2);
        let sel = done.sel.clone();
        press(&mut done, KeyCode::Char('p'));
        let screen = draw(&mut done, 80, 16);
        assert!(screen.contains("final output") && !screen.contains("partial text"), "{screen}");
        press(&mut done, KeyCode::Esc);
        assert_eq!((done.mode, done.sel), (Mode::Trajectory, sel));
    }

    #[test]
    fn bang_and_star_type_into_the_search_prompt() {
        let mut state = trajectory(three_calls());
        let event = state.selected_event();
        let status = state.status.clone();
        press(&mut state, KeyCode::Char('/'));
        press(&mut state, KeyCode::Char('!'));
        press(&mut state, KeyCode::Char('*'));
        assert_eq!(state.event_query, "!*");
        assert_eq!(state.selected_event(), event);
        assert_eq!(state.status, status);
    }

    #[test]
    fn run_is_one_row_until_expanded() {
        let mut state = trajectory(n_calls(12));
        assert_eq!(shown_rows(&state), ["Turn(1)", "Run(2)", "Ev(26)"], "a long run starts closed");
        state.goto(1);
        press(&mut state, KeyCode::Right);
        assert!(shown_rows(&state).contains(&"Ev(2)".to_string()));
        assert!(shown_rows(&state).contains(&"Ev(24)".to_string()));
        assert!(draw(&mut state, 140, 40).contains("Bash  ls"));
    }

    #[test]
    fn angle_bracket_opens_runs_and_bodies_under_a_turn() {
        let mut state = trajectory(three_calls());
        state.goto(0);
        press(&mut state, KeyCode::Char('>'));
        assert_eq!(shown_rows(&state).len(), 10, "{:?}", shown_rows(&state));
        assert!(shown_rows(&state).contains(&"Body(8)".to_string()));

        press(&mut state, KeyCode::Char('G'));
        assert_eq!(state.selected_event(), Some(7), "G lands on the last visible row");
        press(&mut state, KeyCode::Char('g'));
        assert_eq!(state.selected_event(), Some(0));
    }

    #[test]
    fn body_expands_from_the_cached_text_not_the_index() {
        let mut state = trajectory(three_calls());
        state.bodies.insert(2, "{\n  \"cached\": \"yes\"\n}".into());
        state.goto(1);
        press(&mut state, KeyCode::Right);
        state.sel = Some(OutlineKey::Ev(2));
        press(&mut state, KeyCode::Char('l'));
        assert!(shown_rows(&state).contains(&"Body(2)".to_string()));
        let screen = draw(&mut state, 140, 24);
        assert!(screen.contains("\"cached\": \"yes\""), "{screen}");
    }

    #[test]
    fn selection_follows_its_key_through_live_growth() {
        let mut state = trajectory(three_calls());
        state.goto(1);
        press(&mut state, KeyCode::Right);
        state.sel = Some(OutlineKey::Ev(6));
        let mut grown = three_calls();
        grown.push(ev(9, "message", "assistant", "more"));
        state.install_trajectory(grown, None, vec![]);
        assert_eq!(state.sel, Some(OutlineKey::Ev(6)));
        assert_eq!(state.selected_event(), Some(5));
    }

    #[test]
    fn filters_open_the_runs_that_hold_matches() {
        let mut state = trajectory(n_calls(12));
        assert!(!shown_rows(&state).contains(&"Ev(4)".to_string()));
        state.event_query = "pwd".into();
        state.rebuild_outline();
        assert!(shown_rows(&state).contains(&"Ev(4)".to_string()), "a filter opens the run");
        state.event_query.clear();
        state.rebuild_outline();
        assert!(shown_rows(&state).contains(&"Ev(4)".to_string()), "and it stays open once seen");
    }

    #[test]
    fn e_toggles_one_row_and_capital_e_toggles_all() {
        let mut state = trajectory(three_calls());
        state.goto(0);
        press(&mut state, KeyCode::Char('e'));
        assert_eq!(shown_rows(&state), ["Turn(1)"]);
        press(&mut state, KeyCode::Char('E'));
        assert_eq!(shown_rows(&state).len(), 10, "anything folded: expand all");
        press(&mut state, KeyCode::Char('E'));
        assert_eq!(shown_rows(&state), ["Turn(1)"], "nothing folded: collapse all");
    }

    #[test]
    fn fit_rows_keeps_the_selected_row_on_screen() {
        let mut scroll = 0;
        fit_rows(5, &mut scroll, &[1, 1, 1, 1, 1, 1], &[], 3);
        assert_eq!(scroll, 3);
        let mut scroll = 0;
        fit_rows(2, &mut scroll, &[1, 4, 1], &[], 3);
        assert_eq!(scroll, 2, "a tall row above pushes the top down");
    }

    /// 25 identical assistant replies under one prompt (lines 2..=26), scrolled to the end and then
    /// row `line` selected, so the list is scrolled and the selection sits mid-screen.
    fn identical_replies(line: i64) -> TracesModalState {
        let mut rows = vec![ev(1, "message", "user", "go")];
        rows.extend((2..=26).map(|l| ev(l, "message", "assistant", "reply")));
        let mut state = trajectory(rows);
        for l in [26, line] {
            state.sel = Some(OutlineKey::Ev(l));
            draw(&mut state, 80, ANCHOR_H);
        }
        state
    }

    const ANCHOR_H: u16 = 20;

    /// Screen line of event row `line`'s header.
    fn header_y(state: &mut TracesModalState, line: i64) -> usize {
        let tag = format!("#{line}");
        rows_at(state, 80, ANCHOR_H)
            .iter()
            .position(|r| r.find(&tag).is_some_and(|i| !r[i + tag.len()..].starts_with(|c: char| c.is_ascii_digit())))
            .unwrap_or_else(|| panic!("row #{line} is on screen"))
    }

    const LONG_BODY: &str = "reply\nsecond line\nthird line\nfourth line";

    #[test]
    fn traces_anchor_l_keeps_the_header_line_before_and_after_the_body_loads() {
        let mut state = identical_replies(13);
        let y = header_y(&mut state, 13);
        press(&mut state, KeyCode::Char('l'));
        assert_eq!(header_y(&mut state, 13), y, "loading row");
        assert!(state.settle_body(13, Ok(LONG_BODY.to_string())));
        assert_eq!(header_y(&mut state, 13), y, "body loaded");
    }

    #[test]
    fn traces_anchor_capital_e_keeps_the_selected_header_line() {
        let mut state = identical_replies(13);
        let y = header_y(&mut state, 13);
        press(&mut state, KeyCode::Char('E'));
        assert_eq!(header_y(&mut state, 13), y, "everything expanded");
        for l in 2..=26 {
            state.settle_body(l, Ok(LONG_BODY.to_string()));
        }
        assert_eq!(header_y(&mut state, 13), y, "bodies loaded");
    }

    #[test]
    fn traces_anchor_h_on_an_expanded_row_keeps_the_header_line() {
        let mut state = identical_replies(13);
        press(&mut state, KeyCode::Char('l'));
        assert!(state.settle_body(13, Ok(LONG_BODY.to_string())));
        let y = header_y(&mut state, 13);
        press(&mut state, KeyCode::Char('h'));
        assert_eq!(header_y(&mut state, 13), y);
    }

    #[test]
    fn traces_anchor_body_arriving_above_does_not_move_the_selection() {
        let mut state = identical_replies(11);
        press(&mut state, KeyCode::Char('l'));
        state.sel = Some(OutlineKey::Ev(13));
        let y = header_y(&mut state, 13);
        assert!(state.settle_body(11, Ok(LONG_BODY.to_string())));
        assert_eq!(header_y(&mut state, 13), y);
    }

    #[test]
    fn bodies_wrap_to_width_and_cap() {
        assert_eq!(wrap_body("abcdef\nxy", 4), ["abcd", "ef", "xy"]);
        let long = "x\n".repeat(300);
        let lines = wrap_body(&long, 10);
        assert_eq!(lines.len(), BODY_LINES + 1);
        assert_eq!(lines.last().map(String::as_str), Some("… Enter for inspector"));
    }

    #[test]
    fn body_wrap_keeps_words_whole() {
        assert_eq!(wrap_body("aaa bbbb cc", 6), ["aaa", "bbbb", "cc"]);
        assert_eq!(wrap_body("ab cdef", 4), ["ab", "cdef"]);
    }

    #[test]
    fn body_wrap_cjk_rows_fit_odd_width() {
        let rows = wrap_body("中文字符测试", 5);
        assert_eq!(rows, ["中文", "字符", "测试"]);
        assert_eq!(wrap_body("中文 字符", 5), ["中文", "字符"]);
        for row in rows {
            assert!(cols(&row) <= 5, "{row:?} is wider than 5 columns");
        }
    }

    #[test]
    fn body_wrap_repeats_indent_on_continuations() {
        assert_eq!(wrap_body("  one two three", 8), ["  one", "  two", "  three"]);
    }

    #[test]
    fn body_wrap_indent_wider_than_row_is_dropped() {
        assert_eq!(wrap_body("      abc", 4), ["abc"]);
    }

    #[test]
    fn body_wrap_hard_splits_over_long_url() {
        assert_eq!(wrap_body("https://a.b/cdefghij", 8), ["https://", "a.b/cdef", "ghij"]);
    }

    #[test]
    fn body_wrap_keeps_blank_lines() {
        assert_eq!(wrap_body("ab\n\n  cd", 10), ["ab", "", "  cd"]);
        assert_eq!(wrap_body("ab\n   \ncd", 10), ["ab", "", "cd"]);
    }

    #[test]
    fn body_wrap_exact_fit_has_no_trailing_row() {
        assert_eq!(wrap_body("abcd", 4), ["abcd"]);
        assert_eq!(wrap_body("  abcd", 6), ["  abcd"]);
    }

    #[test]
    fn body_wrap_tiny_widths_do_not_panic() {
        assert_eq!(wrap_body("", 0), Vec::<String>::new());
        assert_eq!(wrap_body("  x", 0), ["x"]);
        assert_eq!(wrap_body("abc", 1), ["a", "b", "c"]);
        assert_eq!(wrap_body("中", 1), ["中"]);
    }

    #[test]
    fn body_wrap_expands_tabs() {
        assert_eq!(wrap_body("\tab", 10), ["    ab"]);
        assert_eq!(wrap_body("a\tb", 10), ["a    b"]);
    }

    #[test]
    fn activity_strip_buckets_events() {
        let mk = |t: i64, err: bool, turn: i64| EventRow {
            line: 0,
            event_type: "x".into(),
            kind: "tool".into(),
            role: Some("tool".into()),
            title: "Bash".into(),
            preview: String::new(),
            timestamp: Some(t),
            duration_ms: None,
            input_tokens: None,
            output_tokens: None,
            turn: Some(turn),
            step: None,
            depth: 0,
            model: None,
            is_error: err,
            noise: false,
            detail: None,
            intent: None,
        };
        let events = [mk(0, false, 1), mk(10, false, 1), mk(11, false, 1), mk(100, true, 2)];
        let b = bucket_events(&events, 11);
        assert_eq!((b[0], b[1], b[10]), ((1, false, true), (2, false, false), (1, true, true)));
    }

    #[test]
    fn body_text_reads_each_harness_shape() {
        // Claude assistant: the text block, then the command string (not pretty JSON).
        let claude = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Let me check."},{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}"#;
        assert_eq!(body_text(claude), "Let me check.\n\nls");
        // Claude tool result: content as a string, then as an array of text blocks.
        let plain = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"file.txt"}]}}"#;
        assert_eq!(body_text(plain), "file.txt");
        let blocks = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":[{"type":"text","text":"out 1"},{"type":"text","text":"out 2"}]}]}}"#;
        assert_eq!(body_text(blocks), "out 1\n\nout 2");
        // Codex response item.
        let codex = r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Done."}]}}"#;
        assert_eq!(body_text(codex), "Done.");
        // `cmd` and `script` are body text the same way `command` is.
        assert_eq!(body_text(r#"{"input":{"cmd":"echo hi"}}"#), "echo hi");
        assert_eq!(body_text(r#"{"arguments":{"script":"echo there"}}"#), "echo there");
        // Pi read: arguments carry no text, so the whole line pretty-prints.
        let pi = r#"{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","id":"c1","name":"read","arguments":{"path":"a.rs"}}]}}"#;
        assert_eq!(body_text(pi), pretty(Ok(Some(pi.to_string()))));
    }

    #[test]
    fn body_text_dedupes_and_falls_back_to_pretty() {
        let dup = r#"{"message":{"role":"user","content":"same"},"toolUseResult":{"stdout":"same","stderr":""}}"#;
        assert_eq!(body_text(dup), "same");
        assert_eq!(body_text(r#"{"type":"x"}"#), "{\n  \"type\": \"x\"\n}");
        assert_eq!(body_text("not json"), "not json");
        // Edit/Read inputs (path, description) are not text, so the call falls back to the whole line.
        let edit = r#"{"input":{"file_path":"a.rs","description":"edit a"}}"#;
        assert_eq!(body_text(edit), pretty(Ok(Some(edit.to_string()))));
        // Real text wins: the input object is not pretty-printed beside it.
        assert_eq!(body_text(r#"{"text":"note","input":{"file_path":"a.rs"}}"#), "note");
    }

    #[test]
    fn body_text_renders_bash_script_with_real_newlines() {
        // Five lines and no braces, so a pretty-printed command cannot pass these asserts.
        let script = "python3 -c \"\nimport os\nprint(1)\nprint(2)\n\"";
        assert_eq!(script.lines().count(), 5);
        let raw = serde_json::json!({
            "type": "assistant",
            "message": {
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "name": "Bash",
                    "input": {"command": script}
                }]
            }
        });
        let body = body_text(&serde_json::to_string(&raw).unwrap());
        assert!(body.contains(script), "{body:?}");
        assert!(!body.contains("\\n"), "{body:?}");
        assert!(!body.contains('{') && !body.contains('}'), "{body:?}");
    }

    #[test]
    fn body_text_omits_bare_usage_numbers() {
        let usage_only = r#"{"usage":{"input":3005,"output":0}}"#;
        let body = body_text(usage_only);
        assert!(body.lines().all(|line| line != "3005" && line != "0"), "{body:?}");
        // No text anywhere: the whole record pretty-prints, and those lines are not bare numbers.
        assert_eq!(body, pretty(Ok(Some(usage_only.to_string()))));

        let with_text = r#"{"text":"hello","usage":{"input":3005,"output":0}}"#;
        let body = body_text(with_text);
        assert_eq!(body, "hello");
        assert!(body.lines().all(|line| line != "3005" && line != "0"), "{body:?}");
    }

    #[test]
    fn guides_cut_levels_deeper_than_three() {
        assert_eq!(guide_prefix(&[true]), "└─ ");
        assert_eq!(guide_prefix(&[false, false, true]), "│  │  └─ ");
        assert_eq!(guide_prefix(&[false, true, false, true]), "…    │  └─ ");
    }

    #[test]
    fn a_skipped_sibling_does_not_leave_the_last_drawn_one_open() {
        let mk = |id: &str| SessionRow { id: id.into(), ..row("claude", "/h/x.jsonl") };
        // The second `a` repeats the first, so `session` skips it: `b` is the last line drawn.
        let kids: HashMap<String, Vec<SessionRow>> = HashMap::from([("p".to_string(), vec![mk("a"), mk("a"), mk("b")])]);
        let all_kids = HashSet::new();
        let mut flat = Flatten { kids: &kids, all_kids: &all_kids, tree: Vec::new(), nodes: Vec::new(), seen: HashSet::new() };
        flat.session(&mk("p"), None, Vec::new());
        let (_, nodes) = flat.finish();
        let guides: Vec<String> = nodes.iter().map(|n| guide_prefix(&n.guides)).collect();
        assert_eq!(guides, ["", "├─ ", "└─ "]);
    }

    #[test]
    fn esc_from_a_child_restores_the_parent_folds_and_selects_its_agent_row() {
        // Calls on lines 2, 4 and 6 form a run; line 6 is the Agent dispatch that opens the child.
        let mut rows = three_calls();
        if let Some(agent) = rows.get_mut(5) {
            agent.title = "Agent".into();
        }
        let mut state = trajectory(rows);
        state.session_children = vec![SessionRow { id: "child".into(), ..row("claude", "/h/child.jsonl") }];
        state.dispatch_child = state.match_dispatches();
        state.goto(1);
        press(&mut state, KeyCode::Right);
        state.sel = Some(OutlineKey::Ev(6));
        assert!(shown_rows(&state).contains(&"Ev(6)".to_string()));
        let before = state.ofolds.clone();

        press(&mut state, KeyCode::Enter);
        assert_eq!(state.session.as_ref().map(|s| s.id.as_str()), Some("child"));
        press(&mut state, KeyCode::Char('E'));
        assert_ne!(state.ofolds, before, "the child's folds are its own");

        press(&mut state, KeyCode::Esc);
        assert_eq!(state.session.as_ref().map(|s| s.id.as_str()), Some("x"));
        assert_eq!(state.ofolds, before, "the parent's folds come back");
        assert_eq!(state.sel, Some(OutlineKey::Ev(6)));
        assert!(shown_rows(&state).contains(&"Ev(6)".to_string()), "the Agent row is shown inside its run");
        assert!(state.trail.is_empty());
    }

    #[test]
    fn a_failed_body_read_is_not_cached_and_l_retries_it() {
        let mut state = trajectory(three_calls());
        state.body_pending.insert(2);
        state.body_tx.send(BodyMsg { session_id: "x".into(), line: 2, text: Err("boom".into()) }).unwrap();
        state.poll_remote();
        assert!(!state.bodies.contains_key(&2), "an error is not cached");
        assert!(!state.body_pending.contains(&2));
        assert!(state.status.as_deref().is_some_and(|s| s.contains("press l to retry")), "{:?}", state.status);

        // The row is skipped while its failure stands; `l` on a row that cannot fold clears it.
        state.goto(0);
        press(&mut state, KeyCode::Char('l'));
        assert!(state.body_failed.is_empty());
        assert!(!state.bodies.contains_key(&2));
    }

    #[test]
    fn a_filter_does_not_reopen_a_run_the_user_collapsed() {
        let mut state = trajectory(three_calls());
        state.event_errors_only = true;
        state.rebuild_outline();
        assert!(shown_rows(&state).contains(&"Ev(4)".to_string()), "the filter opens the run");
        state.goto(1);
        press(&mut state, KeyCode::Char('h'));
        assert!(!shown_rows(&state).contains(&"Ev(4)".to_string()));

        state.install_trajectory(three_calls(), None, vec![]);
        assert!(!shown_rows(&state).contains(&"Ev(4)".to_string()), "a live poll keeps the manual collapse");
    }

    #[test]
    fn turn_counts_list_the_nonzero_parts() {
        assert_eq!(turn_counts(14, 3, 1), "  · 14 tools · 3 msgs · ✗1");
        assert_eq!(turn_counts(1, 0, 0), "  · 1 tool");
        assert_eq!(turn_counts(0, 2, 0), "  · 2 msgs");
        assert_eq!(turn_counts(0, 0, 0), "");
    }

    #[test]
    fn turn_head_keeps_the_counts_beside_the_prompt_and_drops_them_when_narrow() {
        let counts = turn_counts(14, 3, 1);
        assert_eq!(
            turn_head("alpha beta gamma", 60, &counts),
            ("alpha beta gamma".to_string(), counts.clone())
        );
        // Eight cells stay for the prompt beside the counts, so it wraps there.
        assert_eq!(
            turn_head("alpha beta gamma", 36, &counts),
            ("alpha".to_string(), counts.clone())
        );
        // One cell fewer and the counts go; the whole prompt fits in what is left.
        assert_eq!(
            turn_head("alpha beta gamma", 35, &counts),
            ("alpha beta gamma".to_string(), String::new())
        );
        assert_eq!(
            turn_head("alpha beta gamma", 14, &counts),
            ("alpha beta".to_string(), String::new())
        );
        assert_eq!(turn_head("", 14, &counts), (String::new(), String::new()));
    }

    #[test]
    fn continuation_is_what_the_header_does_not_show() {
        assert_eq!(
            continuation("hello world", "hello world and more"),
            "and more"
        );
        assert_eq!(
            continuation("a b", "a b"),
            "",
            "a header that is the whole text leaves nothing"
        );
        assert_eq!(
            continuation("a b", "a b\n  c d\n\ne"),
            "c d\n\ne",
            "the rest keeps its lines"
        );
        assert_eq!(
            continuation("a  b", "a b\tc"),
            "c",
            "whitespace runs compare equal"
        );
        assert_eq!(
            continuation("https://ex.com/aa", "https://ex.com/aabb cc"),
            "bb cc",
            "a header cut mid-token"
        );
        assert_eq!(
            continuation("foo", "bar baz"),
            "bar baz",
            "disagreeing texts show everything"
        );
        assert_eq!(
            continuation("abc", "ab"),
            "ab",
            "a text shorter than the header"
        );
        assert_eq!(continuation("", "  x"), "x", "no header: the whole text");
    }

    #[test]
    fn continuation_drops_the_preview_ellipsis_and_the_wrappers() {
        assert_eq!(continuation("abc def…", "abc def ghi jkl"), "ghi jkl", "the preview's cut marker is not text");
        assert_eq!(continuation("abc def…", "abc def"), "", "a whole preview leaves nothing");
        assert_eq!(continuation("abc de…", "abc def ghi"), "def ghi", "the cut lands mid-word: the word is whole");
        assert_eq!(continuation("wait…", "wait… then more"), "then more", "a typed ellipsis is text when it matches");
        assert_eq!(continuation_full(None, "hello world…"), "hello world", "uncached: the index text without its marker");
        assert_eq!(continuation_full(Some(&"hello world".to_string()), "hello"), "hello world");

        let body = "<user_query>\nhello world\n</user_query>";
        assert_eq!(continuation("hello", &continuation_source(body)), "world");
        let injected = "<system-reminder>\nctx\n</system-reminder>\n\nhello world";
        assert_eq!(continuation("hello", &continuation_source(injected)), "world");
        let only = "<system-reminder>\nctx\n</system-reminder>";
        assert_eq!(continuation_source(only), only, "nothing but wrappers: the body as it is");
    }

    #[test]
    fn index_text_is_the_text_for_messages_and_the_summary_for_tools() {
        let mut msg = ev(1, "message", "assistant", "let me look");
        msg.detail = Some("Bash ls".into());
        assert_eq!(index_text(&msg), "let me look", "a message with a tool call shows its text");
        msg.preview.clear();
        assert_eq!(index_text(&msg), "Bash ls", "a message without text falls back to its detail");

        let mut call = ev(2, "tool", "tool", "ls");
        call.detail = Some("ls -la".into());
        assert_eq!(index_text(&call), "ls -la", "a tool row shows its summary");
        call.intent = Some("List files".into());
        assert_eq!(index_text(&call), "List files", "a tool row shows its intent over its summary");
        call.intent = None;
        call.detail = None;
        assert_eq!(index_text(&call), "ls", "a tool row without a summary falls back to its preview");
    }

    #[test]
    fn message_headers_come_from_the_text_and_tool_headers_from_the_summary() {
        let mut user = ev(1, "message", "user", "fix it");
        user.detail = Some("Bash ls".into());
        let mut reply = ev(3, "message", "assistant", "wrapup");
        reply.detail = Some("Bash pwd".into());
        let mut call = ev(2, "tool", "tool", "zzprobe");
        call.detail = Some("ls -la".into());
        let mut state = trajectory(vec![user, call, ev(4, "tool", "tool_result", "a"), reply]);
        let screen = draw(&mut state, 140, 24);
        let row = |tag: &str| screen.lines().find(|l| l.contains(tag)).unwrap_or_default().to_string();
        let turn = row("#1");
        assert!(turn.contains("fix it") && !turn.contains("Bash ls"), "{turn}");
        assert!(row("#2").contains("Bash  ls -la"), "{screen}");
        let answer = row("#3");
        assert!(answer.contains("wrapup") && !answer.contains("Bash pwd"), "{answer}");
    }

    #[test]
    fn turn_row_shows_its_line_chip_and_counts_when_expanded() {
        let mut state = trajectory(three_calls());
        let screen = draw(&mut state, 140, 20);
        let turn = screen
            .lines()
            .find(|l| l.contains("fix it"))
            .expect("turn row");
        assert!(
            turn.contains("#1") && turn.contains("USER") && turn.contains("· 3 tools · 1 msg"),
            "{turn}"
        );
    }

    #[test]
    fn turn_whose_opener_is_filtered_out_shows_the_preview() {
        let mut state = trajectory(three_calls());
        state.hidden[0] = true;
        state.rebuild_outline();
        assert!(matches!(state.outline_items[0], OutlineItem::Turn { opener: false, .. }), "{:?}", state.outline_items[0]);
        let screen = draw(&mut state, 140, 20);
        assert!(!screen.contains("turn ·"), "{screen}");
    }

    #[test]
    fn event_header_reads_the_same_collapsed_and_expanded() {
        let rows = vec![
            ev(1, "message", "user", "fix it"),
            ev(2, "tool", "tool", "zzprobe"),
            ev(3, "tool", "tool_result", "a"),
            ev(4, "message", "assistant", "wrapup"),
        ];
        let mut state = trajectory(rows);
        let row = |screen: &str, tag: &str| screen.lines().find(|l| l.contains(tag)).map(str::to_string).unwrap_or_default();

        let collapsed = row(&draw(&mut state, 140, 24), "#2");
        assert!(collapsed.contains("Bash  zzprobe"), "{collapsed}");
        state.sel = Some(OutlineKey::Ev(2));
        press(&mut state, KeyCode::Right);
        assert!(shown_rows(&state).contains(&"Body(2)".to_string()), "{:?}", shown_rows(&state));
        let expanded = row(&draw(&mut state, 140, 24), "#2");
        assert!(expanded.contains("Bash  zzprobe"), "{expanded}");
    }

    #[test]
    fn long_bash_row_at_80_cols_keeps_the_tail_the_fixed_chrome_clipped() {
        let cmd = "alpha alpha alpha alpha alpha alpha alpha CLIPPED tail";
        let mut call = ev(2, "tool", "tool", cmd);
        call.detail = Some(cmd.into());
        let mut state = trajectory(vec![ev(1, "message", "user", "run it"), call]);
        let row = draw(&mut state, 80, 16).lines().find(|l| l.contains("#2")).unwrap_or("").to_string();
        assert!(row.contains("CLIPPED"), "fixed chrome used to clip this tail at 80 cols:\n{row}");
    }

    /// An opener and two assistant rows. The middle one's cached body is `body`.
    fn two_replies(body: &str) -> TracesModalState {
        let mut state = trajectory(vec![
            ev(1, "message", "user", "Check"),
            ev(2, "message", "assistant", "done"),
            ev(3, "message", "assistant", "later"),
        ]);
        assert!(state.settle_body(2, Ok(body.to_string())));
        state.sel = Some(OutlineKey::Ev(2));
        press(&mut state, KeyCode::Char('l'));
        state
    }

    #[test]
    fn row_heights_map_an_empty_body_to_zero() {
        assert_eq!(
            row_heights(&[None, Some(vec![]), Some(vec!["a".into(), "b".into()])]),
            vec![1, 0, 2]
        );
    }

    #[test]
    fn expanded_event_whose_text_fits_draws_no_body_row() {
        let mut state = two_replies("done");
        assert!(
            shown_rows(&state).contains(&"Body(2)".to_string()),
            "the empty body stays an outline row: {:?}",
            shown_rows(&state)
        );
        let screen = draw(&mut state, 80, 24);
        assert!(!screen.contains("(no more)"), "{screen}");
        let list = list_lines(&screen.lines().map(|l| l.trim_end().to_string()).collect::<Vec<_>>());
        let at = list.iter().position(|l| l.contains("#2")).expect("event row");
        assert!(list[at].contains('⌄'), "the event stays expanded: {}", list[at]);
        assert!(
            list[at + 1].contains("#3"),
            "no phantom line under the event: {list:#?}"
        );
        press(&mut state, KeyCode::Char('j'));
        assert_eq!(state.sel, Some(OutlineKey::Ev(3)), "j skips the empty body");
        press(&mut state, KeyCode::Char('k'));
        assert_eq!(state.sel, Some(OutlineKey::Ev(2)), "k skips the empty body");
    }

    #[test]
    fn expanded_event_with_more_text_still_draws_body_rows() {
        let mut state = two_replies("done and then some more detail");
        let screen = draw(&mut state, 80, 24);
        assert!(screen.contains("and then some more detail"), "{screen}");
        assert!(!screen.contains("(no more)"), "{screen}");
        let list = list_lines(&screen.lines().map(|l| l.trim_end().to_string()).collect::<Vec<_>>());
        let at = list.iter().position(|l| l.contains("#2")).expect("event row");
        assert!(
            list[at + 1].contains("and then some more detail"),
            "the continuation is the next line: {list:#?}"
        );
        press(&mut state, KeyCode::Char('j'));
        assert_eq!(state.sel, Some(OutlineKey::Body(2)), "j lands on a real body row");
    }

    // ---- header stability, prompt rows, seams, snapshots ----

    const SNAP_W: u16 = 56;
    const SNAP_H: u16 = 16;
    const URL: &str = "https://example.com/very/long/path/that/does/not/break/anywhere";
    const BODY: &str = "https://example.com/docs/architecture/index-layout/shard-0042/manifest.json lists the shards, and the reader merges them at startup before any query runs.";

    /// The opener of the turn fixtures: a long multi-sentence prompt with a hex token and a long URL.
    fn opener_prompt() -> String {
        format!("Yoooooo please check the commit so the fix that landed as 7eb90d57 is verified, then read {URL} and summarize what changed and what is still open for the team.")
    }

    /// What the event row keeps in the index: the first 100 chars and an ellipsis.
    fn short_of(s: &str) -> String {
        if s.chars().count() > 100 {
            format!("{}…", s.chars().take(100).collect::<String>())
        } else {
            s.to_string()
        }
    }

    /// Rows of the trajectory at `w` x `h`, trailing spaces trimmed.
    fn rows_at(state: &mut TracesModalState, w: u16, h: u16) -> Vec<String> {
        draw(state, w, h).lines().map(|l| l.trim_end().to_string()).collect()
    }

    fn snapshot_rows(state: &mut TracesModalState) -> Vec<String> {
        rows_at(state, SNAP_W, SNAP_H)
    }

    /// The first row holding `tag`, or empty.
    fn row_with(rows: &[String], tag: &str) -> String {
        rows.iter().find(|r| r.contains(tag)).cloned().unwrap_or_default()
    }

    /// A row with the fold mark normalised, so collapsed and expanded rows compare.
    fn fold_normalised(row: &str) -> String {
        row.replace('⌄', "›")
    }

    /// One opener turn: the prompt (its full text cached when `body`), a call and an assistant reply.
    fn opener_turn(prompt: &str, body: bool) -> TracesModalState {
        let mut state = trajectory(vec![
            ev(1, "message", "user", &short_of(prompt)),
            ev(2, "tool", "tool", "ls"),
            ev(3, "message", "assistant", "done"),
        ]);
        if body {
            assert!(state.settle_body(1, Ok(prompt.to_string())));
        }
        draw(&mut state, SNAP_W, SNAP_H);
        state
    }

    /// An opener and an assistant reply whose body is `BODY`.
    fn reply_fixture() -> TracesModalState {
        let mut state = trajectory(vec![
            ev(1, "message", "user", "Check the build"),
            ev(2, "message", "assistant", &short_of(BODY)),
        ]);
        assert!(state.settle_body(2, Ok(BODY.to_string())));
        state
    }

    /// An opener, an Agent dispatch with its folded result, and a reply; the dispatch opens a child session.
    fn dispatch_fixture() -> TracesModalState {
        let mut agent = ev(2, "tool", "tool", "Explore index");
        agent.title = "Agent".into();
        let child = SessionRow { label: Some("Explore index".into()), status: Status::Completed, ..row("claude", "/h/child.jsonl") };
        let mut state = trajectory(vec![]);
        state.install_trajectory(
            vec![ev(1, "message", "user", "Check the build"), agent, ev(3, "tool", "tool_result", "found it"), ev(4, "message", "assistant", "done")],
            None,
            vec![child],
        );
        assert!(state.settle_body(2, Ok("look at the index and report".to_string())));
        state
    }

    #[test]
    fn trajectory_snapshot_turn_prompt() {
        let prompt = opener_prompt();
        let mut state = opener_turn(&prompt, true);
        let rows = snapshot_rows(&mut state);
        let at = rows.iter().position(|r| r.contains("#1")).expect("turn row");
        let shown: Vec<&str> = rows[at..at + 5].iter().map(String::as_str).collect();
        assert_eq!(
            shown,
            vec![
                "    #1 USER   ⌄ Yoooooo please check  · 1 tool · 1 msg",
                "                the commit so the fix that landed as",
                "                7eb90d57 is verified, then read",
                "                https://example.com/very/long/path/that/",
                "              › … 3 more lines",
            ]
        );
        assert_eq!(rows.iter().filter(|r| r.contains("Yoooooo")).count(), 1, "{rows:#?}");
        assert!(shown_rows(&state).contains(&"PromptMore(1)".to_string()));
        assert!(!shown_rows(&state).contains(&"PromptRest(1)".to_string()), "more rows default collapsed");

        state.sel = Some(OutlineKey::PromptMore(1));
        press(&mut state, KeyCode::Char('l'));
        let all = snapshot_rows(&mut state);
        for tok in prompt.split_whitespace().filter(|t| t.chars().count() <= 40) {
            assert!(all.iter().any(|r| r.split_whitespace().any(|w| w == tok)), "`{tok}` is not whole in any row:\n{all:#?}");
        }
        let i = all.iter().position(|r| r.contains("https://")).expect("url row");
        let piece = all[i].trim_start();
        assert!(all[i + 1].contains("more lines"), "{all:#?}");
        let next = all[i + 2].trim_start().split(' ').next().unwrap_or_default();
        assert_eq!(format!("{piece}{next}"), URL, "{all:#?}");
    }

    #[test]
    fn trajectory_snapshot_event_body() {
        let mut state = reply_fixture();
        state.sel = Some(OutlineKey::Ev(2));
        press(&mut state, KeyCode::Char('l'));
        assert!(shown_rows(&state).contains(&"Body(2)".to_string()), "{:?}", shown_rows(&state));
        let rows = snapshot_rows(&mut state);
        let at = rows.iter().position(|r| r.contains("#1")).expect("turn row");
        let shown: Vec<&str> = rows[at..at + 6].iter().map(String::as_str).collect();
        assert_eq!(
            shown,
            vec![
                "    #1 USER   ⌄ Check the build  · 1 msg",
                "    #2 ◆   ⌄ https://example.com/docs/architecture/index",
                "                  -layout/shard-0042/manifest.json",
                "                  lists the shards, and the reader",
                "                  merges them at startup before any",
                "                  query runs.",
            ]
        );
        // Header and body, with the whitespace dropped, spell the source exactly: no word repeats at the seam.
        let got: String = std::iter::once("https://example.com/docs/architecture/index".to_string())
            .chain(rows[at + 2..at + 6].iter().map(|r| r.trim().to_string()))
            .collect::<String>()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        let want: String = BODY.chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!(got, want);
    }

    /// One turn per string, in order. Short strings stay one line at 100 columns; `three_line_prompt`
    /// wraps to three.
    fn user_turns(texts: &[&str]) -> TracesModalState {
        trajectory(texts.iter().enumerate().map(|(i, t)| ev((i + 1) as i64, "message", "user", t)).collect())
    }

    /// 42 × "alpha" is three lines in the 84-column prompt cell of a 100-wide trajectory
    /// (text column 86, minus the fold mark).
    fn three_line_prompt() -> String {
        std::iter::repeat("alpha").take(42).collect::<Vec<_>>().join(" ")
    }

    /// List rows of a trajectory draw. The status line anchors the bottom; the one header row sits above.
    fn list_lines(rows: &[String]) -> Vec<String> {
        assert!(
            rows.last().is_some_and(|l| l.contains("trace index")),
            "status row anchors the list: {rows:#?}"
        );
        rows.get(1..rows.len() - 1).unwrap_or_default().to_vec()
    }

    #[test]
    fn trajectory_snapshot_single_line_blocks_stay_dense() {
        let mut state = user_turns(&["red", "green", "blue"]);
        let list = list_lines(&rows_at(&mut state, 100, 24));
        assert!(list[0].contains("#1") && list[0].contains("red"), "{list:#?}");
        assert!(list[1].contains("#2") && list[1].contains("green"), "{list:#?}");
        assert!(list[2].contains("#3") && list[2].contains("blue"), "{list:#?}");
        assert!(list[3].is_empty(), "no spacer after a single-line block: {list:#?}");
    }

    #[test]
    fn trajectory_snapshot_spacer_around_a_wrapped_turn() {
        let long = three_line_prompt();
        let mut state = user_turns(&["red", long.as_str(), "blue"]);
        let list = list_lines(&rows_at(&mut state, 100, 24));
        let mid = list.iter().position(|r| r.contains("#2")).expect("wrapped turn");
        assert_eq!(mid, 2, "one spacer between the single-line turn and the wrapped one: {list:#?}");
        assert!(list[0].contains("#1") && list[0].contains("red"), "{list:#?}");
        assert!(list[1].is_empty(), "{list:#?}");
        assert!(!list[mid + 1].is_empty() && !list[mid + 2].is_empty(), "the prompt is three lines: {list:#?}");
        assert!(list[mid + 3].is_empty(), "one spacer below the wrapped turn: {list:#?}");
        assert!(list[mid + 4].contains("#3") && list[mid + 4].contains("blue"), "{list:#?}");
    }

    #[test]
    fn trajectory_snapshot_one_spacer_between_tall_blocks() {
        let long = three_line_prompt();
        let mut state = user_turns(&[long.as_str(), long.as_str()]);
        let list = list_lines(&rows_at(&mut state, 100, 24));
        assert!(list[0].contains("#1"), "the first block has no spacer above it: {list:#?}");
        let second = list.iter().position(|r| r.contains("#2")).expect("second turn");
        assert!(second >= 2, "{list:#?}");
        assert!(list[second - 1].is_empty(), "{list:#?}");
        assert!(!list[second - 2].is_empty(), "exactly one spacer between the blocks: {list:#?}");
        assert!(!list[second].is_empty() && !list.get(second + 1).is_some_and(|l| l.is_empty()), "{list:#?}");
    }

    #[test]
    fn trajectory_snapshot_no_spacer_above_the_first_block() {
        let long = three_line_prompt();
        let mut state = user_turns(&[long.as_str(), "blue"]);
        let list = list_lines(&rows_at(&mut state, 100, 24));
        assert!(list[0].contains("#1"), "a multi-line block first on screen has no spacer above it: {list:#?}");
        assert!(!list[1].is_empty(), "the wrapped prompt stays against its header: {list:#?}");
    }

    #[test]
    fn trajectory_snapshot_selection_skips_the_spacer() {
        let long = three_line_prompt();
        let mut state = user_turns(&["red", long.as_str(), "blue"]);
        state.sel = Some(OutlineKey::Turn(3));
        let rows = rows_at(&mut state, 100, 24);
        let y = rows.iter().position(|r| r.contains("#3")).expect("selected turn");
        assert!(y > 5 && rows[y - 1].is_empty(), "the spacer sits above the selected row: {rows:#?}");
        // The terminal-native palette's highlight is Reset, which cannot be told from an unpainted cell.
        let _theme = crate::theme::cache::pin_theme();
        let was_locked = crate::theme::cache::terminal_native_locked();
        crate::theme::cache::set_terminal_native_lock(false);
        struct RestoreLock(bool);
        impl Drop for RestoreLock {
            fn drop(&mut self) {
                crate::theme::cache::set_terminal_native_lock(self.0);
            }
        }
        let _restore = RestoreLock(was_locked);
        let area = Rect::new(0, 0, 100, 24);
        let mut buf = Buffer::empty(area);
        let theme = Theme::current();
        render_trajectory(&mut buf, area, &mut state, &theme);
        assert_ne!(theme.bg_highlight, Color::Reset);
        assert_ne!(buf[(8, (y - 1) as u16)].bg, theme.bg_highlight, "the spacer is not selected");
        assert_eq!(buf[(8, y as u16)].bg, theme.bg_highlight, "the selected row is");
    }

    #[test]
    fn trajectory_snapshot_narrow_list_has_no_spacers() {
        // Height 9: one header row plus the status line leave a list of 7, under the cutoff of 8.
        let long = three_line_prompt();
        let mut state = user_turns(&["red", long.as_str(), "blue"]);
        let list = list_lines(&rows_at(&mut state, 100, 9));
        let mid = list.iter().position(|r| r.contains("#2")).expect("wrapped turn");
        let next = list.iter().position(|r| r.contains("#3")).expect("following turn");
        assert_eq!(mid, 1, "no spacer above the wrapped turn: {list:#?}");
        assert_eq!(next, mid + 3, "three prompt lines, then the next block, with no spacer: {list:#?}");
    }

    #[test]
    fn j_from_the_last_row_of_a_block_steps_one_outline_row() {
        let long = three_line_prompt();
        let mut state = user_turns(&["red", long.as_str(), "blue"]);
        rows_at(&mut state, 100, 24);
        let roots: Vec<usize> = (0..state.oshown.len())
            .filter(|&i| {
                state.oshown.get(i).and_then(|&n| state.outline.nodes.get(n)).is_some_and(|node| node.depth == 0)
            })
            .collect();
        assert!(roots.len() >= 3 && roots[2] > roots[1] + 1, "the wrapped turn is more than one row: {roots:?}");
        let last = roots[2] - 1;
        let len = state.oshown.len();
        state.goto(last);
        press(&mut state, KeyCode::Char('j'));
        assert_eq!(state.sel_pos(), last + 1);
        assert_eq!(state.oshown.len(), len, "a spacer is not an outline row");
    }

    #[test]
    fn turn_row_is_identical_collapsed_and_expanded() {
        let mut state = opener_turn(&opener_prompt(), true);
        let expanded = row_with(&snapshot_rows(&mut state), "#1");
        state.sel = Some(OutlineKey::Turn(1));
        press(&mut state, KeyCode::Char('h'));
        let collapsed = row_with(&snapshot_rows(&mut state), "#1");
        assert!(expanded.contains('⌄') && collapsed.contains('›'), "{expanded}\n{collapsed}");
        assert_eq!(fold_normalised(&collapsed), fold_normalised(&expanded));
    }

    #[test]
    fn event_row_is_identical_collapsed_and_expanded() {
        let mut state = reply_fixture();
        let collapsed = row_with(&snapshot_rows(&mut state), "#2");
        state.sel = Some(OutlineKey::Ev(2));
        press(&mut state, KeyCode::Char('l'));
        let expanded = row_with(&snapshot_rows(&mut state), "#2");
        assert!(collapsed.contains('›') && expanded.contains('⌄'), "{collapsed}\n{expanded}");
        assert!(!collapsed.contains("ASSIST") && !collapsed.ends_with('-'), "chip text and age stay off at 56 cols: {collapsed}");
        assert_eq!(fold_normalised(&collapsed), fold_normalised(&expanded));
    }

    #[test]
    fn dispatch_row_with_child_suffix_is_identical_collapsed_and_expanded() {
        let mut state = dispatch_fixture();
        let collapsed = row_with(&rows_at(&mut state, 90, SNAP_H), "Agent");
        assert!(collapsed.contains("→ Explore index") && collapsed.contains('✓'), "{collapsed}");
        state.sel = Some(OutlineKey::Ev(2));
        press(&mut state, KeyCode::Char('l'));
        let expanded = row_with(&rows_at(&mut state, 90, SNAP_H), "Agent");
        assert_eq!(fold_normalised(&collapsed), fold_normalised(&expanded));
    }

    #[test]
    fn a_prompt_of_three_rows_or_fewer_has_no_more_row() {
        let mut state = opener_turn("Yoooooo please check the commit so the fix landed", false);
        assert!(shown_rows(&state).contains(&"Prompt(1)".to_string()), "{:?}", shown_rows(&state));
        assert!(!shown_rows(&state).iter().any(|r| r.starts_with("PromptMore")), "{:?}", shown_rows(&state));
        assert!(!draw(&mut state, SNAP_W, SNAP_H).contains("more lines"));
    }

    #[test]
    fn selecting_any_prompt_row_selects_the_opener() {
        let mut state = opener_turn(&opener_prompt(), true);
        for key in [OutlineKey::Prompt(1), OutlineKey::PromptMore(1)] {
            state.sel = Some(key);
            assert_eq!(state.selected_event(), Some(0));
        }
        assert_eq!(state.turn_prompt_text(), Some(opener_prompt()));
        state.sel = Some(OutlineKey::PromptMore(1));
        press(&mut state, KeyCode::Char('l'));
        state.sel = Some(OutlineKey::PromptRest(1));
        assert_eq!(state.selected_event(), Some(0));
    }

    #[test]
    fn a_stale_opener_event_key_selects_its_turn() {
        let mut state = opener_turn(&opener_prompt(), true);
        assert!(state.reveal(&OutlineKey::Ev(1)), "the turn stands in for the opener");
        state.sel = Some(OutlineKey::Ev(1));
        state.rebuild_outline();
        assert_eq!(state.sel, Some(OutlineKey::Turn(1)));
    }

    #[test]
    fn a_vanished_prompt_key_selects_its_turn() {
        let mut state = opener_turn(&opener_prompt(), true);
        state.sel = Some(OutlineKey::Prompt(1));
        // The body shrinks to the header: no continuation, so no prompt rows.
        state.bodies.insert(1, "Yoooooo please check".into());
        state.dirty_outline = true;
        draw(&mut state, SNAP_W, SNAP_H);
        assert!(!shown_rows(&state).iter().any(|r| r.starts_with("Prompt")), "{:?}", shown_rows(&state));
        assert_eq!(state.sel, Some(OutlineKey::Turn(1)));
    }

    #[test]
    fn a_prompt_that_appears_on_body_arrival_keeps_the_turn_selected() {
        let mut state = opener_turn("Check the build", false);
        state.goto(0);
        assert!(!shown_rows(&state).iter().any(|r| r.starts_with("Prompt")));
        let grown = "Check the build and then read the logs and report anything that still fails in the nightly job across all shards.";
        assert!(state.settle_body(1, Ok(grown.into())));
        draw(&mut state, SNAP_W, SNAP_H);
        assert!(shown_rows(&state).contains(&"Prompt(1)".to_string()), "{:?}", shown_rows(&state));
        assert_eq!(state.sel, Some(OutlineKey::Turn(1)));
    }

    #[test]
    fn resizing_rebuilds_the_prompt_and_keeps_the_selection() {
        let mut state = opener_turn(&opener_prompt(), true);
        state.goto(0);
        for w in [30u16, 90, 40, SNAP_W] {
            let screen = draw(&mut state, w, SNAP_H);
            assert!(screen.contains("#1"), "{screen}");
            assert_eq!(state.outline_w, w as usize - 14);
            assert_eq!(state.sel, Some(OutlineKey::Turn(1)));
        }
    }

    #[test]
    fn narrow_rows_drop_the_counts_before_the_header() {
        // Text cell 12: the counts would leave 1 cell for the header, under MIN_HEAD_W, so they go.
        let mut state = reply_fixture();
        let rows = rows_at(&mut state, 26, SNAP_H);
        let turn = row_with(&rows, "#1");
        assert!(turn.contains("Check") && !turn.contains('·'), "{turn}");
        // Text cell 20: 9 cells of counts leave 9 for the header, so they stay.
        let rows = rows_at(&mut state, 34, SNAP_H);
        let turn = row_with(&rows, "#1");
        assert!(turn.contains("Check") && turn.contains("· 1 msg"), "{turn}");
        // Body expanded at the narrow width: no panic, header still shown.
        state.sel = Some(OutlineKey::Ev(2));
        press(&mut state, KeyCode::Char('l'));
        let rows = rows_at(&mut state, 26, SNAP_H);
        assert!(!row_with(&rows, "#2").trim().is_empty());
    }

    #[test]
    fn status_line_expires_after_four_seconds() {
        let mut s = TracesModalState::open_with_hosts(None, vec![]);
        s.set_status("indexed 3 changed files");
        s.poll_live_at(0);
        assert_eq!(s.status.as_deref(), Some("indexed 3 changed files"));
        s.status_at = Some(Instant::now() - Duration::from_secs(5));
        s.poll_live_at(0);
        assert!(s.status.is_none());
        assert!(s.status_at.is_none());
    }

    fn mouse_at(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE }
    }

    fn click_at(state: &mut TracesModalState, column: u16, row: u16) -> InputOutcome {
        handle_traces_mouse(state, &mouse_at(MouseEventKind::Down(MouseButton::Left), column, row))
    }

    fn ledger_rows(n: usize) -> TracesModalState {
        let mut state = TracesModalState::open_with_hosts(None, vec![]);
        state.clear_status();
        state.sessions = (0..n)
            .map(|i| SessionRow { id: format!("s{i}"), title: format!("task {i}"), ..row("claude", "/h/x.jsonl") })
            .collect();
        state.rebuild_ledger();
        state
    }

    fn paint_ledger(state: &mut TracesModalState, w: u16, h: u16) -> Buffer {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        render_ledger(&mut buf, area, state, &Theme::current());
        buf
    }

    fn paint_trajectory(state: &mut TracesModalState, w: u16, h: u16) -> Buffer {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        render_trajectory(&mut buf, area, state, &Theme::current());
        buf
    }

    fn find_glyph(buf: &Buffer, glyph: &str) -> Option<(u16, u16)> {
        let area = buf.area;
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                if buf[(x, y)].symbol() == glyph {
                    return Some((x, y));
                }
            }
        }
        None
    }

    #[test]
    fn row_at_y_walks_heights_gaps_and_scroll() {
        // 2-line row, a height-0 row, then a 1-line row whose pre-spacer sits on y=12.
        let heights = [2, 0, 1];
        let gaps = [(0, 0), (0, 0), (1, 0)];
        let hit = row_at_y(11, 10, 20, 0, &heights, &gaps).unwrap();
        assert_eq!((hit.index, hit.top), (0, 10), "lower line of a tall row stays that row");
        assert!(row_at_y(12, 10, 20, 0, &heights, &gaps).is_none(), "spacer is not a row");
        assert_eq!(row_at_y(13, 10, 20, 0, &heights, &gaps).unwrap().index, 2);
        // Scrolled in: the first drawn row's pre-spacer is not painted, so y=10 is row 2.
        assert_eq!(row_at_y(10, 10, 20, 2, &[1, 1, 1], &[(0, 0), (0, 0), (1, 0)]).unwrap().index, 2);
        assert!(row_at_y(9, 10, 20, 0, &heights, &gaps).is_none());
        assert!(row_at_y(20, 10, 20, 0, &heights, &gaps).is_none());
    }

    #[test]
    fn mouse_click_selects_a_ledger_row_and_second_click_opens_it() {
        let mut state = ledger_rows(6);
        let _ = paint_ledger(&mut state, 80, 12);
        // Header is row 2; the first session is row 3. Row 5 is session s2.
        assert!(matches!(click_at(&mut state, 40, 2), InputOutcome::Unchanged), "header is not a row");
        assert!(matches!(click_at(&mut state, 90, 5), InputOutcome::Unchanged), "outside the list");
        assert_eq!(state.selected, 0);
        assert!(matches!(click_at(&mut state, 40, 5), InputOutcome::Changed));
        assert_eq!(state.selected, 2);
        assert_eq!(state.mode, Mode::List);
        assert!(matches!(click_at(&mut state, 40, 5), InputOutcome::Changed));
        assert_eq!(state.mode, Mode::Trajectory);
        assert_eq!(state.session.as_ref().map(|s| s.id.as_str()), Some("s2"));
    }

    #[test]
    fn mouse_wheel_moves_ledger_selection_by_three_and_scrolls() {
        let mut state = ledger_rows(20);
        let _ = paint_ledger(&mut state, 80, 8);
        // List height is 5 (rows start at y=3). Selection 0 is already at the top.
        let up = handle_traces_mouse(&mut state, &mouse_at(MouseEventKind::ScrollUp, 10, 4));
        assert!(matches!(up, InputOutcome::Unchanged));
        assert_eq!(state.selected, 0);
        assert!(matches!(handle_traces_mouse(&mut state, &mouse_at(MouseEventKind::ScrollDown, 10, 4)), InputOutcome::Changed));
        assert_eq!((state.selected, state.scroll), (3, 0));
        assert!(matches!(handle_traces_mouse(&mut state, &mouse_at(MouseEventKind::ScrollDown, 10, 4)), InputOutcome::Changed));
        assert_eq!((state.selected, state.scroll), (6, 2));
    }

    #[test]
    fn mouse_fold_click_toggles_that_ledger_row() {
        let mut state = tree_state(&[("r", None), ("c1", Some("r")), ("g1", Some("c1")), ("gg1", Some("g1")), ("c2", Some("r"))]);
        state.clear_status();
        assert_eq!(shown(&state), ["r", "c1", "c2"]);
        let buf = paint_ledger(&mut state, 140, 20);
        let (x, y) = find_glyph(&buf, crate::glyphs::chevron()).expect("collapsed fold glyph");
        assert!(matches!(click_at(&mut state, x, y), InputOutcome::Changed));
        assert_eq!(shown(&state), ["r", "c1", "g1", "c2"], "the triangle toggles, it does not open");
        assert_eq!(state.mode, Mode::List);
        assert_eq!(state.selected_session().map(|s| s.id.as_str()), Some("c1"));
    }

    #[test]
    fn mouse_trajectory_selects_scrolls_and_toggles_a_fold() {
        let mut state = user_turns(&["a", "b", "c", "d", "e", "f", "g", "h"]);
        state.clear_status();
        let _ = paint_trajectory(&mut state, 100, 8);
        // One-row compact header: the list starts at y=1 and is 7 lines tall. Row 1 is "b".
        assert!(matches!(click_at(&mut state, 40, 2), InputOutcome::Changed));
        assert_eq!(state.sel_pos(), 1);
        assert!(matches!(handle_traces_mouse(&mut state, &mouse_at(MouseEventKind::ScrollDown, 40, 6)), InputOutcome::Changed));
        assert_eq!(state.sel_pos(), 4);
        assert_eq!(state.event_scroll, 0, "every row still fits the 7-line viewport");

        let mut state = trajectory(three_calls());
        state.clear_status();
        let buf = paint_trajectory(&mut state, 100, 24);
        let (x, y) = find_glyph(&buf, crate::glyphs::chevron_down()).expect("open fold glyph");
        let before = shown_rows(&state);
        assert!(before.len() > 1, "{before:?}");
        assert!(matches!(click_at(&mut state, x, y), InputOutcome::Changed));
        assert_eq!(shown_rows(&state), ["Turn(1)"]);
        assert_eq!(state.mode, Mode::Trajectory, "a fold click is not Enter");
    }

    #[test]
    fn mouse_inspector_ignores_clicks_and_the_wheel() {
        let mut state = trajectory(three_calls());
        state.mode = Mode::Inspector;
        let sel = state.sel.clone();
        let scroll = state.event_scroll;
        assert!(matches!(click_at(&mut state, 14, 5), InputOutcome::Unchanged));
        assert!(matches!(handle_traces_mouse(&mut state, &mouse_at(MouseEventKind::ScrollDown, 14, 5)), InputOutcome::Unchanged));
        assert_eq!(state.mode, Mode::Inspector);
        assert_eq!(state.sel, sel);
        assert_eq!(state.event_scroll, scroll);
    }

    /// An inspector on a Bash call with script `script` and result `out`; `raw` is a fake source line.
    fn payload_state(script: &str, out: &str) -> TracesModalState {
        let mut state = trajectory(three_calls());
        let own = Ok(Some(format!("{{\"input\":{{\"command\":{script:?}}}}}")));
        let res = Some((Ok(Some(format!("{{\"output\":{out:?}}}"))), false));
        state.payload.open(Some(compose_body(own, res)), Shape::Call);
        state.raw = Some("{\n  \"parentId\": \"p1\"\n}".into());
        let line = state.selected_event().map(|i| state.events[i].line).unwrap_or(1);
        state.raw_lines.insert(line, "RAWLINE".into());
        state.mode = Mode::Inspector;
        state
    }

    #[test]
    fn inspector_opens_on_the_payload_and_m_flips_to_json_and_back() {
        let mut state = payload_state("python3 - <<EOF\nprint(1)\nEOF", "1");
        let rows = rows_at(&mut state, 100, 24).join("\n");
        assert!(rows.contains("1 │ python3 - <<EOF") && rows.contains("3 │ EOF") && rows.contains(RESULT_SEP), "{rows}");
        assert!(!rows.contains("parentId"), "{rows}");
        press(&mut state, KeyCode::Char('m'));
        let rows = rows_at(&mut state, 100, 24).join("\n");
        assert!(rows.contains("parentId") && rows.contains("line "), "{rows}");
        assert!(!rows.contains("python3 -"), "{rows}");
        press(&mut state, KeyCode::Esc);
        assert_eq!(state.mode, Mode::Inspector, "Esc leaves metadata first");
        assert!(!state.payload.meta);
        press(&mut state, KeyCode::Esc);
        assert_eq!(state.mode, Mode::Trajectory);
    }

    #[test]
    fn w_toggles_wrap_in_both_views() {
        let long = "x".repeat(200);
        let mut state = payload_state(&long, "r");
        let count = |rows: &[String]| rows.iter().filter(|r| r.contains("xxxx")).count();
        assert!(count(&rows_at(&mut state, 80, 24)) > 1, "payload wraps by default");
        press(&mut state, KeyCode::Char('w'));
        assert_eq!(count(&rows_at(&mut state, 80, 24)), 1, "w truncates");
        state.raw = Some(format!("{{\"k\": \"{long}\"}}"));
        press(&mut state, KeyCode::Char('m'));
        assert_eq!(count(&rows_at(&mut state, 80, 24)), 1, "metadata honours the same toggle");
        press(&mut state, KeyCode::Char('w'));
        assert!(count(&rows_at(&mut state, 80, 24)) > 1, "metadata wraps when w is on");
    }

    #[test]
    fn y_copies_the_command_and_capital_y_the_raw_line() {
        let mut state = payload_state("echo a\necho b", "out");
        let copied = |o: InputOutcome| match o {
            InputOutcome::Action(Action::TracesCopy { text }) => text,
            _ => panic!("no copy action"),
        };
        let y = handle_traces_key(&mut state, &KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert_eq!(copied(y), "echo a\necho b", "no margin, no separator, no result");
        let upper = handle_traces_key(&mut state, &KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::SHIFT));
        assert_eq!(copied(upper), "RAWLINE");
    }

    #[test]
    fn inspector_scrolls_by_page_and_to_the_ends() {
        let out = (1..=100).map(|i| format!("l{i}")).collect::<Vec<_>>().join("\n");
        let mut state = payload_state("c", &out);
        rows_at(&mut state, 80, 24);
        handle_traces_key(&mut state, &KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert_eq!(state.raw_scroll, 10);
        handle_traces_key(&mut state, &KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(state.raw_scroll, 0);
        press(&mut state, KeyCode::Char('G'));
        assert!(rows_at(&mut state, 80, 24).iter().any(|r| r.contains("l100")), "G reaches the last line");
        press(&mut state, KeyCode::Char('g'));
        assert_eq!(state.raw_scroll, 0);
    }

    #[test]
    fn inspector_hints_list_the_keys_that_work() {
        let mut state = payload_state("c", "o");
        let hints = hint_labels(&state, 120);
        for want in ["j/k scroll", "g/G ends", "w wrap", "m metadata", "y copy", "Y copy json", "Esc back"] {
            assert!(hints.contains(&want), "{hints:?}");
        }
        assert!(!hints.contains(&"r retry"));
        state.payload.open(Some(Err("io".into())), Shape::Call);
        assert!(hint_labels(&state, 120).contains(&"r retry"));
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    fn lr(
        id: &str,
        source: &str,
        title: &str,
        state: LiveState,
        text: &str,
        age_s: i64,
    ) -> LiveRow {
        LiveRow {
            session_id: id.into(),
            source: source.into(),
            title: title.into(),
            state,
            text: text.into(),
            age_ms: age_s * 1000,
            ..Default::default()
        }
    }

    fn kid(
        parent: &str,
        id: &str,
        label: &str,
        state: LiveState,
        text: &str,
        age_s: i64,
    ) -> LiveRow {
        LiveRow {
            parent_id: Some(parent.into()),
            label: Some(label.into()),
            ..lr(id, "claude", label, state, text, age_s)
        }
    }

    fn state() -> TracesModalState {
        let mut s = TracesModalState::open_with_hosts(None, vec![]);
        s.mode = Mode::Live;
        s.local_host = "sab-mini".into();
        s.clear_status();
        s
    }

    fn rows_of(buf: &Buffer, w: u16, h: u16) -> Vec<String> {
        (0..h)
            .map(|y| {
                let mut line = String::new();
                let mut x = 0;
                while x < w {
                    let sym = buf[(x, y)].symbol();
                    x += sym.width().max(1) as u16;
                    line.push_str(sym);
                }
                line.trim_end().to_string()
            })
            .collect()
    }

    fn snap(s: &mut TracesModalState, w: u16, h: u16, now: Instant) -> Vec<String> {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        render_live_at(&mut buf, area, s, &Theme::current(), now);
        rows_of(&buf, w, h)
    }

    fn states() -> Vec<LiveRow> {
        let mut v = vec![
            lr(
                "w",
                "claude",
                "Traces packet on Mini",
                LiveState::Waiting,
                "permission: Bash cargo build --release",
                120,
            ),
            lr(
                "p",
                "claude",
                "snapshot tests",
                LiveState::Prompt,
                "your turn",
                60,
            ),
            lr(
                "t",
                "cursor",
                "grok-build",
                LiveState::Tool,
                "Shell cargo test -p xai-grok-traces",
                8,
            ),
            lr("c", "claude", "compactor", LiveState::Compacting, "", 5),
            lr(
                "k",
                "pi",
                "khoj worker",
                LiveState::Working,
                "Bash rg --files",
                1,
            ),
            lr(
                "q",
                "claude",
                "fork lead",
                LiveState::Working,
                "Edit src/views/traces_modal.rs",
                240,
            ),
            lr(
                "f",
                "claude",
                "failed one",
                LiveState::Working,
                "Read x",
                30,
            ),
            lr(
                "u",
                "claude",
                "stuck tool",
                LiveState::Tool,
                "Bash sleep",
                400,
            ),
            lr("d", "pi", "no pid", LiveState::Working, "Read y", 50),
            lr(
                "ww",
                "claude",
                "long wait",
                LiveState::Waiting,
                "permission: Edit a",
                500,
            ),
        ];
        v[0].since_ms = Some(5_000);
        v[2].since_ms = Some(8_000);
        v[2].context_tokens = Some(142_300);
        v[2].context_window = Some(200_000);
        v[3].since_ms = Some(5_000);
        v[5].quiet_ms = 240_000;
        v[5].context_tokens = Some(142_000);
        v[6].failed = true;
        v[7].since_ms = Some(400_000);
        v[7].quiet_ms = 400_000;
        v[8].pid_unknown = true;
        v[9].since_ms = Some(500_000);
        v[9].quiet_ms = 10_000;
        v
    }

    fn idx(tmp: &tempfile::TempDir) -> (TraceIndex, TraceIndex) {
        let db = tmp.path().join("idx/t.sqlite");
        (
            TraceIndex::open(&db).unwrap(),
            TraceIndex::open(&db).unwrap(),
        )
    }

    fn receipt(writer: &mut TraceIndex) {
        let r = xai_grok_traces::Receipt {
            source: "claude".into(),
            event: "UserPromptSubmit".into(),
            ..Default::default()
        };
        writer.record_receipt(&r).unwrap();
    }

    fn with_index(index: Option<TraceIndex>) -> TracesModalState {
        let mut s = TracesModalState::open_with_hosts(index, vec![]);
        s.mode = Mode::Live;
        s.local_host = "sab-mini".into();
        s.clear_status();
        s
    }

    fn family() -> Vec<LiveRow> {
        vec![
            lr(
                "r",
                "claude",
                "Traces packet on Mini",
                LiveState::Working,
                "Read docs/SPEC.md",
                4,
            ),
            kid(
                "r",
                "k1",
                "Explore index code",
                LiveState::Waiting,
                "permission: Bash rg x",
                3,
            ),
            kid("r", "k2", "review spec", LiveState::Working, "Grep live", 9),
            lr("s", "claude", "other", LiveState::Prompt, "your turn", 70),
        ]
    }

    fn remote_rows() -> Vec<LiveRow> {
        let mk =
            |id: &str, source: &str, title: &str, st: LiveState, text: &str, age: i64| LiveRow {
                host: Some("sab-air".into()),
                ..lr(&format!("sab-air:{id}"), source, title, st, text, age)
            };
        let mut waiting = mk(
            "a",
            "claude",
            "fork lead",
            LiveState::Waiting,
            "permission: Bash cargo build",
            5,
        );
        waiting.since_ms = Some(5_000);
        let mut tool = mk(
            "b",
            "pi",
            "khoj worker",
            LiveState::Tool,
            "Bash rg --files",
            2,
        );
        tool.since_ms = Some(25_000);
        vec![
            waiting,
            tool,
            mk(
                "c",
                "claude",
                "docs sweep",
                LiveState::Working,
                "Read README.md",
                40,
            ),
        ]
    }

    fn scenarios() -> Vec<(&'static str, Vec<String>)> {
        let t0 = Instant::now();
        let ago = |s: u64| t0 - Duration::from_secs(s);
        let mut out = Vec::new();
        let mut s = state();
        s.set_live(states(), vec![]);
        out.push(("w120", snap(&mut s, 120, 11, t0)));
        out.push(("w80", snap(&mut s, 80, 11, t0)));
        out.push(("w50", snap(&mut s, 50, 11, t0)));
        out.push(("w36", snap(&mut s, 36, 4, t0)));

        let mut s = state();
        s.set_live(family(), vec![]);
        out.push(("children expanded, waiting child", snap(&mut s, 100, 6, t0)));
        s.live_collapsed.insert("r".into());
        out.push((
            "children collapsed (+2, root shows warn)",
            snap(&mut s, 100, 6, t0),
        ));
        s.live_collapsed.clear();
        s.live_sel = Some("k1".into());
        out.push(("child selected", snap(&mut s, 100, 6, t0)));

        let mut s = state();
        let mut parent = lr("p1", "claude", "bg parent", LiveState::Working, "Read x", 3);
        parent.bg = 2;
        parent.context_tokens = Some(90_400);
        let mut idle = lr(
            "p2",
            "claude",
            "idle parent",
            LiveState::Prompt,
            "your turn",
            20,
        );
        idle.bg = 1;
        s.set_live(vec![parent, idle], vec![]);
        out.push(("bg parents", snap(&mut s, 100, 4, t0)));

        let mut s = state();
        let mut rcpt = lr(
            "cursor:rcpt:abc",
            "cursor",
            "grok-build",
            LiveState::Working,
            "Shell ls",
            2,
        );
        rcpt.context_tokens = None;
        let mut chat = lr(
            "cur2",
            "cursor",
            "chat with transcript",
            LiveState::Working,
            "Edit a.rs",
            6,
        );
        chat.context_tokens = Some(33_000);
        chat.context_window = Some(128_000);
        s.set_live(vec![rcpt, chat], vec![]);
        out.push(("cursor receipt-only and with ctx", snap(&mut s, 100, 4, t0)));

        let mut s = state();
        s.set_live(
            vec![lr(
                "l1",
                "claude",
                "local one",
                LiveState::Working,
                "Read a",
                20,
            )],
            vec![("sab@sab-air", remote_rows(), ago(0), true, None)],
        );
        out.push(("remote ok, interleaved", snap(&mut s, 100, 6, t0)));
        s.live_sel = Some("sab-air:b".into());
        out.push(("remote selected", snap(&mut s, 100, 6, t0)));

        let mut s = state();
        s.set_live(
            vec![lr(
                "l1",
                "claude",
                "local one",
                LiveState::Working,
                "Read a",
                20,
            )],
            vec![("sab@sab-air", remote_rows(), ago(3), true, None)],
        );
        out.push((
            "remote fresh fetch 3s ago: waiting first, age 8s",
            snap(&mut s, 100, 6, t0),
        ));

        let mut s = state();
        s.set_live(
            vec![lr(
                "l1",
                "claude",
                "local one",
                LiveState::Working,
                "Read a",
                20,
            )],
            vec![("sab@sab-air", remote_rows(), ago(8), false, None)],
        );
        out.push((
            "remote failed, cached rows advanced 8s",
            snap(&mut s, 100, 6, t0),
        ));

        let mut s = state();
        s.set_live(
            vec![lr(
                "l1",
                "claude",
                "local one",
                LiveState::Working,
                "Read a",
                20,
            )],
            vec![(
                "sab@sab-air",
                vec![],
                ago(0),
                true,
                Some("sab-air: update grok-traces (live_schema none)"),
            )],
        );
        out.push(("remote gated note", snap(&mut s, 100, 5, t0)));

        let mut s = state();
        s.set_live(
            vec![lr(
                "l1",
                "claude",
                "local one",
                LiveState::Working,
                "Read a",
                20,
            )],
            vec![],
        );
        s.hosts = vec!["sab@sab-air".into()];
        out.push(("host before first result", snap(&mut s, 100, 3, t0)));

        let mut s = state();
        s.set_live(
            vec![lr(
                "o",
                "claude",
                "child of gone",
                LiveState::Working,
                "Read a",
                5,
            )],
            vec![],
        );
        s.live_rows[0].parent_id = Some("gone".into());
        s.live_titles
            .insert("gone".into(), "the parent title".into());
        out.push(("orphan", snap(&mut s, 100, 3, t0)));

        let mut s = state();
        s.set_live(states(), vec![]);
        s.live_query = "BASH".into();
        out.push(("filter bash", snap(&mut s, 100, 6, t0)));
        s.live_query = "zzz".into();
        out.push(("no match", snap(&mut s, 100, 3, t0)));
        s.live_query.clear();
        s.typing = Typing::Live;
        s.live_query = "fo".into();
        out.push(("typing line", snap(&mut s, 100, 8, t0)));

        let mut s = state();
        s.standalone = true;
        s.set_live(
            vec![lr(
                "l1",
                "claude",
                "local one",
                LiveState::Working,
                "Read a",
                20,
            )],
            vec![],
        );
        out.push(("standalone hints", snap(&mut s, 100, 4, t0)));

        let mut s = state();
        let mut bb = lr("v1", "claude", "docs sweep", LiveState::Working, "Read a", 4);
        bb.via = Some("bb".into());
        let mut herdr = lr(
            "v2",
            "claude",
            "a title that is much too long for the cell",
            LiveState::Working,
            "Read b",
            5,
        );
        herdr.via = Some("herdr".into());
        let mut cur = lr("v3", "cursor", "grok-build", LiveState::Working, "Shell ls", 6);
        cur.via = Some("cursor".into());
        s.set_live(vec![bb, herdr, cur], vec![]);
        out.push(("via bb, herdr; none on cursor", snap(&mut s, 100, 5, t0)));

        let mut s = state();
        s.set_live(
            vec![lr(
                "a",
                "claude",
                "日本語のタイトルがとても長い",
                LiveState::Tool,
                "Shell echo ünïcode",
                3,
            )],
            vec![],
        );
        s.live_rows[0].since_ms = Some(40_000);
        out.push(("wide chars align", snap(&mut s, 80, 3, t0)));

        let mut s = state();
        s.standalone = true;
        s.set_live(family(), vec![]);
        out.push(("live expanded hjkl", snap(&mut s, 100, 6, t0)));
        s.live_sel = Some("r".into());
        press(&mut s, KeyCode::Char('h'));
        out.push(("live after h", snap(&mut s, 100, 6, t0)));
        out
    }

    fn frozen() -> Vec<(&'static str, Vec<&'static str>)> {
        vec![
            (
                "w120",
                vec![
                    r#" grok live · 10 agents · 2 waiting                                                                           sab-mini ●"#,
                    r#" ⚠  claude  Traces packet on Mini                      permission: Bash cargo build --release                    2m"#,
                    r#" ⚠? claude  long wait                                  permission: Edit a                                        8m"#,
                    r#" ○  claude  snapshot tests                             your turn                                                 1m"#,
                    r#" ●  pi      khoj worker                                Bash rg --files                                           1s"#,
                    r#" ⟳  claude  compactor                                  compacting · 5s                                           5s"#,
                    r#" ▶  cursor  grok-build                   ctx 142k/200k Shell cargo test -p xai-grok-traces · 8s                  8s"#,
                    r#" ●  claude  failed one                                 Read x                                                   30s ✗"#,
                    r#" ●  pi      no pid                                     Read y                                                   50s pid?"#,
                    r#" ◌  claude  fork lead                    ctx 142k      quiet · Edit src/views/traces_modal.rs                    4m"#,
                    r#" ⏳ claude  stuck tool                                 Bash sleep · 6m                                           6m ?"#,
                ],
            ),
            (
                "w80",
                vec![
                    r#" grok live · 10 agents · 2 waiting                                   sab-mini ●"#,
                    r#" ⚠  claude  Traces packet on…    permission: Bash cargo build…           2m"#,
                    r#" ⚠? claude  long wait            permission: Edit a                      8m"#,
                    r#" ○  claude  snapshot tests       your turn                               1m"#,
                    r#" ●  pi      khoj worker          Bash rg --files                         1s"#,
                    r#" ⟳  claude  compactor            compacting · 5s                         5s"#,
                    r#" ▶  cursor  grok-build           Shell cargo test -p xai-grok-traces…    8s"#,
                    r#" ●  claude  failed one           Read x                                 30s ✗"#,
                    r#" ●  pi      no pid               Read y                                 50s pid?"#,
                    r#" ◌  claude  fork lead            quiet · Edit…                           4m"#,
                    r#" ⏳ claude  stuck tool           Bash sleep · 6m                         6m ?"#,
                ],
            ),
            (
                "w50",
                vec![
                    r#" grok live · 10 agents · 2 waiting     sab-mini ●"#,
                    r#" ⚠  claude  Traces packet… permission:…    2m"#,
                    r#" ⚠? claude  long wait      permission:…    8m"#,
                    r#" ○  claude  snapshot tests your turn       1m"#,
                    r#" ●  pi      khoj worker    Bash rg…        1s"#,
                    r#" ⟳  claude  compactor      compacting ·…   5s"#,
                    r#" ▶  cursor  grok-build     Shell cargo…    8s"#,
                    r#" ●  claude  failed one     Read x         30s ✗"#,
                    r#" ●  pi      no pid         Read y         50s pid?"#,
                    r#" ◌  claude  fork lead      quiet · Edit…   4m"#,
                    r#" ⏳ claude  stuck tool     Bash sleep ·…   6m ?"#,
                ],
            ),
            (
                "w36",
                vec![
                    r#" grok live · 10 agents…  sab-mini ●"#,
                    r#" ⚠  Traces packet… permis…   2m"#,
                    r#" ⚠? long wait      permis…   8m"#,
                    r#" ○  snapshot tests your…     1m"#,
                ],
            ),
            (
                "children expanded, waiting child",
                vec![
                    r#" grok live · 4 agents · 1 waiting                                                        sab-mini ●"#,
                    r#" ○  claude  other                                      your turn                             1m"#,
                    r#" ●  claude  Traces packet on Mini                      Read docs/SPEC.md                     4s"#,
                    r#"   ⚠  claude  Explore index code                       permission: Bash rg x                 3s"#,
                    r#"   ●  claude  review spec                              Grep live                             9s"#,
                    r#""#,
                ],
            ),
            (
                "children collapsed (+2, root shows warn)",
                vec![
                    r#" grok live · 4 agents · 1 waiting                                                        sab-mini ●"#,
                    r#" ○  claude  other                                      your turn                             1m"#,
                    r#" ⚠  claude  Traces packet on Mini +2                   Read docs/SPEC.md                     4s"#,
                    r#""#,
                    r#""#,
                    r#""#,
                ],
            ),
            (
                "child selected",
                vec![
                    r#" grok live · 4 agents · 1 waiting                                                        sab-mini ●"#,
                    r#" ○  claude  other                                      your turn                             1m"#,
                    r#" ●  claude  Traces packet on Mini                      Read docs/SPEC.md                     4s"#,
                    r#"   ⚠  claude  Explore index code                       permission: Bash rg x                 3s"#,
                    r#"   ●  claude  review spec                              Grep live                             9s"#,
                    r#""#,
                ],
            ),
            (
                "bg parents",
                vec![
                    r#" grok live · 2 agents                                                                    sab-mini ●"#,
                    r#" ○  claude  idle parent                                your turn · 1 bg                     20s"#,
                    r#" ●  claude  bg parent                    ctx 90k       2 bg · Read x                         3s"#,
                    r#""#,
                ],
            ),
            (
                "cursor receipt-only and with ctx",
                vec![
                    r#" grok live · 2 agents                                                                    sab-mini ●"#,
                    r#" ●  cursor  grok-build                                 Shell ls                              2s"#,
                    r#" ●  cursor  chat with transcript         ctx 33k/128k  Edit a.rs                             6s"#,
                    r#""#,
                ],
            ),
            (
                "remote ok, interleaved",
                vec![
                    r#" grok live · 4 agents · 1 waiting                                              sab-mini ● sab-air ●"#,
                    r#" ⚠  claude  sab-air: fork lead                         permission: Bash cargo build          5s"#,
                    r#" ▶  pi      sab-air: khoj worker                       Bash rg --files · 25s                 2s"#,
                    r#" ●  claude  local one                                  Read a                               20s"#,
                    r#" ●  claude  sab-air: docs sweep                        Read README.md                       40s"#,
                    r#""#,
                ],
            ),
            (
                "remote selected",
                vec![
                    r#" grok live · 4 agents · 1 waiting                                              sab-mini ● sab-air ●"#,
                    r#" ⚠  claude  sab-air: fork lead                         permission: Bash cargo build          5s"#,
                    r#" ▶  pi      sab-air: khoj worker                       Bash rg --files · 25s                 2s"#,
                    r#" ●  claude  local one                                  Read a                               20s"#,
                    r#" ●  claude  sab-air: docs sweep                        Read README.md                       40s"#,
                    r#""#,
                ],
            ),
            (
                "remote fresh fetch 3s ago: waiting first, age 8s",
                vec![
                    r#" grok live · 4 agents · 1 waiting                                              sab-mini ● sab-air ●"#,
                    r#" ⚠  claude  sab-air: fork lead                         permission: Bash cargo build          8s"#,
                    r#" ▶  pi      sab-air: khoj worker                       Bash rg --files · 28s                 5s"#,
                    r#" ●  claude  local one                                  Read a                               20s"#,
                    r#" ●  claude  sab-air: docs sweep                        Read README.md                       43s"#,
                    r#""#,
                ],
            ),
            (
                "remote failed, cached rows advanced 8s",
                vec![
                    r#" grok live · 4 agents · 1 waiting                                              sab-mini ● sab-air ✗"#,
                    r#" ⚠  claude  sab-air: fork lead                         permission: Bash cargo build         13s"#,
                    r#" ⏳ pi      sab-air: khoj worker                       Bash rg --files · 33s                10s"#,
                    r#" ●  claude  local one                                  Read a                               20s"#,
                    r#" ●  claude  sab-air: docs sweep                        Read README.md                       48s"#,
                    r#""#,
                ],
            ),
            (
                "remote gated note",
                vec![
                    r#" grok live · 1 agent                                                           sab-mini ● sab-air ●"#,
                    r#"sab-air: update grok-traces (live_schema none)"#,
                    r#" ●  claude  local one                                  Read a                               20s"#,
                    r#""#,
                    r#""#,
                ],
            ),
            (
                "host before first result",
                vec![
                    r#" grok live · 1 agent                                                           sab-mini ● sab-air …"#,
                    r#" ●  claude  local one                                  Read a                               20s"#,
                    r#""#,
                ],
            ),
            (
                "orphan",
                vec![
                    r#" grok live · 1 agent                                                                     sab-mini ●"#,
                    r#" ●  claude  ↑ the parent title                         Read a                                5s"#,
                    r#""#,
                ],
            ),
            (
                "filter bash",
                vec![
                    r#" grok live · 3 agents · 1 waiting                                                        sab-mini ●"#,
                    r#" ⚠  claude  Traces packet on Mini                      permission: Bash cargo build…         2m"#,
                    r#" ●  pi      khoj worker                                Bash rg --files                       1s"#,
                    r#" ⏳ claude  stuck tool                                 Bash sleep · 6m                       6m ?"#,
                    r#""#,
                    r#" /BASH   esc clear"#,
                ],
            ),
            (
                "no match",
                vec![
                    r#" grok live · 0 agents                                                                    sab-mini ●"#,
                    r#" no match"#,
                    r#" /zzz   esc clear"#,
                ],
            ),
            (
                "typing line",
                vec![
                    r#" grok live · 1 agent                                                                     sab-mini ●"#,
                    r#" ◌  claude  fork lead                    ctx 142k      quiet · Edit…                         4m"#,
                    r#""#,
                    r#""#,
                    r#""#,
                    r#""#,
                    r#""#,
                    r#" /fo▏"#,
                ],
            ),
            (
                "standalone hints",
                vec![
                    r#" grok live · 1 agent                                                                     sab-mini ●"#,
                    r#" ●  claude  local one                                  Read a                               20s"#,
                    r#""#,
                    r#" enter open   j/k   h/l fold   / search   q quit"#,
                ],
            ),
            (
                "via bb, herdr; none on cursor",
                vec![
                    r#" grok live · 3 agents                                                                    sab-mini ●"#,
                    r#" ●  claude  docs sweep via bb                          Read a                                4s"#,
                    r#" ●  claude  a title that is… via herdr                 Read b                                5s"#,
                    r#" ●  cursor  grok-build                                 Shell ls                              6s"#,
                    r#""#,
                ],
            ),
            (
                "wide chars align",
                vec![
                    r#" grok live · 1 agent                                                 sab-mini ●"#,
                    r#" ⏳ claude  日本語のタイトルが…  Shell echo ünïcode · 40s                3s"#,
                    r#""#,
                ],
            ),
            (
                "live expanded hjkl",
                vec![
                    r#" grok live · 4 agents · 1 waiting                                                        sab-mini ●"#,
                    r#" ○  claude  other                                      your turn                             1m"#,
                    r#" ●  claude  Traces packet on Mini                      Read docs/SPEC.md                     4s"#,
                    r#"   ⚠  claude  Explore index code                       permission: Bash rg x                 3s"#,
                    r#"   ●  claude  review spec                              Grep live                             9s"#,
                    r#" enter open   j/k   h/l fold   / search   q quit"#,
                ],
            ),
            (
                "live after h",
                vec![
                    r#" grok live · 4 agents · 1 waiting                                                        sab-mini ●"#,
                    r#" ○  claude  other                                      your turn                             1m"#,
                    r#" ⚠  claude  Traces packet on Mini +2                   Read docs/SPEC.md                     4s"#,
                    r#""#,
                    r#""#,
                    r#" enter open   j/k   h/l fold   / search   q quit"#,
                ],
            ),
        ]
    }

    #[test]
    #[ignore = "regenerates the frozen snapshots: cargo test live_tests::dump -- --ignored --nocapture"]
    fn dump() {
        for (name, rows) in scenarios() {
            eprintln!("@@ {name}");
            for r in rows {
                eprintln!("|{r}");
            }
        }
    }

    #[test]
    fn snapshots_match() {
        let got = scenarios();
        let want = frozen();
        assert_eq!(got.len(), want.len());
        for ((gn, g), (wn, w)) in got.iter().zip(&want) {
            assert_eq!(gn, wn);
            assert_eq!(
                g.iter().map(String::as_str).collect::<Vec<_>>(),
                *w,
                "snapshot {gn}"
            );
        }
    }

    fn press(s: &mut TracesModalState, code: KeyCode) -> InputOutcome {
        handle_traces_key(s, &KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn ids(s: &TracesModalState) -> Vec<String> {
        s.live_view(Instant::now())
            .lines
            .iter()
            .map(|l| l.row.session_id.clone())
            .collect()
    }

    #[test]
    fn fit_is_exact_width_and_never_panics() {
        assert_eq!(fit("", 5), "     ");
        assert_eq!(fit("abc", 0), "");
        assert_eq!(fit("abc", 5), "abc  ");
        assert_eq!(fit("hello world foo", 8), "hello…  ");
        assert_eq!(fit("averyveryverylongword", 8), "averyve…");
        assert_eq!(fit("日本語のタイトル", 7), "日本語…");
        assert_eq!(fit("日本語のタイトル", 8).width(), 8);
        assert_eq!(fit("abcdef", 1), "…");
        assert_eq!(cut("✗ ? pid?", 4), "✗ ?…");
        assert_eq!(cut("✗", 4), "✗   ");
    }

    #[test]
    fn empty_states() {
        let t0 = Instant::now();
        let tmp = tempfile::tempdir().unwrap();
        let (mut writer, reader) = idx(&tmp);
        let mut s = with_index(Some(reader));
        assert_eq!(snap(&mut s, 100, 4, t0)[1], " no live agents", "before the first fold");
        s.fold_now(now_ms());
        let rows = snap(&mut s, 100, 4, t0);
        assert_eq!(
            rows[1],
            " no live agents · 0 hook receipts in the last hour · grok-traces doctor"
        );
        assert_eq!(
            rows[2],
            " install hooks: crates/codegen/xai-grok-traces/hooks/install.sh --apply"
        );
        receipt(&mut writer);
        s.fold_now(now_ms());
        let rows = snap(&mut s, 100, 4, t0);
        assert_eq!(
            rows[1],
            " no live agents · 1 hook receipt in the last hour · grok-traces doctor"
        );
        assert_eq!(rows[2], "");
        receipt(&mut writer);
        receipt(&mut writer);
        s.fold_now(now_ms());
        assert_eq!(
            snap(&mut s, 100, 4, t0)[1],
            " no live agents · 3 hook receipts in the last hour · grok-traces doctor"
        );
        let mut none = with_index(None);
        assert_eq!(snap(&mut none, 100, 3, t0)[1], " no live agents · no index");
        none.live_query = "x".into();
        assert_eq!(snap(&mut none, 100, 3, t0)[1], " no match");
    }

    #[test]
    fn tripped_index_says_so_and_the_body_still_renders() {
        let t0 = Instant::now();
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("idx/t.sqlite");
        let limits = xai_grok_traces::Limits {
            ceiling: 1,
            ..Default::default()
        };
        let ix = TraceIndex::open_with_limits(&db, None, limits).unwrap();
        assert!(ix.health().is_some());
        let mut s = with_index(Some(ix));
        s.set_live(
            vec![lr(
                "l1",
                "claude",
                "local one",
                LiveState::Working,
                "Read a",
                20,
            )],
            vec![],
        );
        let rows = snap(&mut s, 100, 3, t0);
        eprintln!("tripped: {rows:#?}");
        assert!(rows[0].starts_with(" index tripped · grok-traces doctor --reset"));
        assert!(rows[1].contains("local one"));
    }

    #[test]
    fn overflow_keeps_the_selected_last_row_visible_and_highlighted() {
        let t0 = Instant::now();
        let mut s = state();
        let rows: Vec<LiveRow> = (0..50)
            .map(|i| {
                lr(
                    &format!("a{i:02}"),
                    "claude",
                    &format!("agent {i:02}"),
                    LiveState::Working,
                    "Read x",
                    i,
                )
            })
            .collect();
        s.set_live(rows, vec![]);
        s.live_sel = Some("a49".into());
        let area = Rect::new(0, 0, 100, 20);
        let mut buf = Buffer::empty(area);
        // Pinned RGB theme: `current()` is terminal-detected and bandless (Reset colors) under TERM=dumb.
        let theme = Theme::groknight();
        render_live_at(&mut buf, area, &mut s, &theme, t0);
        let out = rows_of(&buf, 100, 20);
        assert!(out[19].contains("agent 49"), "{out:#?}");
        assert!(out[1].contains("agent 31"), "{out:#?}");
        assert_eq!(buf[(5, 19)].bg, theme.bg_highlight);
        assert_ne!(buf[(5, 18)].bg, theme.bg_highlight);
    }

    #[test]
    fn keys_collapse_move_and_back() {
        let mut s = state();
        s.set_live(family(), vec![]);
        assert_eq!(ids(&s), ["s", "r", "k1", "k2"]);
        press(&mut s, KeyCode::Char('j'));
        assert_eq!(s.live_sel.as_deref(), Some("r"));
        press(&mut s, KeyCode::Char('c'));
        assert_eq!(ids(&s), ["s", "r"], "collapsed");
        press(&mut s, KeyCode::Char('c'));
        assert_eq!(ids(&s), ["s", "r", "k1", "k2"], "expanded again");
        press(&mut s, KeyCode::Char('j'));
        assert_eq!(s.live_sel.as_deref(), Some("k1"));
        press(&mut s, KeyCode::Char('c'));
        assert_eq!(
            s.live_sel.as_deref(),
            Some("r"),
            "c on a child collapses the root and selects it"
        );
        assert_eq!(ids(&s), ["s", "r"]);
        s.live_wheel(true);
        assert_eq!(s.live_sel.as_deref(), Some("r"), "clamped at the last row");
        s.live_wheel(false);
        assert_eq!(s.live_sel.as_deref(), Some("s"));
    }

    #[test]
    fn hjkl_folds_the_live_screen() {
        let mut s = state();
        s.set_live(family(), vec![]);
        s.live_sel = Some("r".into());
        assert!(matches!(press(&mut s, KeyCode::Char('l')), InputOutcome::Unchanged));
        assert_eq!(ids(&s), ["s", "r", "k1", "k2"]);
        assert!(s.live_collapsed.is_empty());

        assert!(matches!(press(&mut s, KeyCode::Char('h')), InputOutcome::Changed));
        assert_eq!(ids(&s), ["s", "r"]);
        assert!(s.live_collapsed.contains("r"));
        assert_eq!(s.live_sel.as_deref(), Some("r"));

        assert!(matches!(press(&mut s, KeyCode::Char('h')), InputOutcome::Unchanged));
        assert!(s.live_collapsed.contains("r"));

        assert!(matches!(press(&mut s, KeyCode::Char('l')), InputOutcome::Changed));
        assert_eq!(ids(&s), ["s", "r", "k1", "k2"]);
        assert!(!s.live_collapsed.contains("r"));
        assert_eq!(s.live_sel.as_deref(), Some("r"));

        assert!(matches!(press(&mut s, KeyCode::Right), InputOutcome::Unchanged));
        assert!(matches!(press(&mut s, KeyCode::Left), InputOutcome::Changed));
        assert_eq!(ids(&s), ["s", "r"]);
        assert_eq!(s.live_sel.as_deref(), Some("r"));
        assert!(matches!(press(&mut s, KeyCode::Left), InputOutcome::Unchanged));
        assert!(matches!(press(&mut s, KeyCode::Right), InputOutcome::Changed));
        assert_eq!(ids(&s), ["s", "r", "k1", "k2"]);
        assert_eq!(s.live_sel.as_deref(), Some("r"));

        s.live_sel = Some("k1".into());
        assert!(matches!(press(&mut s, KeyCode::Char('l')), InputOutcome::Unchanged));
        assert_eq!(ids(&s), ["s", "r", "k1", "k2"]);
        assert!(matches!(press(&mut s, KeyCode::Char('h')), InputOutcome::Changed));
        assert_eq!(s.live_sel.as_deref(), Some("r"));
        assert_eq!(ids(&s), ["s", "r"]);

        assert!(matches!(press(&mut s, KeyCode::Right), InputOutcome::Changed));
        s.live_sel = Some("k2".into());
        assert!(matches!(press(&mut s, KeyCode::Left), InputOutcome::Changed));
        assert_eq!(s.live_sel.as_deref(), Some("r"));
        assert_eq!(ids(&s), ["s", "r"]);

        s.live_sel = Some("s".into());
        assert!(matches!(press(&mut s, KeyCode::Char('h')), InputOutcome::Unchanged));
        assert!(matches!(press(&mut s, KeyCode::Char('l')), InputOutcome::Unchanged));
        assert!(matches!(press(&mut s, KeyCode::Left), InputOutcome::Unchanged));
        assert!(matches!(press(&mut s, KeyCode::Right), InputOutcome::Unchanged));
        assert!(!s.live_collapsed.contains("s"), "a childless root is not recorded");
    }

    #[test]
    fn shortcut_labels_name_hjkl_fold() {
        let mut s = state();
        s.set_live(family(), vec![]);
        s.live_sel = Some("r".into());
        let labels: Vec<_> = shortcuts(&s, u16::MAX).iter().map(|sc| sc.label).collect();
        assert!(labels.contains(&"h/l fold"), "{labels:?}");
        assert!(labels.contains(&"c fold") && labels.contains(&"o open"), "{labels:?}");
        assert!(!labels.iter().any(|label| label.starts_with('q')), "q stays unlisted: {labels:?}");
    }

    #[test]
    fn q_goes_back_in_traces_and_exits_standalone() {
        let mut s = state();
        press(&mut s, KeyCode::Char('q'));
        assert_eq!(s.mode, Mode::List);
        assert!(!s.live_wants_exit());
        let mut s = state();
        s.standalone = true;
        press(&mut s, KeyCode::Char('q'));
        assert_eq!(s.mode, Mode::Live);
        assert!(s.live_wants_exit());
    }

    #[test]
    fn esc_standalone_sets_wants_exit() {
        let mut s = state();
        s.standalone = true;
        press(&mut s, KeyCode::Esc);
        assert!(s.live_wants_exit());
    }

    #[test]
    fn esc_with_query_clears_not_exits() {
        let mut s = state();
        s.standalone = true;
        s.live_query = "bash".into();
        press(&mut s, KeyCode::Esc);
        assert!(s.live_query.is_empty());
        assert!(!s.live_wants_exit());
        press(&mut s, KeyCode::Esc);
        assert!(s.live_wants_exit());
    }

    #[test]
    fn slash_filters_and_keeps_parents_of_matching_children() {
        let mut s = state();
        s.set_live(family(), vec![]);
        press(&mut s, KeyCode::Char('/'));
        assert_eq!(s.typing, Typing::Live);
        for c in "explore".chars() {
            press(&mut s, KeyCode::Char(c));
        }
        assert_eq!(ids(&s), ["r", "k1"], "child match keeps its parent");
        press(&mut s, KeyCode::Enter);
        assert_eq!(s.typing, Typing::No);
        assert_eq!(s.live_query, "explore");
        press(&mut s, KeyCode::Esc);
        assert_eq!(ids(&s).len(), 4);
        press(&mut s, KeyCode::Char('/'));
        press(&mut s, KeyCode::Char('x'));
        press(&mut s, KeyCode::Esc);
        assert_eq!(
            (s.typing, s.live_query.as_str()),
            (Typing::No, ""),
            "Esc while typing clears and stops"
        );
    }

    #[test]
    fn selection_survives_a_refold_that_reorders_rows() {
        let mut s = state();
        s.set_live(
            vec![
                lr("a", "claude", "A", LiveState::Working, "x", 1),
                lr("b", "claude", "B", LiveState::Working, "x", 2),
                lr("c", "claude", "C", LiveState::Working, "x", 3),
            ],
            vec![],
        );
        s.live_sel = Some("b".into());
        s.live_pos(&s.live_view(Instant::now()).lines);
        assert_eq!(s.live_sel_idx, 1);
        let rows = vec![
            lr("a", "claude", "A", LiveState::Working, "x", 9),
            lr("b", "claude", "B", LiveState::Working, "x", 0),
            lr("c", "claude", "C", LiveState::Working, "x", 3),
        ];
        s.apply_fold(LiveFold {
            rows: Ok(rows),
            ..fold0()
        });
        let v = s.live_view(Instant::now());
        assert_eq!(s.live_pos(&v.lines), Some(0));
        assert_eq!(s.live_sel.as_deref(), Some("b"));
        // The selected row vanishes: the row now at the same position is selected.
        s.apply_fold(LiveFold {
            rows: Ok(vec![
                lr("a", "claude", "A", LiveState::Working, "x", 9),
                lr("c", "claude", "C", LiveState::Working, "x", 3),
            ]),
            ..fold0()
        });
        let v = s.live_view(Instant::now());
        assert_eq!(s.live_pos(&v.lines), Some(0));
        assert_eq!(
            s.live_sel.as_deref(),
            Some("b"),
            "a fallback row does not replace the user's selection"
        );
        // It comes back, and is selected again.
        s.apply_fold(LiveFold {
            rows: Ok(vec![
                lr("a", "claude", "A", LiveState::Working, "x", 9),
                lr("b", "claude", "B", LiveState::Working, "x", 0),
            ]),
            ..fold0()
        });
        let v = s.live_view(Instant::now());
        assert_eq!(s.live_pos(&v.lines).map(|i| v.lines[i].row.session_id.as_str()), Some("b"));
    }

    fn fold0() -> LiveFold {
        LiveFold {
            generation: 0,
            at: Instant::now(),
            rows: Ok(Vec::new()),
            titles: HashMap::new(),
            receipts_hour: 0,
            receipts_any: None,
        }
    }

    #[test]
    fn via_suffix_is_gray() {
        let t0 = Instant::now();
        let mut s = state();
        let mut r = lr("v", "claude", "docs sweep", LiveState::Working, "x", 1);
        r.via = Some("bb".into());
        s.set_live(vec![r], vec![]);
        let area = Rect::new(0, 0, 100, 3);
        let mut buf = Buffer::empty(area);
        // Pinned RGB theme: `current()` is terminal-detected and bandless (Reset colors) under TERM=dumb.
        let theme = Theme::groknight();
        render_live_at(&mut buf, area, &mut s, &theme, t0);
        let row = rows_of(&buf, 100, 3)[1].clone();
        let x = row[..row.find("via").unwrap()].chars().count() as u16;
        assert_eq!(buf[(x, 1)].fg, theme.gray);
        assert_ne!(buf[(x - 3, 1)].fg, theme.gray);
    }

    #[test]
    fn local_rows_age_since_their_fold() {
        let t0 = Instant::now();
        let mut s = state();
        s.set_live(vec![lr("a", "claude", "A", LiveState::Working, "x", 5)], vec![]);
        s.live_fold_at = t0 - Duration::from_secs(8);
        let v = s.live_view(t0);
        assert_eq!(live::fmt_age(live::age_ms(&v.lines[0].row, 0)), "13s");
        s.live_fold_at = t0;
        let v = s.live_view(t0);
        assert_eq!(live::fmt_age(live::age_ms(&v.lines[0].row, 0)), "5s");
    }

    #[test]
    fn q_in_traces_clears_the_live_query() {
        let mut s = state();
        s.live_query = "bash".into();
        press(&mut s, KeyCode::Char('q'));
        assert_eq!(s.mode, Mode::List);
        assert!(s.live_query.is_empty());
    }

    #[test]
    fn narrow_header_keeps_the_local_host_and_drops_remotes_first() {
        let t0 = Instant::now();
        let mut s = state();
        s.set_live(
            states(),
            vec![("sab@sab-air", vec![], t0, true, None)],
        );
        let wide = snap(&mut s, 60, 2, t0);
        assert!(wide[0].ends_with("sab-mini ● sab-air ●"), "{:?}", wide[0]);
        let mid = snap(&mut s, 50, 2, t0);
        assert!(mid[0].ends_with("sab-mini ●") && !mid[0].contains("sab-air"), "{:?}", mid[0]);
        let tight = snap(&mut s, 30, 2, t0);
        assert!(tight[0].ends_with("sab-mini ●"), "{:?}", tight[0]);
        assert!(tight[0].contains("grok live"), "{:?}", tight[0]);
        let tiny = snap(&mut s, 8, 2, t0);
        assert!(tiny[0].width() <= 8);
    }

    #[test]
    fn a_gated_host_shows_no_cached_rows_after_a_later_error() {
        let mut s = state();
        s.hosts = vec!["sab@sab-air.invalid".into()];
        let ok = |rows: Vec<LiveRow>, note: Option<&str>| LiveHostResult {
            target: "sab@sab-air.invalid".into(),
            result: Ok(Fetched { rows, note: note.map(String::from) }),
        };
        let err = || LiveHostResult {
            target: "sab@sab-air.invalid".into(),
            result: Err(remote::Error::Unreachable { host: "sab@sab-air.invalid".into(), detail: "down".into() }),
        };
        s.apply_live_host(ok(vec![], Some("sab-air: update grok-traces")));
        assert!(s.remote_live["sab@sab-air.invalid"].gated);
        s.apply_live_host(err());
        let r = &s.remote_live["sab@sab-air.invalid"];
        assert!(r.rows.is_empty() && !r.ok && r.gated);
        assert!(r.note.is_some(), "the note is kept");
        s.apply_live_host(ok(remote_rows(), None));
        let r = &s.remote_live["sab@sab-air.invalid"];
        assert!(!r.gated && r.ok && r.rows.len() == 3);
    }

    fn pi_state() -> (tempfile::TempDir, TracesModalState, String) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("pi");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.jsonl"), "{\"type\":\"message\",\"timestamp\":\"2026-10-01T00:00:01Z\",\"message\":{\"role\":\"user\",\"content\":\"task a\"}}\n").unwrap();
        let roots = [xai_grok_traces::Root {
            id: "pi".into(),
            source: "pi".into(),
            path: dir,
            format: xai_grok_traces::Format::Jsonl,
            file_name: None,
        }];
        let db = tmp.path().join("idx/t.sqlite");
        TraceIndex::open(&db).unwrap().scan(&roots).unwrap();
        let mut s = with_index(Some(TraceIndex::open(&db).unwrap()));
        s.show_all = true;
        s.refresh();
        let id = s.sessions[0].id.clone();
        (tmp, s, id)
    }

    #[test]
    fn o_opens_the_trajectory_and_esc_returns_to_live_with_the_selection() {
        let (_tmp, mut s, id) = pi_state();
        s.set_live(
            vec![
                lr(&id, "pi", "task a", LiveState::Working, "x", 1),
                lr("zz", "pi", "other", LiveState::Working, "x", 5),
            ],
            vec![],
        );
        press(&mut s, KeyCode::Char('j'));
        assert_eq!(s.live_sel.as_deref(), Some("zz"));
        press(&mut s, KeyCode::Char('k'));
        press(&mut s, KeyCode::Char('o'));
        assert_eq!(s.mode, Mode::Trajectory);
        assert_eq!(s.session.as_ref().map(|x| x.id.as_str()), Some(id.as_str()));
        press(&mut s, KeyCode::Esc);
        assert_eq!(s.mode, Mode::Live);
        assert!(!s.live_return && s.session.is_none());
        assert_eq!(s.live_sel.as_deref(), Some(id.as_str()));
        // A row the index does not know cannot be opened.
        press(&mut s, KeyCode::Char('j'));
        press(&mut s, KeyCode::Enter);
        assert_eq!(s.mode, Mode::Live);
        assert_eq!(s.status.as_deref(), Some("cannot open zz"));
    }

    #[test]
    fn o_on_a_remote_row_opens_with_the_host_set() {
        let mut s = state();
        s.set_live(
            vec![],
            vec![(
                "nobody@sab-air.invalid",
                remote_rows(),
                Instant::now(),
                true,
                None,
            )],
        );
        s.live_sel = Some("sab-air:b".into());
        // The row's host label is what the ledger matches on; this target's label differs, so no ssh is attempted.
        press(&mut s, KeyCode::Char('o'));
        assert_eq!(s.mode, Mode::Trajectory);
        let sess = s.session.as_ref().unwrap();
        assert_eq!(
            (sess.id.as_str(), sess.host.as_deref(), sess.source.as_str()),
            ("sab-air:b", Some("sab-air"), "pi")
        );
        assert!(s.live_return);
    }

    #[test]
    fn l_enters_live_from_the_list() {
        let tmp = tempfile::tempdir().unwrap();
        let (_w, r) = idx(&tmp);
        let mut s = TracesModalState::open_with_hosts(Some(r), vec![]);
        assert_eq!(s.mode, Mode::List);
        press(&mut s, KeyCode::Char('L'));
        assert_eq!(s.mode, Mode::Live);
        assert!(s.live_inflight, "first fold starts at once");
        let end = Instant::now() + Duration::from_secs(10);
        while s.live_inflight && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(5));
            assert!(s.poll_live_at(now_ms()), "Live repaints every tick");
        }
        s.fold_now(now_ms());
        assert!(s.live_rows.is_empty() && s.status.is_none());
    }

}
