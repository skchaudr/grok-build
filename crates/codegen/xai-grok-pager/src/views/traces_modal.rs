//! `/traces`: ledger of local agent sessions → trajectory → raw-line inspector.
//!
//! Queries run synchronously against the local index (cheap); discovery and
//! parsing run in a background effect and refresh the ledger when done.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use xai_grok_traces::remote::{self, EventQuery, Fetched, RemoteFilter};
use xai_grok_traces::{EventFilter, EventRow, RECEIPT_WINDOW_MS, SOURCES, SessionFilter, SessionRow, Sort, Status, TraceIndex, source_label};

use crate::app::actions::Action;
use crate::app::app_view::InputOutcome;
use crate::render::line_utils::truncate_str;
use crate::theme::Theme;
use crate::views::modal_window::{self, ModalContentArea, ModalSizing, ModalWindowConfig, ModalWindowState, Shortcut};
use crate::views::traces_fold::{FoldNode, FoldState, FoldTree};
use crate::views::traces_outline::{self, OutlineItem, OutlineKey};

pub const TITLE: &str = "Traces";
const PAGE: usize = 500;
const EVENT_PAGE: usize = 5000;
/// Children shown under a parent before `+N more`.
const MAX_KIDS: usize = 6;
/// Most wrapped lines a message body shows before its `… Enter for inspector` tail.
const BODY_LINES: usize = 200;
/// Columns before a trajectory row's text: `#line` and the role chip, each with a space.
const TEXT_COL: usize = 14;
/// Deepest ledger level: roots are 0, and nothing below depth `MAX_DEPTH` is loaded or shown.
const MAX_DEPTH: usize = 8;
/// Tree-guide levels drawn in the ledger (the node's own level included); deeper ones are cut to `…`.
const GUIDE_LEVELS: usize = 3;
/// Separators between a folded call's body and its result.
const RESULT_SEP: &str = "── result ──";
const RESULT_ERR_SEP: &str = "── ✗ result ──";
/// Key prefix of a `+N more` node; it cannot collide with a session id.
const MORE_KEY: &str = "\u{1}more:";
/// How often remote hosts are refetched while the modal is open.
const REMOTE_EVERY: Duration = Duration::from_secs(10);
/// Remote body fetches allowed in flight at once; the rest wait for a later render.
const MAX_BODY_FETCHES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    List,
    Trajectory,
    Inspector,
}

/// Which line editor has the keyboard, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Typing {
    No,
    Ledger,
    Events,
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
    /// The selected row, by key: kept across folds, filters and live growth.
    sel: Option<OutlineKey>,
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
    raw_rx: Option<mpsc::Receiver<String>>,
    /// Raw line of the inspected remote event, for `y`.
    raw_source: Option<String>,
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
            sel: None,
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

    /// Cheap checks on the UI thread (three tiny queries); anything heavier runs on a worker
    /// and is applied by a later poll.
    fn poll_live_at(&mut self, now_ms: i64) -> bool {
        let mut redraw = false;
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
        let Some(index) = &self.index else { return redraw };
        let mark = index.receipts_watermark().unwrap_or(self.watermark);
        let live = index.last_receipt_at().ok().flatten().is_some_and(|at| now_ms - at < RECEIPT_WINDOW_MS);
        let health = index.health();
        redraw |= live != self.live || health != self.health;
        (self.live, self.health) = (live, health);
        if mark != self.watermark && self.inflight.is_none() {
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
            // Follow the bottom if the user was there.
            let at_bottom = self.sel_pos() + 1 >= self.oshown.len();
            self.install_trajectory(t.rows, t.session, t.children);
            if at_bottom {
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
            Err(e) => self.status = Some(format!("query failed: {e}")),
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
        self.status = Some(summary);
        self.refresh();
    }

    pub fn report_copy(&mut self, delivery: &crate::clipboard::CopyDelivery) {
        self.status = Some(delivery.toast_message().into_owned());
    }

    fn open_selected(&mut self) {
        let Some(row) = self.selected_session().cloned() else { return };
        self.trail.clear();
        self.open_session(row);
    }

    fn open_session(&mut self, row: SessionRow) {
        let remote = row.host.is_some();
        self.parent_name = match (&self.index, &row.parent_id) {
            _ if remote => row.parent_id.as_deref().and_then(|pid| self.remote_rows.values().flatten().find(|r| r.id == pid)).map(clean_name),
            (Some(i), Some(pid)) => i.session(pid).ok().flatten().map(|p| clean_name(&p)),
            _ => None,
        };
        self.session = Some(row);
        self.sel = None;
        self.reveal = None;
        self.event_scroll = 0;
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
        if let Some(t) = self.trail.pop() {
            self.open_session(t.session);
            // Back where we left: the same folds and row. The row's ancestors open so it shows.
            self.ofolds = t.folds;
            self.oseen = t.seen;
            self.reveal = t.sel.clone();
            self.sel = t.sel;
            self.rebuild_outline();
        } else {
            self.mode = Mode::List;
            self.session = None;
            self.set_events(Vec::new());
            self.remote_loading = false;
            self.remote_ev = None;
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
            Err(e) => self.status = Some(format!("query failed: {e}")),
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
        self.sel.as_ref().and_then(|k| self.oshown.iter().position(|&n| self.key_at(n) == Some(k))).unwrap_or(0)
    }

    /// Selects the visible row at `pos`.
    fn goto(&mut self, pos: usize) {
        self.sel = self.oshown.get(pos).and_then(|&n| self.key_at(n)).cloned();
    }

    fn step(&mut self, delta: isize) {
        let mut pos = self.sel_pos();
        move_sel(&mut pos, self.oshown.len(), delta);
        self.goto(pos);
    }

    /// Event index of the selected row: a turn's opener, a run's first call, or the row's own event.
    fn selected_event(&self) -> Option<usize> {
        match self.outline_items.get(*self.oshown.get(self.sel_pos())?)? {
            OutlineItem::Turn { event, .. } | OutlineItem::Run { first: event, .. } => Some(*event),
            OutlineItem::Event(i) | OutlineItem::Body(i) => Some(*i),
        }
    }

    /// Rebuilds the outline from `events`. The selection stays on its key when that row is still
    /// shown, else on the row at the old position.
    fn rebuild_outline(&mut self) {
        let pos = self.sel_pos();
        let (tree, items) = traces_outline::outline(&self.events, &self.hidden);
        (self.outline, self.outline_items) = (tree, items);
        // An empty outline (remote events still loading, or no match) keeps the folds and selection for when rows return.
        if !self.outline.nodes.is_empty() {
            self.ofolds.retain(&self.outline);
            let live: HashSet<OutlineKey> = self.outline.nodes.iter().map(|n| n.key.clone()).collect();
            self.oseen.retain(|k| live.contains(k));
        }
        // Defaults on first sight: turns open, runs and event rows closed. Under a filter, turns and runs open
        // on first sight and again when the filter itself changes, so a manual collapse survives live polls.
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
            let fresh = self.oseen.insert(key);
            if fresh && !turn && !(filtering && run) {
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
        self.place(self.sel.clone(), pos);
    }

    /// Expands every ancestor of the outline row `key` so it is shown. False when there is no such row.
    fn reveal(&mut self, key: &OutlineKey) -> bool {
        let Some(i) = self.outline.nodes.iter().position(|n| &n.key == key) else { return false };
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
        let at = keep
            .as_ref()
            .and_then(|k| self.oshown.iter().position(|&n| self.key_at(n) == Some(k)))
            .unwrap_or_else(|| pos.min(self.oshown.len().saturating_sub(1)));
        self.goto(at);
    }

    /// Re-lists the outline after a fold change on `node`, keeping the selection on it or its nearest shown ancestor.
    fn reshow(&mut self, node: usize) {
        self.oshown = self.outline.visible(&self.ofolds);
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

    /// Wrapped body lines per shown row (`None` for rows that are not bodies), `text_w` columns wide.
    fn body_rows(&self, text_w: usize) -> Vec<Option<Vec<String>>> {
        self.oshown
            .iter()
            .map(|&n| {
                let depth = self.outline.nodes.get(n).map_or(0, |x| x.depth);
                match self.outline_items.get(n) {
                    Some(OutlineItem::Body(i)) => Some(self.body_lines(*i, text_w.saturating_sub(2 * depth))),
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
                self.bodies.insert(line, text);
                true
            }
            Err(e) => {
                self.body_failed.insert(line);
                self.status = Some(format!("(could not read #{line}: {e}, press l to retry)"));
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
            self.status = None;
        }
        true
    }

    /// The result folded into event `i`'s call row: its line and whether it failed.
    fn result_of(&self, i: usize) -> Option<(i64, bool)> {
        let fold = self.folded.get(i)?.as_ref()?;
        Some((self.events.get(i + 1)?.line, fold.error))
    }

    /// The body text of event `ev` wrapped to `width`; a placeholder until a remote body arrives.
    fn body_lines(&self, ev: usize, width: usize) -> Vec<String> {
        let Some(line) = self.events.get(ev).map(|e| e.line) else { return vec!["loading…".to_string()] };
        match self.bodies.get(&line) {
            Some(text) => wrap_body(text, width),
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
        self.raw_scroll = 0;
        self.mode = Mode::Inspector;
    }

    /// The raw source line for the inspected event, unformatted.
    fn raw_source_line(&self) -> Option<String> {
        if self.session.as_ref().is_some_and(|s| s.host.is_some()) {
            return self.raw_source.clone();
        }
        let index = self.index.as_ref()?;
        let session = self.session.as_ref()?;
        let line = self.events.get(self.selected_event()?)?.line;
        index.raw_line(&session.id, line).ok().flatten()
    }

    fn resume(&self, row: Option<&SessionRow>) -> InputOutcome {
        let Some(row) = row else { return InputOutcome::Unchanged };
        if let Some(host) = &row.host {
            return InputOutcome::Action(Action::TracesNotice { text: format!("remote session on {host}: resume it there") });
        }
        match resume_target(row) {
            Some(t) => InputOutcome::Action(Action::TracesResume { source: t.source, native_id: t.native_id }),
            None => InputOutcome::Action(Action::TracesNotice { text: format!("no resume for {}", source_label(&row.source)) }),
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
            if !self.hosts.is_empty() && self.remote_at.is_none_or(|t| t.elapsed() >= REMOTE_EVERY) {
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
                    Err(e) => self.status = Some(e.to_string()),
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
            && let Ok(text) = rx.try_recv()
        {
            self.raw_rx = None;
            self.raw_source = Some(text.clone());
            self.raw = Some(pretty(Ok(Some(text))));
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
                if note.is_some() {
                    self.status = note;
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
            self.status = Some("host is no longer configured".into());
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
                q.is_empty() || [Some(&e.title), Some(&e.preview), e.detail.as_ref()].into_iter().flatten().any(|f| f.to_lowercase().contains(&q))
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
        self.raw_scroll = 0;
        self.mode = Mode::Inspector;
        std::thread::spawn(move || {
            let text = match remote::fetch_raw(&target, remote::remote_id(&target, &session.id), line) {
                Ok(Some(text)) => text,
                Ok(None) => return drop(tx.send(String::new())),
                Err(e) => return drop(tx.send(format!("{{\"error\": \"{e}\"}}"))),
            };
            let _ = tx.send(text);
        });
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
    for line in text.lines() {
        let mut cur = String::new();
        let mut w = 0;
        for c in line.chars() {
            let cw = UnicodeWidthChar::width(c).unwrap_or(0);
            if w + cw > width && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
                w = 0;
            }
            cur.push(c);
            w += cw;
        }
        out.push(cur);
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

/// Scrolls so row `sel` sits fully inside `height` lines, where row `i` is `heights[i]` lines tall.
fn fit_rows(sel: usize, scroll: &mut usize, heights: &[usize], height: usize) {
    if sel < *scroll {
        *scroll = sel;
    }
    let mut total: usize = heights.iter().skip(*scroll).take(sel + 1 - *scroll).sum();
    while total > height && *scroll < sel {
        total -= heights.get(*scroll).copied().unwrap_or(0);
        *scroll += 1;
    }
}

fn pretty(raw: Result<Option<String>, String>) -> String {
    match raw {
        Ok(Some(line)) if !line.is_empty() => serde_json::from_str::<serde_json::Value>(&line)
            .ok()
            .and_then(|v| serde_json::to_string_pretty(&v).ok())
            .unwrap_or(line),
        Ok(_) => "(line no longer in the source file)".to_string(),
        Err(e) => format!("(could not read source: {e})"),
    }
}

/// String values under these keys are body text, in any harness format.
const TEXT_KEYS: [&str; 6] = ["text", "thinking", "output", "result", "stdout", "stderr"];
/// Tool inputs under these keys render as JSON (or as the string, when it already is one).
const INPUT_KEYS: [&str; 4] = ["input", "arguments", "args", "params"];

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
                    _ if INPUT_KEYS.contains(&key) => {
                        if let Ok(pretty) = serde_json::to_string_pretty(x) {
                            push_chunk(out, &pretty);
                        }
                    }
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
fn compose_body(own: Result<Option<String>, String>, result: Option<(Result<Option<String>, String>, bool)>) -> Result<String, String> {
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

/// Esc in a sub-view or an editor is handled here; only the ledger's Esc closes the modal.
pub fn wants_esc(state: &TracesModalState) -> bool {
    state.typing != Typing::No || state.mode != Mode::List
}

pub fn handle_traces_key(state: &mut TracesModalState, key: &KeyEvent) -> InputOutcome {
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
        Typing::No => {}
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
                KeyCode::Char('r') => return state.resume(state.selected_session()),
                KeyCode::Enter => {
                    if !state.expand_selected_more() {
                        state.open_selected();
                    }
                }
                _ => return InputOutcome::Unchanged,
            }
        }
        Mode::Trajectory => {
            let len = state.oshown.len();
            match key.code {
                KeyCode::Esc => state.leave_trajectory(),
                KeyCode::Char('j') | KeyCode::Down => state.step(1),
                KeyCode::Char('k') | KeyCode::Up => state.step(-1),
                KeyCode::PageDown => state.step(page),
                KeyCode::PageUp => state.step(-page),
                KeyCode::Char('g') | KeyCode::Home => state.goto(0),
                KeyCode::Char('G') | KeyCode::End => state.goto(len.saturating_sub(1)),
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
                KeyCode::Char('r') => return state.resume(state.session.as_ref()),
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
        }
        Mode::Inspector => match key.code {
            KeyCode::Esc => {
                state.mode = Mode::Trajectory;
                state.raw = None;
            }
            KeyCode::Char('j') | KeyCode::Down => state.raw_scroll += 1,
            KeyCode::Char('k') | KeyCode::Up => state.raw_scroll = state.raw_scroll.saturating_sub(1),
            KeyCode::PageDown => state.raw_scroll += page as usize,
            KeyCode::PageUp => state.raw_scroll = state.raw_scroll.saturating_sub(page as usize),
            KeyCode::Char('g') | KeyCode::Home => state.raw_scroll = 0,
            KeyCode::Char('y') => {
                if let Some(text) = state.raw_source_line() {
                    return InputOutcome::Action(Action::TracesCopy { text });
                }
                return InputOutcome::Unchanged;
            }
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

fn shortcuts(state: &TracesModalState) -> Vec<Shortcut<'static>> {
    let plain = |label: &'static str| Shortcut { label, clickable: false, id: 0 };
    if state.typing != Typing::No {
        return vec![plain("type to filter"), plain("Enter search events"), plain("Esc clear")];
    }
    match state.mode {
        Mode::List => vec![
            plain("j/k move"),
            plain("Tab source"),
            plain("/ search"),
            plain("a all"),
            plain("A active"),
            plain("x errors"),
            plain("H host"),
            plain("s sort"),
            plain("Enter open"),
            plain("e fold"),
            plain("E all"),
            plain("</> more/less"),
            plain("→ expand"),
            plain("r resume"),
            plain("R rescan"),
            plain("Esc close"),
        ],
        Mode::Trajectory => vec![
            plain("j/k move"),
            plain("Enter inspect/open agent"),
            plain("e fold"),
            plain("E all"),
            plain("</> more/less"),
            plain("n noise"),
            plain("x errors"),
            plain("t tool"),
            plain("/ search"),
            plain("r resume"),
            plain("Esc back"),
        ],
        Mode::Inspector => vec![plain("j/k scroll"), plain("y copy line"), plain("Esc back")],
    }
}

pub fn render_traces_modal(buf: &mut Buffer, full_area: Rect, state: &mut TracesModalState, compact: bool) {
    let theme = Theme::current();
    let shortcuts = shortcuts(state);
    let config = ModalWindowConfig {
        title: TITLE,
        tabs: None,
        shortcuts: &shortcuts,
        sizing: ModalSizing { width_pct: 1.0, max_width: u16::MAX, min_width: 44, v_margin: 0, h_pad: 2, v_pad: 0, footer_lines: 2 }
            .with_compact(compact),
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
fn status_glyph(s: &SessionRow, theme: &Theme) -> (&'static str, Color) {
    match s.status {
        Status::Active => ("●", theme.accent_running),
        _ if s.failed => ("✗", theme.accent_error),
        Status::Completed => ("✓", theme.gray),
        _ => ("○", theme.gray_dim),
    }
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
    flags.push(format!("sort:{:?}", state.sort).to_lowercase());
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
                v.push((format!("{}  +{hidden} more (→)", guide_prefix(&node.guides)), c.name, base.fg(theme.gray)));
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

fn render_trajectory(buf: &mut Buffer, area: Rect, state: &mut TracesModalState, theme: &Theme) {
    let Some(session) = state.session.clone() else { return };
    let dim = Style::default().fg(theme.gray);
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

    let status_rows = u16::from(state.status.is_some());
    let list = Rect { y: area.y + 5, height: area.height.saturating_sub(5 + status_rows), ..area };
    if state.oshown.is_empty() {
        let msg = if state.remote_loading { "Fetching events over ssh…" } else { "No events match." };
        buf.set_span(list.x, list.y, &Span::styled(msg, dim), list.width);
    }
    // Body rows are as tall as their wrapped text, so scrolling works in lines, not rows.
    let text_w = (list.width as usize).saturating_sub(TEXT_COL);
    let height = list.height as usize;
    let mut bodies = state.body_rows(text_w);
    let mut heights = row_heights(&bodies);
    fit_rows(state.sel_pos(), &mut state.event_scroll, &heights, height);
    // Load the bodies on screen; one cached right here changes row heights, so measure again.
    if state.ensure_bodies(state.event_scroll, height) {
        bodies = state.body_rows(text_w);
        heights = row_heights(&bodies);
        fit_rows(state.sel_pos(), &mut state.event_scroll, &heights, height);
    }

    let summary_w = (list.width as usize).saturating_sub(7 + 7 + 9 + 2);
    let bottom = list.y + list.height;
    let sel = state.sel_pos();
    let mut y = list.y;
    for (k, &n) in state.oshown.iter().enumerate().skip(state.event_scroll) {
        let h = heights.get(k).copied().unwrap_or(1) as u16;
        if y >= bottom {
            break;
        }
        let span = h.min(bottom - y);
        let selected = k == sel;
        let base = if selected { Style::default().bg(theme.bg_highlight).fg(theme.text_primary) } else { Style::default().fg(theme.text_primary) };
        if selected {
            for dy in 0..span {
                fill(buf, list, y + dy, base);
            }
        }
        let depth = state.outline.nodes.get(n).map_or(0, |x| x.depth);
        let indent = "  ".repeat(depth);
        let mark = state.fold_mark(n);
        let collapsed = state.outline.has_children(n) && state.key_at(n).is_some_and(|key| state.ofolds.is_collapsed(key));
        match state.outline_items.get(n) {
            Some(OutlineItem::Turn { event, tools, msgs, errors }) => {
                let Some(e) = state.events.get(*event) else {
                    y += h;
                    continue;
                };
                let preview = one_line(e.detail.as_deref().filter(|d| !d.is_empty()).unwrap_or(&e.preview));
                let mut text = format!("{indent}{mark}{preview}");
                if collapsed {
                    let mut parts = Vec::new();
                    if *tools > 0 {
                        parts.push(count_of(*tools, "tool", "tools"));
                    }
                    if *msgs > 0 {
                        parts.push(count_of(*msgs, "msg", "msgs"));
                    }
                    if *errors > 0 {
                        parts.push(format!("✗{errors}"));
                    }
                    if !parts.is_empty() {
                        text.push_str(&format!("  · {}", parts.join(" · ")));
                    }
                }
                put_row(
                    buf,
                    list,
                    y,
                    &[
                        (format!("{:>6}", format!("#{}", e.line)), 6, base.fg(theme.gray)),
                        (role_chip(e).to_string(), 6, base.fg(chip_color(e, theme)).add_modifier(Modifier::BOLD)),
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
                    continue;
                };
                // Noise (only present when `n` is on) is dimmed.
                let (text_style, chip_fg) = if e.noise { (base.fg(theme.gray_dim), theme.gray_dim) } else { (base, chip_color(e, theme)) };
                let one = one_line(e.detail.as_deref().filter(|d| !d.is_empty()).unwrap_or(&e.preview));
                let body = if is_call(e) { format!("{indent}{mark}{}  {}", e.title, one) } else { format!("{indent}{mark}{one}") };
                // Suffix: where a dispatch went, and how the call ended.
                let mut suffix = String::new();
                if let Some(c) = state.child_of(*i) {
                    suffix.push_str(&format!("→ {} {}", truncate_str(&clean_name(c), 28), status_glyph(c, theme).0));
                }
                let mut failed = e.is_error;
                if let Some(Some(f)) = state.folded.get(*i) {
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
                let sw = suffix.width().min(summary_w / 2);
                let body_w = summary_w.saturating_sub(if sw > 0 { sw + 1 } else { 0 });
                let body_style = if is_result(e) && e.is_error { text_style.fg(theme.accent_error) } else { text_style };
                let mut cells = vec![
                    (format!("{:>6}", format!("#{}", e.line)), 6, base.fg(theme.gray)),
                    (role_chip(e).to_string(), 6, base.fg(chip_fg).add_modifier(Modifier::BOLD)),
                    (body, body_w, body_style),
                ];
                if sw > 0 {
                    cells.push((suffix, sw, base.fg(if failed { theme.accent_error } else { theme.accent_success })));
                }
                cells.push((fmt_time(e.timestamp, false), 8, base.fg(theme.text_secondary)));
                put_row(buf, list, y, &cells);
            }
            Some(OutlineItem::Body(_)) => {
                let indent_w = (2 * depth).min(text_w);
                // A failed call's result section (after its separator) is drawn in the error colour.
                let failed = matches!(state.outline_items.get(n), Some(OutlineItem::Body(i)) if state.folded.get(*i).and_then(Option::as_ref).is_some_and(|f| f.error));
                let mut in_result = false;
                for (dy, line) in bodies.get(k).into_iter().flatten().flatten().take(span as usize).enumerate() {
                    in_result |= line.as_str() == RESULT_SEP || line.as_str() == RESULT_ERR_SEP;
                    let fg = if in_result && failed { theme.accent_error } else { theme.text_secondary };
                    let text = format!("{}{line}", " ".repeat(indent_w));
                    put_row(buf, list, y + dy as u16, &[(String::new(), 6, base), (String::new(), 6, base), (text, text_w, base.fg(fg))]);
                }
            }
            None => {}
        }
        y += h;
    }
    render_status(buf, area, state, theme);
}

/// Lines per shown row: a body is as tall as its wrapped text, every other row is one line.
fn row_heights(bodies: &[Option<Vec<String>>]) -> Vec<usize> {
    bodies.iter().map(|b| b.as_ref().map_or(1, |v| v.len().max(1))).collect()
}

/// `1 tool` / `3 tools`.
fn count_of(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn render_inspector(buf: &mut Buffer, area: Rect, state: &mut TracesModalState, theme: &Theme) {
    let dim = Style::default().fg(theme.gray);
    if let Some(e) = state.selected_event().and_then(|i| state.events.get(i)) {
        let head = format!("{} · line {} · {} · {}", e.event_type, e.line, fmt_time(e.timestamp, true), fmt_duration(e.duration_ms));
        buf.set_span(area.x, area.y, &Span::styled(truncate_str(&head, area.width as usize).as_str(), Style::default().fg(theme.text_primary).add_modifier(Modifier::BOLD)), area.width);
    }
    let Some(raw) = &state.raw else { return };
    let lines: Vec<&str> = raw.lines().collect();
    let status_rows = u16::from(state.status.is_some());
    let body = Rect { y: area.y + 2, height: area.height.saturating_sub(2 + status_rows), ..area };
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
            _ => render_ledger(&mut buf, area, state, &theme),
        }
        (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect::<Vec<_>>().join("\n")
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
        assert!(matches!(state.resume(state.selected_session()), InputOutcome::Action(Action::TracesNotice { .. })));

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

    #[test]
    fn turn_collapses_to_a_summary_and_back() {
        let mut state = trajectory(three_calls());
        assert_eq!(shown_rows(&state), ["Turn(1)", "Ev(1)", "Run(2)", "Ev(8)"], "turn open, opener and run closed");
        assert!(draw(&mut state, 140, 20).contains("3 tool calls"));

        state.goto(0);
        press(&mut state, KeyCode::Char('h'));
        assert_eq!(shown_rows(&state), ["Turn(1)"]);
        assert!(draw(&mut state, 140, 20).contains("3 tools · 1 msg"), "{}", draw(&mut state, 140, 20));

        press(&mut state, KeyCode::Char('l'));
        assert_eq!(shown_rows(&state).len(), 4);
    }

    #[test]
    fn run_is_one_row_until_expanded() {
        let mut state = trajectory(three_calls());
        state.goto(2);
        press(&mut state, KeyCode::Right);
        assert_eq!(shown_rows(&state), ["Turn(1)", "Ev(1)", "Run(2)", "Ev(2)", "Ev(4)", "Ev(6)", "Ev(8)"]);
        assert!(draw(&mut state, 140, 20).contains("Bash  ls"));
    }

    #[test]
    fn angle_bracket_opens_runs_and_bodies_under_a_turn() {
        let mut state = trajectory(three_calls());
        state.goto(0);
        press(&mut state, KeyCode::Char('>'));
        assert_eq!(shown_rows(&state).len(), 12, "{:?}", shown_rows(&state));
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
        state.goto(2);
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
        state.goto(2);
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
        let mut state = trajectory(three_calls());
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
        assert_eq!(shown_rows(&state).len(), 12, "anything folded: expand all");
        press(&mut state, KeyCode::Char('E'));
        assert_eq!(shown_rows(&state), ["Turn(1)"], "nothing folded: collapse all");
    }

    #[test]
    fn fit_rows_keeps_the_selected_row_on_screen() {
        let mut scroll = 0;
        fit_rows(5, &mut scroll, &[1, 1, 1, 1, 1, 1], 3);
        assert_eq!(scroll, 3);
        let mut scroll = 0;
        fit_rows(2, &mut scroll, &[1, 4, 1], 3);
        assert_eq!(scroll, 2, "a tall row above pushes the top down");
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
        };
        let events = [mk(0, false, 1), mk(10, false, 1), mk(11, false, 1), mk(100, true, 2)];
        let b = bucket_events(&events, 11);
        assert_eq!((b[0], b[1], b[10]), ((1, false, true), (2, false, false), (1, true, true)));
    }

    #[test]
    fn body_text_reads_each_harness_shape() {
        // Claude assistant: the text block, then the tool input as JSON.
        let claude = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Let me check."},{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}"#;
        assert_eq!(body_text(claude), "Let me check.\n\n{\n  \"command\": \"ls\"\n}");
        // Claude tool result: content as a string, then as an array of text blocks.
        let plain = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"file.txt"}]}}"#;
        assert_eq!(body_text(plain), "file.txt");
        let blocks = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":[{"type":"text","text":"out 1"},{"type":"text","text":"out 2"}]}]}}"#;
        assert_eq!(body_text(blocks), "out 1\n\nout 2");
        // Codex response item.
        let codex = r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Done."}]}}"#;
        assert_eq!(body_text(codex), "Done.");
        // Pi tool call: arguments as JSON.
        let pi = r#"{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","id":"c1","name":"read","arguments":{"path":"a.rs"}}]}}"#;
        assert_eq!(body_text(pi), "{\n  \"path\": \"a.rs\"\n}");
    }

    #[test]
    fn body_text_dedupes_and_falls_back_to_pretty() {
        let dup = r#"{"message":{"role":"user","content":"same"},"toolUseResult":{"stdout":"same","stderr":""}}"#;
        assert_eq!(body_text(dup), "same");
        assert_eq!(body_text(r#"{"type":"x"}"#), "{\n  \"type\": \"x\"\n}");
        assert_eq!(body_text("not json"), "not json");
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
        state.goto(2);
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
        state.goto(2);
        press(&mut state, KeyCode::Char('h'));
        assert!(!shown_rows(&state).contains(&"Ev(4)".to_string()));

        state.install_trajectory(three_calls(), None, vec![]);
        assert!(!shown_rows(&state).contains(&"Ev(4)".to_string()), "a live poll keeps the manual collapse");
    }
}
