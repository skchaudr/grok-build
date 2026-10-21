//! Live view of running sessions, built on hook receipts. Process identity (`alive`, the
//! ancestry walk) comes first; the receipt queries and the state machine follow.

use serde::{Deserialize, Serialize};

use crate::index::Result;
use crate::query::{SessionRow, Status};
use crate::TraceIndex;

/// Interpreters a hook is run through; the harness is the first ancestor that is not one of these.
const SHELLS: [&str; 5] = ["sh", "bash", "zsh", "dash", "fish"];
/// Ancestors examined before giving up.
const MAX_WALK: usize = 64;

/// One process of an ancestry chain: `(pid, ppid, comm)`.
pub type Proc = (i64, i64, String);

fn is_shell(comm: &str) -> bool {
    let base = comm.rsplit('/').next().unwrap_or(comm);
    SHELLS.contains(&base.strip_prefix('-').unwrap_or(base))
}

/// The first process of `chain` (nearest ancestor first) whose `comm` is not a shell.
/// A chain of shells only gives `None`.
pub fn harness_in_chain(chain: &[Proc]) -> Option<i64> {
    chain.iter().find(|(_, _, comm)| !is_shell(comm)).map(|(pid, _, _)| *pid)
}

/// (ppid, comm, start in Unix seconds, is zombie) of a process.
#[cfg(target_os = "macos")]
fn proc_info(pid: i64) -> Option<(i64, String, i64, bool)> {
    let pid32 = i32::try_from(pid).ok().filter(|p| *p > 0)?;
    // SAFETY: `proc_pidinfo` fills at most `size` bytes of the zeroed struct we pass.
    let info = unsafe {
        let mut info: libc::proc_bsdinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let got = libc::proc_pidinfo(pid32, libc::PROC_PIDTBSDINFO, 0, std::ptr::addr_of_mut!(info).cast(), size);
        if got != size {
            return None;
        }
        info
    };
    let text = |raw: &[libc::c_char]| {
        let bytes: Vec<u8> = raw.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
        String::from_utf8_lossy(&bytes).into_owned()
    };
    let name = text(&info.pbi_name);
    let comm = if name.is_empty() { text(&info.pbi_comm) } else { name };
    Some((i64::from(info.pbi_ppid), comm, i64::try_from(info.pbi_start_tvsec).ok()?, info.pbi_status == libc::SZOMB))
}

#[cfg(not(target_os = "macos"))]
fn proc_info(_pid: i64) -> Option<(i64, String, i64, bool)> {
    None
}

/// Start time of a process in Unix seconds.
pub fn proc_start(pid: i64) -> Option<i64> {
    proc_info(pid).map(|(_, _, start, _)| start)
}

/// The harness above the hook: walks up from this process's parent, past shells.
/// Returns `(pid, start)`; `None` when the chain is unreadable or all shells.
pub fn find_harness() -> Option<(i64, i64)> {
    #[cfg(unix)]
    {
        // SAFETY: getppid has no preconditions.
        let mut pid = i64::from(unsafe { libc::getppid() });
        let mut chain: Vec<Proc> = Vec::new();
        let mut starts = Vec::new();
        while pid > 1 && chain.len() < MAX_WALK && !chain.iter().any(|(p, _, _)| *p == pid) {
            let Some((ppid, comm, start, _)) = proc_info(pid) else { break };
            chain.push((pid, ppid, comm));
            starts.push(start);
            pid = ppid;
        }
        let harness = harness_in_chain(&chain)?;
        let i = chain.iter().position(|(p, _, _)| *p == harness)?;
        Some((harness, starts[i]))
    }
    #[cfg(not(unix))]
    None
}

/// The process exists, is not a zombie, and is the one that started at `start` (a reused pid has a different start).
pub fn alive(pid: i64, start: i64) -> bool {
    #[cfg(unix)]
    {
        let Ok(pid32) = i32::try_from(pid) else { return false };
        if pid32 <= 0 {
            return false;
        }
        // SAFETY: signal 0 only checks that the process can be signalled.
        let exists = unsafe { libc::kill(pid32, 0) } == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
        // A zombie has exited; its pid and start linger until the parent reaps it.
        exists && proc_info(pid).is_some_and(|(_, _, s, zombie)| s == start && !zombie)
    }
    #[cfg(not(unix))]
    {
        let _ = (pid, start);
        false
    }
}

// ---- receipts as the live signal ----

/// One hook receipt as the live screen reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LiveReceipt {
    pub id: i64,
    pub session_id: Option<String>,
    pub native_session: Option<String>,
    pub agent_id: Option<String>,
    pub event: String,
    pub tool: Option<String>,
    pub detail: Option<String>,
    pub tool_use_id: Option<String>,
    pub at_ms: i64,
}

/// A turn boundary: what the session did when a receipt of this kind arrived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boundary {
    Idle,
    Prompt,
    End,
}

/// Subagent lifetime events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Child {
    Start,
    Stop,
}

/// Every event name `boundary` can map; the SQL prefilter (it still decides on `detail`).
pub const BOUNDARY_EVENTS: &[&str] = &[
    "SessionStart",
    "Stop",
    "StopFailure",
    "Notification",
    "UserPromptSubmit",
    "SessionEnd",
    "session_start",
    "agent_end",
    "agent_start",
    "session_shutdown",
    "sessionStart",
    "stop",
    "beforeSubmitPrompt",
    "sessionEnd",
];

/// Every event name `child_kind` can map.
pub const CHILD_EVENTS: &[&str] = &["SubagentStart", "SubagentStop", "subagentStart", "subagentStop"];

/// The turn boundary an event is, per source. Event names are case-sensitive per source.
/// A Claude `SessionStart` with detail `compact` fires mid-turn and is no boundary;
/// a Claude `Notification` is one only as `idle_prompt`.
pub fn boundary(source: &str, event: &str, detail: Option<&str>) -> Option<Boundary> {
    use Boundary::{End, Idle, Prompt};
    match (source, event) {
        ("claude", "SessionStart") => (detail != Some("compact")).then_some(Idle),
        ("claude", "Stop" | "StopFailure") => Some(Idle),
        ("claude", "Notification") => (detail == Some("idle_prompt")).then_some(Idle),
        ("claude", "UserPromptSubmit") => Some(Prompt),
        ("claude", "SessionEnd") => Some(End),
        ("pi", "session_start" | "agent_end") => Some(Idle),
        ("pi", "agent_start") => Some(Prompt),
        ("pi", "session_shutdown") => Some(End),
        ("cursor", "sessionStart" | "stop") => Some(Idle),
        ("cursor", "beforeSubmitPrompt") => Some(Prompt),
        ("cursor", "sessionEnd") => Some(End),
        _ => None,
    }
}

/// Subagent start/stop events: Claude `SubagentStart/Stop`, Cursor `subagentStart/Stop`.
pub fn child_kind(source: &str, event: &str) -> Option<Child> {
    match (source, event) {
        ("claude", "SubagentStart") | ("cursor", "subagentStart") => Some(Child::Start),
        ("claude", "SubagentStop") | ("cursor", "subagentStop") => Some(Child::Stop),
        _ => None,
    }
}

/// Every event name that maps to `Boundary::End`; the SQL prefilter for the newest-`End` lookup.
pub const END_EVENTS: &[&str] = &["SessionEnd", "session_shutdown", "sessionEnd"];

// ---- event normalizer ----

/// What a receipt means to the state machine. Ids, tool and detail stay on the receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ev {
    Idle { failed: bool },
    Prompt,
    End { failed: bool },
    Pre,
    Post,
    Tick,
    Ask,
    ChildStart,
    ChildStop,
    CompactStart,
    CompactEnd,
}

/// Notification types that mean the harness waits on the user (`idle_prompt` is a boundary).
const ASK_NOTIFICATIONS: [&str; 4] = ["permission_prompt", "elicitation_dialog", "elicitation_url_dialog", "agent_needs_input"];

/// The normalizer table of `SPEC-live.md`. Anything not in it is `None`.
pub fn norm(source: &str, event: &str, detail: Option<&str>) -> Option<Ev> {
    if let Some(b) = boundary(source, event, detail) {
        return Some(match b {
            Boundary::Idle => Ev::Idle { failed: source == "claude" && event == "StopFailure" },
            Boundary::Prompt => Ev::Prompt,
            Boundary::End => Ev::End { failed: false },
        });
    }
    if let Some(c) = child_kind(source, event) {
        return Some(if c == Child::Start { Ev::ChildStart } else { Ev::ChildStop });
    }
    match (source, event) {
        ("claude", "PreToolUse") | ("pi", "tool_execution_start") => Some(Ev::Pre),
        ("cursor", "preToolUse" | "beforeShellExecution" | "beforeMCPExecution" | "beforeReadFile") => Some(Ev::Pre),
        ("claude", "PostToolUse" | "PostToolUseFailure" | "PermissionDenied") | ("pi", "tool_execution_end") => Some(Ev::Post),
        ("cursor", "postToolUse" | "postToolUseFailure" | "afterShellExecution" | "afterMCPExecution") => Some(Ev::Post),
        ("cursor", "afterFileEdit" | "afterAgentThought" | "afterAgentResponse") => Some(Ev::Tick),
        ("claude", "PermissionRequest") => Some(Ev::Ask),
        ("claude", "Notification") => detail.is_some_and(|d| ASK_NOTIFICATIONS.contains(&d)).then_some(Ev::Ask),
        ("claude", "PreCompact") | ("cursor", "preCompact") => Some(Ev::CompactStart),
        ("claude", "PostCompact") => Some(Ev::CompactEnd),
        _ => None,
    }
}

/// Whether a `Post` closes only the `Pre` with its id (strict) or also every older open `Pre` (close-older).
///
/// Pi: true (`toolCallId` on both events, no hook denial; MAP "Live captures" row 9).
/// Claude: false. Row 1 says ids are present, but row 5 says a hook-denied `Pre` gets no `PostToolUseFailure`
/// or `PermissionDenied`, so orphans exist.
/// Cursor: false. Rows 12 (ids on every pair) is yes, but row 5 covered Claude only and there is no Cursor
/// denial capture, so orphans are not ruled out. Rows not on the mini are the fallback.
pub fn strict_ids(source: &str) -> bool {
    source == "pi"
}

// ---- durations ----

/// "0s".."59s", "3m", "2h", "5d", for the age column. Unlike the pager's `fmt_age` it never prints "now".
pub fn fmt_age(ago_ms: i64) -> String {
    let s = ago_ms.max(0) / 1000;
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        3600..86_400 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86_400),
    }
}

/// "0s".."59s", "Nm", "Nh": durations inside the state text.
pub fn fmt_dur(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        _ => format!("{}h", s / 3600),
    }
}

// ---- rows ----

/// Bumped on any `LiveRow` change. `extra_tools` was added before anything shipped, so it stays 1.
pub const LIVE_SCHEMA: u32 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveState {
    Waiting,
    Compacting,
    Tool,
    Prompt,
    #[default]
    Working,
}

/// One row of the live screen: state and numbers, no glyphs and no durations in `text`.
/// `text` is the data part: `<tool> <detail>`, `permission: <tool> <detail>`, or `SessionRow.now`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LiveRow {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub state: LiveState,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub age_ms: i64,
    #[serde(default)]
    pub since_ms: Option<i64>,
    #[serde(default)]
    pub quiet_ms: i64,
    #[serde(default)]
    pub bg: u32,
    #[serde(default)]
    pub bg_quiet: bool,
    /// Open tools beyond the newest one (the `+N`); only in state `Tool`.
    #[serde(default)]
    pub extra_tools: u32,
    #[serde(default)]
    pub failed: bool,
    #[serde(default)]
    pub pid_unknown: bool,
    #[serde(default)]
    pub native_id: Option<String>,
    #[serde(default)]
    pub context_tokens: Option<i64>,
    #[serde(default)]
    pub context_window: Option<i64>,
    #[serde(default)]
    pub host: Option<String>,
}

/// A glyph, the full state text and the ordered marks (`✗`, `?`, `pid?`). The pager adds the age and `⌖`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rendered {
    pub glyph: &'static str,
    pub text: String,
    pub marks: Vec<&'static str>,
}

const WARN_MS: i64 = 30_000;
const QUIET_MS: i64 = 120_000;
const IDLE_DEBOUNCE_MS: i64 = 2_000;
const BG_QUIET_MS: i64 = 600_000;

fn join(parts: &[String]) -> String {
    parts.iter().filter(|p| !p.is_empty()).cloned().collect::<Vec<_>>().join(" · ")
}

fn bg_label(row: &LiveRow) -> String {
    if row.bg == 0 { String::new() } else { format!("{} bg{}", row.bg, if row.bg_quiet { "?" } else { "" }) }
}

/// Glyph, state text and marks from the row's numbers plus `extra_ms` (0 for local rows,
/// the age of the fetch for remote ones). The one render path for both.
pub fn render(row: &LiveRow, extra_ms: i64) -> Rendered {
    let add = |v: i64| (v + extra_ms).max(0);
    let since = add(row.since_ms.unwrap_or(0));
    let quiet = add(row.quiet_ms);
    let (glyph, text) = match row.state {
        LiveState::Waiting => (if since >= WARN_MS { "⚠?" } else { "⚠" }, row.text.clone()),
        LiveState::Compacting => ("⟳", format!("compacting · {}", fmt_dur(since))),
        LiveState::Tool => {
            let mut t = join(&[row.text.clone(), fmt_dur(since)]);
            if row.extra_tools > 0 {
                t.push_str(&format!(" +{}", row.extra_tools));
            }
            (if since >= WARN_MS { "⏳" } else { "▶" }, t)
        }
        LiveState::Prompt => ("○", join(&["your turn".into(), bg_label(row)])),
        LiveState::Working if quiet >= QUIET_MS => ("◌", join(&[bg_label(row), "quiet".into(), row.text.clone()])),
        LiveState::Working => ("●", join(&[bg_label(row), row.text.clone()])),
    };
    let mut marks = Vec::new();
    if row.failed {
        marks.push("✗");
    }
    if matches!(row.state, LiveState::Waiting | LiveState::Compacting | LiveState::Tool) && quiet >= QUIET_MS {
        marks.push("?");
    }
    if row.pid_unknown {
        marks.push("pid?");
    }
    Rendered { glyph, text, marks }
}

// ---- the fold ----

/// A `Pre` not yet closed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[doc(hidden)]
pub struct OpenTool {
    pub id: Option<String>,
    pub tool: Option<String>,
    pub detail: Option<String>,
    pub at_ms: i64,
}

/// One session folded: what is open, and its row (`None` = gone).
#[derive(Clone, Debug, Default)]
#[doc(hidden)]
pub struct Folded {
    pub open_tools: Vec<OpenTool>,
    pub open_children: Vec<String>,
    pub row: Option<LiveRow>,
}

/// `Ask` sorts after anything else at the same millisecond, so insert order cannot hide `Waiting`.
fn sort_key(ev: Ev, r: &LiveReceipt) -> (i64, u8, i64) {
    (r.at_ms, u8::from(ev == Ev::Ask), r.id)
}

/// The receipts the fold reads. Parent (`agent == None`): own-thread rows plus every child start/stop.
/// Child: its `agent_id` rows minus child start/stop and boundaries (a subagent has no turns).
fn prepare<'a>(source: &str, receipts: &'a [LiveReceipt], agent: Option<&str>) -> Vec<(Ev, &'a LiveReceipt)> {
    let mut out: Vec<(Ev, &LiveReceipt)> = receipts
        .iter()
        .filter_map(|r| Some((norm(source, &r.event, r.detail.as_deref())?, r)))
        .filter(|(ev, r)| match agent {
            None => r.agent_id.is_none() || matches!(ev, Ev::ChildStart | Ev::ChildStop),
            Some(a) => r.agent_id.as_deref() == Some(a) && !matches!(ev, Ev::ChildStart | Ev::ChildStop | Ev::Idle { .. } | Ev::Prompt | Ev::End { .. }),
        })
        .collect();
    out.sort_by_key(|(ev, r)| sort_key(*ev, r));
    out
}

#[derive(Default)]
struct AskRun {
    first_at: i64,
    /// Tool and detail of the newest `Ask` in the run that had a tool.
    tool: Option<(String, Option<String>)>,
    /// Notification type (or event) of the newest `Ask`.
    note: String,
}

#[derive(Default)]
struct Run {
    open_tools: Vec<OpenTool>,
    open_children: Vec<String>,
    stopped_children: Vec<String>,
    last: Option<(Ev, i64)>,
    ask: Option<AskRun>,
    /// The newest boundary is an `End`.
    ended: bool,
    /// The newest `Idle`/`End` failed.
    failed: bool,
}

fn run(items: &[(Ev, &LiveReceipt)], strict: bool) -> Run {
    let mut s = Run::default();
    for (ev, r) in items {
        match ev {
            Ev::ChildStart => {
                if let Some(a) = &r.agent_id {
                    s.stopped_children.retain(|c| c != a);
                    if !s.open_children.contains(a) {
                        s.open_children.push(a.clone());
                    }
                }
                continue;
            }
            Ev::ChildStop => {
                if let Some(a) = &r.agent_id {
                    s.open_children.retain(|c| c != a);
                    s.stopped_children.push(a.clone());
                }
                continue;
            }
            // A `Tick` right after an `Idle` is not the new last event (Cursor sends `afterAgentResponse` after `stop`).
            Ev::Tick if matches!(s.last, Some((Ev::Idle { .. }, at)) if r.at_ms - at < IDLE_DEBOUNCE_MS) => continue,
            _ => {}
        }
        match ev {
            Ev::Idle { failed } | Ev::End { failed } => {
                s.open_tools.clear();
                s.failed = *failed;
                s.ended = matches!(ev, Ev::End { .. });
                if s.ended {
                    s.open_children.clear();
                }
            }
            Ev::Prompt => {
                s.open_tools.clear();
                s.ended = false;
            }
            Ev::Pre => {
                if !strict && r.tool_use_id.is_none() {
                    s.open_tools.retain(|t| !(t.id.is_none() && t.tool == r.tool));
                }
                s.open_tools.push(OpenTool { id: r.tool_use_id.clone(), tool: r.tool.clone(), detail: r.detail.clone(), at_ms: r.at_ms });
            }
            Ev::Post => {
                let hit = match &r.tool_use_id {
                    Some(id) => s.open_tools.iter().position(|t| t.id.as_deref() == Some(id)),
                    None => s.open_tools.iter().position(|t| t.tool == r.tool),
                };
                if let Some(i) = hit {
                    if strict {
                        s.open_tools.remove(i);
                    } else {
                        s.open_tools.drain(..=i);
                    }
                }
            }
            Ev::Ask => {
                let a = match (&mut s.ask, s.last) {
                    (Some(a), Some((Ev::Ask, _))) => a,
                    (slot, _) => slot.insert(AskRun { first_at: r.at_ms, ..Default::default() }),
                };
                if let Some(t) = &r.tool {
                    a.tool = Some((t.clone(), r.detail.clone()));
                }
                a.note = r.detail.clone().unwrap_or_else(|| r.event.clone());
            }
            Ev::Tick | Ev::CompactStart | Ev::CompactEnd | Ev::ChildStart | Ev::ChildStop => {}
        }
        if *ev != Ev::Ask {
            s.ask = None;
        }
        s.last = Some((*ev, r.at_ms));
    }
    s
}

fn words(parts: &[Option<&str>]) -> String {
    parts.iter().flatten().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(" ")
}

/// State, text, `since` and `extra_tools` from a finished run.
fn state_of(s: &Run, now: i64, working_text: &str) -> (LiveState, String, Option<i64>, u32) {
    match s.last {
        Some((Ev::Ask, _)) => {
            let a = s.ask.as_ref();
            let text = match a.and_then(|a| a.tool.as_ref()) {
                Some((tool, detail)) => format!("permission: {}", words(&[Some(tool), detail.as_deref()])),
                None => a.map(|a| a.note.clone()).unwrap_or_default(),
            };
            (LiveState::Waiting, text, a.map(|a| now - a.first_at), 0)
        }
        Some((Ev::CompactStart, at)) => (LiveState::Compacting, String::new(), Some(now - at), 0),
        _ => {
            if let Some(t) = s.open_tools.last() {
                let text = words(&[Some(t.tool.as_deref().unwrap_or("tool")), t.detail.as_deref()]);
                return (LiveState::Tool, text, Some(now - t.at_ms), (s.open_tools.len() - 1) as u32);
            }
            match s.last {
                Some((Ev::Idle { .. }, at)) if now - at >= IDLE_DEBOUNCE_MS => (LiveState::Prompt, String::new(), None, 0),
                _ => (LiveState::Working, working_text.to_string(), None, 0),
            }
        }
    }
}

/// `now − max(present values)`, clamped at 0; none present gives 0.
fn age_of(now: i64, times: &[Option<i64>]) -> i64 {
    times.iter().flatten().max().map_or(0, |t| (now - t).max(0))
}

fn agent_of(native_id: Option<&str>) -> Option<&str> {
    native_id?.rsplit('/').next()?.strip_prefix("agent-")
}

/// Folds one session. `receipts` is `turn_receipts` (any superset works); `kids` are its indexed children.
#[doc(hidden)]
pub fn fold_session(source: &str, row: &SessionRow, receipts: &[LiveReceipt], kids: &[SessionRow], now: i64, strict: bool) -> Folded {
    let items = prepare(source, receipts, None);
    let s = run(&items, strict);
    let open_tools = s.open_tools.clone();
    let open_children = s.open_children.clone();
    if s.ended {
        return Folded { open_tools, open_children, row: None };
    }
    let (state, text, since_ms, extra_tools) = state_of(&s, now, row.now.as_deref().unwrap_or(""));
    // Last activity per open child: its receipts (the start included) and its mapped session row.
    let bg_quiet = open_children.iter().any(|a| {
        let kid = kids.iter().find(|k| agent_of(k.native_id.as_deref()) == Some(a.as_str()));
        let mut times: Vec<Option<i64>> = receipts.iter().filter(|r| r.agent_id.as_deref() == Some(a.as_str())).map(|r| Some(r.at_ms)).collect();
        times.extend([kid.and_then(|k| k.last_event_at), kid.and_then(|k| k.mtime_ms)]);
        age_of(now, &times) >= BG_QUIET_MS
    });
    let age = age_of(now, &[s.last.map(|(_, at)| at), row.last_event_at, row.mtime_ms]);
    let live = LiveRow {
        session_id: row.id.clone(),
        source: row.source.clone(),
        title: row.title.clone(),
        label: row.label.clone(),
        state,
        text,
        age_ms: age,
        since_ms,
        quiet_ms: age,
        bg: open_children.len() as u32,
        bg_quiet,
        extra_tools,
        failed: row.failed || s.failed,
        pid_unknown: row.harness_pid.is_none(),
        native_id: row.native_id.clone(),
        context_tokens: row.context_tokens,
        context_window: row.context_window,
        ..Default::default()
    };
    Folded { open_tools, open_children, row: Some(live) }
}

/// Child rows of a folded parent. A child is a session of `kids` with `parent_id == parent.id`, matched to
/// hook `agent_id`s by the last path component of its `native_id` being exactly `agent-<agent_id>`.
/// A child with receipts of its own folds them; a Pre from before the parent's boundary is not returned by
/// `turn_receipts`, so a child may show `●` across a parent `Stop`.
/// A child without any shows `● <now>` from its transcript while it is not stopped, is open (or no start/stop
/// exists for it) and is not `Completed`.
#[doc(hidden)]
pub fn fold_children(source: &str, parent: &SessionRow, receipts: &[LiveReceipt], kids: &[SessionRow], open: &[String], now: i64, strict: bool) -> Vec<LiveRow> {
    let stopped = |a: &str| receipts.iter().any(|r| r.agent_id.as_deref() == Some(a) && norm(source, &r.event, r.detail.as_deref()) == Some(Ev::ChildStop));
    let started = |a: &str| receipts.iter().any(|r| r.agent_id.as_deref() == Some(a) && norm(source, &r.event, r.detail.as_deref()) == Some(Ev::ChildStart));
    let mut out = Vec::new();
    for kid in kids.iter().filter(|k| k.parent_id.as_deref() == Some(parent.id.as_str())) {
        let agent = agent_of(kid.native_id.as_deref());
        let is_open = agent.is_some_and(|a| open.iter().any(|o| o == a));
        if agent.is_some_and(stopped) && !is_open {
            continue;
        }
        let items = agent.map(|a| prepare(source, receipts, Some(a))).unwrap_or_default();
        if items.is_empty() {
            let known = agent.is_some_and(|a| started(a) || stopped(a));
            if (known && !is_open) || kid.status == Status::Completed {
                continue;
            }
        }
        let s = run(&items, strict);
        let (state, text, since_ms, extra_tools) = state_of(&s, now, kid.now.as_deref().unwrap_or(""));
        let age = age_of(now, &[s.last.map(|(_, at)| at), kid.last_event_at, kid.mtime_ms]);
        out.push(LiveRow {
            session_id: kid.id.clone(),
            parent_id: Some(parent.id.clone()),
            source: kid.source.clone(),
            title: kid.title.clone(),
            label: kid.label.clone(),
            state,
            text,
            age_ms: age,
            since_ms,
            quiet_ms: age,
            extra_tools,
            failed: kid.failed || s.failed,
            pid_unknown: parent.harness_pid.is_none(),
            native_id: kid.native_id.clone(),
            context_tokens: kid.context_tokens,
            context_window: kid.context_window,
            ..Default::default()
        });
    }
    out
}

fn rank(r: &LiveRow) -> (u8, i64, &str) {
    let rank = match r.state {
        LiveState::Waiting => 0,
        LiveState::Prompt => 1,
        _ => 2,
    };
    (rank, r.age_ms, r.session_id.as_str())
}

/// Roots by `(rank, age, session_id)`; each root's children directly under it by the same key.
/// A group is a root followed by its children.
#[doc(hidden)]
pub fn arrange(mut groups: Vec<Vec<LiveRow>>) -> Vec<LiveRow> {
    groups.retain(|g| !g.is_empty());
    for g in &mut groups {
        g[1..].sort_by(|a, b| rank(a).cmp(&rank(b)));
    }
    groups.sort_by(|a, b| rank(&a[0]).cmp(&rank(&b[0])));
    groups.into_iter().flatten().collect()
}

/// The live screen's rows: `live_sessions` → not-alive dropped (a missing pid stays) → same-process supersede
/// for claude/pi → `turn_receipts` → fold → children attached → ordered.
pub fn fold_all(index: &TraceIndex, now_ms: i64, alive: impl Fn(i64, i64) -> bool) -> Result<Vec<LiveRow>> {
    let mut sessions: Vec<SessionRow> = index
        .live_sessions()?
        .into_iter()
        .filter(|s| match (s.harness_pid, s.harness_start) {
            (Some(pid), Some(start)) => alive(pid, start),
            _ => true,
        })
        .collect();
    // Of sessions sharing one process, only the one with the newest receipt stays.
    type Process = (String, i64, Option<i64>);
    let mut newest: std::collections::HashMap<Process, ((i64, i64), String)> = std::collections::HashMap::new();
    for s in &sessions {
        let (Some(pid), true) = (s.harness_pid, matches!(s.source.as_str(), "claude" | "pi")) else { continue };
        let key = index.last_receipt_key(&s.id)?.unwrap_or((i64::MIN, 0));
        let slot = newest.entry((s.source.clone(), pid, s.harness_start)).or_insert((key, s.id.clone()));
        if key > slot.0 {
            *slot = (key, s.id.clone());
        }
    }
    sessions.retain(|s| match (s.harness_pid, matches!(s.source.as_str(), "claude" | "pi")) {
        (Some(pid), true) => newest.get(&(s.source.clone(), pid, s.harness_start)).is_some_and(|(_, id)| *id == s.id),
        _ => true,
    });
    let mut groups = Vec::new();
    for s in &sessions {
        let receipts = index.turn_receipts(&s.id)?;
        let kids = if matches!(s.source.as_str(), "claude" | "cursor") && !s.id.starts_with(crate::receipts::RECEIPT_ONLY_PREFIX) { index.children(&s.id)? } else { Vec::new() };
        let strict = strict_ids(&s.source);
        let folded = fold_session(&s.source, s, &receipts, &kids, now_ms, strict);
        let Some(root) = folded.row else { continue };
        let mut group = vec![root];
        group.extend(fold_children(&s.source, s, &receipts, &kids, &folded.open_children, now_ms, strict));
        groups.push(group);
    }
    Ok(arrange(groups))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(comms: &[&str]) -> Vec<Proc> {
        comms.iter().enumerate().map(|(i, c)| (100 + i as i64, 101 + i as i64, (*c).to_string())).collect()
    }

    #[test]
    fn walk_skips_shells_by_basename() {
        // The real Claude chain: sh, /bin/zsh, claude, -zsh, herdr.
        assert_eq!(harness_in_chain(&chain(&["sh", "/bin/zsh", "claude", "-zsh", "herdr"])), Some(102));
        assert_eq!(harness_in_chain(&chain(&["sh", "-zsh", "Cursor Helper (Plugin)"])), Some(102));
        assert_eq!(harness_in_chain(&chain(&["claude"])), Some(100));
        assert_eq!(harness_in_chain(&chain(&["sh", "-bash", "fish"])), None);
        assert_eq!(harness_in_chain(&[]), None);
    }

    #[test]
    fn boundary_table() {
        use Boundary::*;
        let b = boundary;
        assert_eq!(b("claude", "SessionStart", Some("startup")), Some(Idle));
        assert_eq!(b("claude", "SessionStart", None), Some(Idle));
        assert_eq!(b("claude", "SessionStart", Some("compact")), None, "mid-turn after auto-compaction");
        assert_eq!(b("claude", "Notification", Some("idle_prompt")), Some(Idle));
        assert_eq!(b("claude", "Notification", Some("permission_prompt")), None);
        assert_eq!(b("claude", "Notification", None), None);
        assert_eq!((b("claude", "Stop", None), b("claude", "StopFailure", None)), (Some(Idle), Some(Idle)));
        assert_eq!((b("claude", "UserPromptSubmit", None), b("claude", "SessionEnd", None)), (Some(Prompt), Some(End)));
        assert_eq!((b("pi", "agent_end", None), b("pi", "agent_start", None), b("pi", "session_shutdown", None)), (Some(Idle), Some(Prompt), Some(End)));
        assert_eq!(b("pi", "session_start", None), Some(Idle));
        assert_eq!((b("cursor", "stop", None), b("cursor", "beforeSubmitPrompt", None), b("cursor", "sessionEnd", None)), (Some(Idle), Some(Prompt), Some(End)));
        assert_eq!(b("cursor", "sessionStart", None), Some(Idle));
        // Case-sensitive per source; tools and ticks are no boundary.
        assert_eq!((b("cursor", "Stop", None), b("claude", "stop", None), b("claude", "PreToolUse", None), b("codex", "Stop", None)), (None, None, None, None));
        assert_eq!(b("cursor", "afterAgentResponse", None), None);
    }

    #[test]
    fn boundary_events_cover_the_table() {
        for (source, event) in [("claude", "Notification"), ("claude", "Stop"), ("pi", "agent_end"), ("cursor", "sessionEnd")] {
            assert!(BOUNDARY_EVENTS.contains(&event), "{source} {event}");
        }
        // Every name the table can map is prefiltered in.
        for source in ["claude", "pi", "cursor"] {
            for event in BOUNDARY_EVENTS {
                let _ = boundary(source, event, Some("idle_prompt"));
            }
        }
        let mapped: Vec<&str> = BOUNDARY_EVENTS
            .iter()
            .copied()
            .filter(|e| ["claude", "pi", "cursor"].iter().any(|s| boundary(s, e, Some("idle_prompt")).is_some()))
            .collect();
        assert_eq!(mapped.len(), BOUNDARY_EVENTS.len(), "no dead names in the prefilter");
    }

    #[test]
    fn child_kinds_are_case_sensitive_per_source() {
        assert_eq!(child_kind("claude", "SubagentStart"), Some(Child::Start));
        assert_eq!(child_kind("claude", "SubagentStop"), Some(Child::Stop));
        assert_eq!(child_kind("cursor", "subagentStart"), Some(Child::Start));
        assert_eq!(child_kind("cursor", "subagentStop"), Some(Child::Stop));
        assert_eq!((child_kind("claude", "subagentStart"), child_kind("cursor", "SubagentStop"), child_kind("pi", "SubagentStart")), (None, None, None));
        for e in CHILD_EVENTS {
            assert!(["claude", "cursor"].iter().any(|s| child_kind(s, e).is_some()), "{e}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn alive_checks_pid_and_start() {
        let me = i64::from(std::process::id());
        let start = proc_start(me);
        if cfg!(target_os = "macos") {
            let start = start.expect("own start");
            assert!(alive(me, start));
            assert!(!alive(me, start + 1000), "start mismatch = reused pid");
        }
        assert!(!alive(0, 0));
        assert!(!alive(-5, 0));
        assert!(!alive(i64::from(i32::MAX), 1));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_zombie_is_not_alive() {
        let mut child = std::process::Command::new("sh").args(["-c", "sleep 0.3"]).spawn().unwrap();
        let pid = i64::from(child.id());
        let start = proc_start(pid).expect("child start");
        assert!(alive(pid, start));
        // Exited but not yet waited for: the pid is still in the process table (a zombie, or already unreadable).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while alive(pid, start) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!alive(pid, start), "an exited, unreaped child is not alive");
        child.wait().unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn find_harness_resolves_something_above_the_test() {
        // Under cargo test the parent chain exists; only shape is asserted.
        if let Some((pid, start)) = find_harness() {
            assert!(pid > 1 && start > 0 && alive(pid, start));
        }
    }
}

/// One test per row of the sequence table in `SPEC-live.md`. `@` times are seconds. Rows 7, 12, 20, 32, 37, 38,
/// 46, 47 and 48 go through a real index in `tests/live.rs`; 40-45 and 53 belong to the remote tests.
/// Row 52 is `tests::walk_skips_shells_by_basename`.
#[cfg(test)]
mod seq {
    use super::*;

    fn ms(s: f64) -> i64 {
        (s * 1000.0).round() as i64
    }

    fn rc(event: &str, at: f64) -> LiveReceipt {
        LiveReceipt { event: event.into(), at_ms: ms(at), ..Default::default() }
    }

    trait B: Sized {
        fn tool(self, t: &str) -> Self;
        fn id(self, i: &str) -> Self;
        fn det(self, d: &str) -> Self;
        fn agent(self, a: &str) -> Self;
    }
    impl B for LiveReceipt {
        fn tool(mut self, t: &str) -> Self {
            self.tool = Some(t.into());
            self
        }
        fn id(mut self, i: &str) -> Self {
            self.tool_use_id = Some(i.into());
            self
        }
        fn det(mut self, d: &str) -> Self {
            self.detail = Some(d.into());
            self
        }
        fn agent(mut self, a: &str) -> Self {
            self.agent_id = Some(a.into());
            self
        }
    }

    const PROMPT: &str = "UserPromptSubmit";
    const PRE: &str = "PreToolUse";
    const POST: &str = "PostToolUse";

    struct Seq {
        source: &'static str,
        rs: Vec<LiveReceipt>,
        row: SessionRow,
        kids: Vec<SessionRow>,
    }

    /// Receipts get ids in the listed order.
    fn seq(source: &'static str, rs: Vec<LiveReceipt>) -> Seq {
        let rs = rs.into_iter().enumerate().map(|(i, mut r)| {
            r.id = i as i64 + 1;
            r
        });
        let row = SessionRow {
            id: "p".into(),
            source: source.into(),
            now: Some("w".into()),
            harness_pid: Some(1),
            harness_start: Some(1),
            ..Default::default()
        };
        Seq { source, rs: rs.collect(), row, kids: Vec::new() }
    }

    fn claude(rs: Vec<LiveReceipt>) -> Seq {
        seq("claude", rs)
    }

    fn kid(agent: &str) -> SessionRow {
        SessionRow {
            id: format!("k-{agent}"),
            source: "claude".into(),
            parent_id: Some("p".into()),
            native_id: Some(format!("proj/sess/subagents/agent-{agent}")),
            now: Some("kidnow".into()),
            ..Default::default()
        }
    }

    impl Seq {
        fn fold_in(&self, now: f64, strict: bool) -> Folded {
            fold_session(self.source, &self.row, &self.rs, &self.kids, ms(now), strict)
        }
        fn fold(&self, now: f64) -> Folded {
            self.fold_in(now, strict_ids(self.source))
        }
        fn show(&self, now: f64) -> String {
            show(&self.fold(now))
        }
        fn show_in(&self, now: f64, strict: bool) -> String {
            show(&self.fold_in(now, strict))
        }
        fn rows(&self, now: f64) -> Vec<LiveRow> {
            let f = self.fold(now);
            let mut g = vec![f.row.expect("on screen")];
            g.extend(fold_children(self.source, &self.row, &self.rs, &self.kids, &f.open_children, ms(now), strict_ids(self.source)));
            g
        }
    }

    fn row(f: &Folded) -> &LiveRow {
        f.row.as_ref().expect("on screen")
    }

    fn show(f: &Folded) -> String {
        match &f.row {
            None => "gone".into(),
            Some(r) => {
                let v = render(r, 0);
                format!("{} {}", v.glyph, v.text)
            }
        }
    }

    fn marks(f: &Folded) -> Vec<&'static str> {
        render(row(f), 0).marks
    }

    #[test]
    fn seq_01_post_inserted_before_pre_at_earlier_time() {
        let s = claude(vec![rc(PROMPT, 0.0), rc(POST, 1.05).tool("A").id("A"), rc(PRE, 1.00).tool("A").id("A")]);
        let f = s.fold(2.0);
        assert_eq!(show(&f), "● w");
        assert!(f.open_tools.is_empty());
    }

    #[test]
    fn seq_02_newest_open_tool_without_plus() {
        let s = claude(vec![rc(PROMPT, 0.0), rc(PRE, 1.0).tool("A").id("A"), rc(PRE, 2.0).tool("B").id("B"), rc(POST, 3.0).tool("A").id("A")]);
        assert_eq!(s.show(4.0), "▶ B · 2s");
        assert_eq!(row(&s.fold(4.0)).extra_tools, 0);
    }

    #[test]
    fn seq_03_child_receipts_fold_under_the_parent() {
        let mut s = claude(vec![
            rc(PROMPT, 0.0),
            rc(PRE, 1.0).tool("Agent").id("ag"),
            rc(POST, 1.1).tool("Agent").id("ag"),
            rc("SubagentStart", 1.1).agent("x"),
            rc(PRE, 2.0).tool("Read").id("r").agent("x"),
        ]);
        s.kids = vec![kid("x")];
        let rows = s.rows(3.0);
        assert_eq!(rows.len(), 2);
        let show = |r: &LiveRow| {
            let v = render(r, 0);
            format!("{} {}", v.glyph, v.text)
        };
        assert_eq!(show(&rows[0]), "● 1 bg · w");
        assert_eq!(show(&rows[1]), "▶ Read · 1s");
        assert_eq!((rows[1].parent_id.as_deref(), rows[1].session_id.as_str()), (Some("p"), "k-x"));
    }

    #[test]
    fn seq_04_waiting_with_a_stale_ask() {
        let s = claude(vec![
            rc(PROMPT, 0.0),
            rc(PRE, 1.0).tool("Bash").id("b").det("cargo build"),
            rc("PermissionRequest", 1.0).tool("Bash").det("cargo build"),
        ]);
        let f = s.fold(31.0);
        assert_eq!(show(&f), "⚠? permission: Bash cargo build");
        assert_eq!(fmt_age(row(&f).age_ms), "30s");
    }

    #[test]
    fn seq_05_permission_denied_closes_the_call() {
        let s = claude(vec![
            rc(PROMPT, 0.0),
            rc(PRE, 1.0).tool("Bash").id("id"),
            rc("PermissionRequest", 1.0).tool("Bash"),
            rc("PermissionDenied", 3.0).tool("Bash").id("id"),
        ]);
        let f = s.fold(4.0);
        assert_eq!(show(&f), "● w");
        assert!(f.open_tools.is_empty());
    }

    #[test]
    fn seq_06_unlisted_notifications_and_stray_posts_do_nothing() {
        let s = claude(vec![
            rc(PROMPT, 0.0),
            rc(POST, 1.0),
            rc("Notification", 2.0).det("auth_success"),
            rc("Notification", 2.0).det("auth_success"),
        ]);
        assert_eq!(s.show(3.0), "● w");
    }

    #[test]
    fn seq_08_mtime_counts_toward_age() {
        let mut s = claude(vec![rc(PROMPT, 0.0)]);
        s.row.mtime_ms = Some(ms(470.0));
        let f = s.fold(480.0);
        assert_eq!(show(&f), "● w");
        assert_eq!(fmt_age(row(&f).age_ms), "10s");
    }

    #[test]
    fn seq_09_pi_turn() {
        let s = seq(
            "pi",
            vec![rc("agent_start", 0.0), rc("tool_execution_start", 1.0).tool("A").id("A"), rc("tool_execution_end", 2.0).tool("A").id("A"), rc("agent_end", 3.0)],
        );
        let f = s.fold(6.0);
        assert_eq!(show(&f), "○ your turn");
        assert_eq!(row(&f).age_ms, 3000);
    }

    #[test]
    fn seq_10_compaction_clears_on_the_next_event() {
        let mut rs = vec![rc(PROMPT, 0.0), rc("PreCompact", 1.0)];
        let s = claude(rs.clone());
        assert_eq!(s.show(5.0), "⟳ compacting · 4s");
        rs.push(rc("PostCompact", 6.0));
        assert_eq!(claude(rs).show(7.0), "● w");
    }

    #[test]
    fn seq_11_idle_then_end_is_gone() {
        let f = claude(vec![rc("Stop", 0.0), rc("SessionEnd", 1.0)]).fold(2.0);
        assert!(f.row.is_none());
    }

    #[test]
    fn seq_13_session_start_is_your_turn() {
        let f = claude(vec![rc("SessionStart", 0.0).det("startup")]).fold(180.0);
        assert_eq!(show(&f), "○ your turn");
        assert_eq!(fmt_age(row(&f).age_ms), "3m");
        assert!(marks(&f).is_empty());
    }

    #[test]
    fn seq_14_idle_debounce() {
        assert_eq!(claude(vec![rc("Stop", 0.0), rc(PRE, 1.0).tool("A").id("A")]).show(1.5), "▶ A · 0s");
        assert_eq!(claude(vec![rc("Stop", 0.0)]).show(0.5), "● w");
    }

    #[test]
    fn seq_15_failed_session_is_a_mark() {
        let mut s = claude(vec![rc(PROMPT, 0.0)]);
        s.row.failed = true;
        let f = s.fold(1.0);
        assert_eq!(show(&f), "● w");
        assert_eq!(marks(&f), ["✗"]);
        // StopFailure marks it too.
        let f = claude(vec![rc("StopFailure", 0.0)]).fold(5.0);
        assert_eq!((show(&f), marks(&f)), ("○ your turn".into(), vec!["✗"]));
    }

    #[test]
    fn seq_16_null_ids_pair_fifo_in_close_older() {
        let s = claude(vec![rc(PROMPT, 0.0), rc(PRE, 1.0).tool("A"), rc(POST, 2.0).tool("A")]);
        let f = s.fold_in(3.0, false);
        assert_eq!(show(&f), "● w");
        assert!(f.open_tools.is_empty());
    }

    #[test]
    fn seq_17_long_tool_is_hourglass_with_question_mark() {
        let mut s = claude(vec![rc(PROMPT, 0.0), rc(PRE, 1.0).tool("Bash")]);
        s.row.mtime_ms = Some(ms(1.0));
        let f = s.fold(121.0);
        assert_eq!(show(&f), "⏳ Bash · 2m");
        assert_eq!(marks(&f), ["?"]);
    }

    #[test]
    fn seq_18_hook_denied_pre_both_modes() {
        let s = claude(vec![
            rc(PROMPT, 0.0),
            rc(PRE, 1.0).tool("Bash").id("id1"),
            rc(PRE, 2.0).tool("Edit").id("id2"),
            rc(POST, 3.0).tool("Edit").id("id2"),
        ]);
        let f = s.fold_in(4.0, false);
        assert_eq!(show(&f), "● w");
        assert!(f.open_tools.is_empty());
        let f = s.fold_in(4.0, true);
        assert_eq!(show(&f), "▶ Bash · 3s");
        assert_eq!(f.open_tools.len(), 1);
    }

    #[test]
    fn seq_19_two_children_one_stops() {
        let s = claude(vec![rc(PROMPT, 0.0), rc("SubagentStart", 1.0).agent("x"), rc("SubagentStart", 2.0).agent("y"), rc("SubagentStop", 3.0).agent("x")]);
        let f = s.fold(4.0);
        assert_eq!(show(&f), "● 1 bg · w");
        assert_eq!(f.open_children, ["y"]);
    }

    #[test]
    fn seq_21_three_open_tools_plus_two() {
        let s = claude(vec![rc(PROMPT, 0.0), rc(PRE, 1.0).tool("A").id("A"), rc(PRE, 2.0).tool("B").id("B"), rc(PRE, 3.0).tool("C").id("C")]);
        assert_eq!(s.show(4.0), "▶ C · 1s +2");
        assert_eq!(row(&s.fold(4.0)).extra_tools, 2);
    }

    #[test]
    fn seq_22_end_clears_children_and_removes_the_row() {
        let f = claude(vec![rc(PROMPT, 0.0), rc("SubagentStart", 1.0).agent("x"), rc("SessionEnd", 2.0)]).fold(3.0);
        assert!(f.row.is_none() && f.open_children.is_empty());
    }

    #[test]
    fn seq_23_notification_without_a_tool() {
        let s = claude(vec![rc(PROMPT, 0.0), rc("Notification", 1.0).det("permission_prompt")]);
        assert_eq!(s.show(2.0), "⚠ permission_prompt");
    }

    #[test]
    fn seq_26_unknown_pid_stays_and_goes_quiet() {
        let mut s = claude(vec![rc(PROMPT, 0.0)]);
        s.row.harness_pid = None;
        let f = s.fold(3600.0);
        assert_eq!(show(&f), "◌ quiet · w");
        assert_eq!(marks(&f), ["pid?"]);
    }

    #[test]
    fn seq_27_ordering() {
        let at = |id: &str, rs: Vec<LiveReceipt>| {
            let mut s = claude(rs);
            s.row.id = id.into();
            s.rows(100.0)
        };
        let a = at("a1", vec![rc("Stop", 95.0)]);
        let b = at("b", vec![rc(PROMPT, 40.0), rc("PermissionRequest", 50.0).tool("Bash")]);
        let c = at("c", vec![rc(PROMPT, 99.0)]);
        let d = at("a0", vec![rc("Stop", 95.0)]);
        assert_eq!((a[0].state, b[0].state, c[0].state), (LiveState::Prompt, LiveState::Waiting, LiveState::Working));
        let order: Vec<String> = arrange(vec![a, b, c, d]).into_iter().map(|r| r.session_id).collect();
        assert_eq!(order, ["b", "a0", "a1", "c"]);
    }

    #[test]
    fn seq_28_waiting_boundary_at_30s() {
        let s = claude(vec![rc(PROMPT, 0.0), rc("PermissionRequest", 1.0).tool("Bash")]);
        assert!(s.show(31.0).starts_with("⚠? "));
        assert!(s.show(30.999).starts_with("⚠ "));
    }

    #[test]
    fn seq_29_prompt_boundary_at_2s() {
        let s = claude(vec![rc("Stop", 0.0)]);
        assert_eq!(s.show(2.0), "○ your turn");
        assert_eq!(s.show(1.999), "● w");
    }

    #[test]
    fn seq_30_quiet_boundary_at_2min() {
        let s = claude(vec![rc(PROMPT, 0.0)]);
        assert_eq!(s.show(120.0), "◌ quiet · w");
        assert_eq!(s.show(119.999), "● w");
    }

    #[test]
    fn seq_31_hourglass_boundary_at_30s() {
        let s = claude(vec![rc(PROMPT, 0.0), rc(PRE, 1.0).tool("A").id("A")]);
        assert!(s.show(31.0).starts_with("⏳ "));
        assert!(s.show(30.999).starts_with("▶ "));
    }

    #[test]
    fn seq_33_failure_post_closes_the_denied_call() {
        let s = claude(vec![rc(PROMPT, 0.0), rc(PRE, 1.0).tool("Bash").id("id1"), rc("PostToolUseFailure", 2.0).tool("Bash").id("id1")]);
        let f = s.fold(3.0);
        assert_eq!(show(&f), "● w");
        assert!(f.open_tools.is_empty());
    }

    #[test]
    fn seq_34_cursor_double_pre_closed_by_double_post() {
        let rs = vec![
            rc("beforeSubmitPrompt", 0.0),
            rc("beforeShellExecution", 1.0).tool("Shell").id("c1"),
            rc("preToolUse", 1.0).tool("Shell").id("c1"),
            rc("afterShellExecution", 3.0).tool("Shell").id("c1"),
            rc("postToolUse", 3.0).tool("Shell").id("c1"),
        ];
        let s = seq("cursor", rs);
        for strict in [false, true] {
            let f = s.fold_in(4.0, strict);
            assert_eq!(show(&f), "● w", "strict={strict}");
            assert!(f.open_tools.is_empty());
        }
        let mut half = s.rs.clone();
        half.pop();
        let f = fold_session("cursor", &s.row, &half, &[], ms(4.0), false);
        assert_eq!(f.open_tools.len(), 1, "one Post closes one Pre");
    }

    #[test]
    fn seq_35_cursor_ticks_set_the_clock() {
        let f = seq("cursor", vec![rc("beforeSubmitPrompt", 0.0), rc("afterAgentThought", 1.0), rc("afterFileEdit", 2.0)]).fold(3.0);
        assert_eq!(show(&f), "● w");
        assert_eq!(row(&f).age_ms, 1000);
    }

    #[test]
    fn seq_36_cursor_stop_closes_an_orphan_read() {
        let s = seq("cursor", vec![rc("beforeSubmitPrompt", 0.0), rc("beforeReadFile", 1.0).tool("Read"), rc("afterAgentResponse", 2.0), rc("stop", 3.0)]);
        let f = s.fold_in(6.0, false);
        assert_eq!(show(&f), "○ your turn");
        assert!(f.open_tools.is_empty());
    }

    #[test]
    fn seq_39_cursor_session_end_is_gone() {
        assert!(seq("cursor", vec![rc("stop", 0.0), rc("sessionEnd", 1.0)]).fold(2.0).row.is_none());
    }

    #[test]
    fn seq_49_close_older_loses_a_parallel_call_strict_keeps_it() {
        let rs = |pre: &str, post: &str, start: &str| {
            vec![rc(start, 0.0), rc(pre, 1.0).tool("A").id("id1"), rc(pre, 2.0).tool("B").id("id2"), rc(post, 3.0).tool("B").id("id2")]
        };
        let s = claude(rs(PRE, POST, PROMPT));
        assert_eq!(s.show_in(4.0, false), "● w");
        assert_eq!(s.show_in(4.0, true), "▶ A · 3s");
        let pi = seq("pi", rs("tool_execution_start", "tool_execution_end", "agent_start"));
        assert_eq!(pi.show(4.0), "▶ A · 3s", "Pi is strict today");
    }

    #[test]
    fn seq_50_cursor_tick_right_after_stop_is_dropped() {
        let s = seq("cursor", vec![rc("beforeSubmitPrompt", 0.0), rc("stop", 5.0), rc("afterAgentResponse", 5.5)]);
        let f = s.fold(8.0);
        assert_eq!(show(&f), "○ your turn");
        assert_eq!(row(&f).age_ms, 3000, "a dropped Tick never sets ev_at");
        let s = seq("cursor", vec![rc("beforeSubmitPrompt", 0.0), rc("stop", 5.0), rc("afterAgentResponse", 8.0)]);
        assert_eq!(s.show(9.0), "● w");
    }

    #[test]
    fn seq_51_compact_session_start_is_dropped() {
        let s = claude(vec![rc(PROMPT, 0.0), rc(PRE, 1.0).tool("A").id("A"), rc("SessionStart", 2.0).det("compact")]);
        assert_eq!(s.show(3.0), "▶ A · 2s");
    }

    #[test]
    fn seq_54_unquiet_children_boundary_at_10min() {
        let mut s = claude(vec![rc(PROMPT, 0.0), rc("SubagentStart", 1.0).agent("x")]);
        s.row.mtime_ms = Some(ms(601.0 - 1.0));
        let f = s.fold(601.0);
        assert_eq!(show(&f), "● 1 bg? · w");
        assert!(row(&f).bg_quiet);
        s.row.mtime_ms = Some(ms(599.0 - 1.0));
        assert_eq!(s.show(599.0), "● 1 bg · w");
    }

    #[test]
    fn seq_55_render_boundary_through_the_shared_path() {
        let row = LiveRow { state: LiveState::Waiting, since_ms: Some(29_999), text: "permission: Bash".into(), ..Default::default() };
        assert_eq!(render(&row, 0).glyph, "⚠");
        assert_eq!(render(&row, 1).glyph, "⚠?");
    }

    // ---- amendments ----

    #[test]
    fn norm_table() {
        use Ev::*;
        let n = norm;
        assert_eq!(n("claude", "StopFailure", None), Some(Idle { failed: true }));
        assert_eq!(n("claude", "Stop", None), Some(Idle { failed: false }));
        assert_eq!(n("claude", "Notification", Some("idle_prompt")), Some(Idle { failed: false }));
        for t in ["permission_prompt", "elicitation_dialog", "elicitation_url_dialog", "agent_needs_input"] {
            assert_eq!(n("claude", "Notification", Some(t)), Some(Ask), "{t}");
        }
        for t in [Some("auth_success"), Some("elicitation_complete"), None] {
            assert_eq!(n("claude", "Notification", t), None);
        }
        assert_eq!(n("claude", "SessionStart", Some("compact")), None);
        assert_eq!(n("claude", "PermissionDenied", None), Some(Post));
        assert_eq!(n("claude", "SubagentStart", None), Some(ChildStart));
        assert_eq!(n("claude", "PostCompact", None), Some(CompactEnd));
        assert_eq!((n("pi", "tool_execution_start", None), n("pi", "agent_end", None)), (Some(Pre), Some(Idle { failed: false })));
        assert_eq!(n("pi", "PreToolUse", None), None);
        for e in ["preToolUse", "beforeShellExecution", "beforeMCPExecution", "beforeReadFile"] {
            assert_eq!(n("cursor", e, None), Some(Pre), "{e}");
        }
        for e in ["afterFileEdit", "afterAgentThought", "afterAgentResponse"] {
            assert_eq!(n("cursor", e, None), Some(Tick), "{e}");
        }
        assert_eq!((n("cursor", "preCompact", None), n("cursor", "subagentStop", None)), (Some(CompactStart), Some(ChildStop)));
        assert_eq!((n("claude", "unparsed", None), n("codex", "Stop", None)), (None, None));
        assert!(strict_ids("pi") && !strict_ids("claude") && !strict_ids("cursor"));
    }

    #[test]
    fn durations_use_their_own_formatters() {
        assert_eq!((fmt_age(-400), fmt_age(10_000), fmt_age(180_000), fmt_age(7_200_000), fmt_age(172_800_000)), ("0s".into(), "10s".into(), "3m".into(), "2h".into(), "2d".into()));
        assert_eq!((fmt_dur(0), fmt_dur(59_999), fmt_dur(120_000), fmt_dur(3_600_000), fmt_dur(-5)), ("0s".into(), "59s".into(), "2m".into(), "1h".into(), "0s".into()));
    }

    #[test]
    fn ask_run_keeps_the_tool_and_the_first_since() {
        let s = claude(vec![
            rc(PROMPT, 0.0),
            rc("PermissionRequest", 1.0).tool("Bash").det("cargo build"),
            rc("Notification", 2.0).det("permission_prompt"),
        ]);
        let f = s.fold(10.0);
        assert_eq!(show(&f), "⚠ permission: Bash cargo build");
        assert_eq!(row(&f).since_ms, Some(9000));
        // A later non-Ask event ends the run; a new one starts fresh.
        let s = claude(vec![
            rc(PROMPT, 0.0),
            rc("PermissionRequest", 1.0).tool("Bash"),
            rc(PRE, 2.0).tool("Bash").id("b"),
            rc("Notification", 5.0).det("agent_needs_input"),
        ]);
        let f = s.fold(6.0);
        assert_eq!((show(&f), row(&f).since_ms), ("⚠ agent_needs_input".into(), Some(1000)));
    }

    #[test]
    fn equal_ms_ask_sorts_after_pre() {
        // Ask inserted first (lower id) at the same millisecond as its Pre.
        let s = claude(vec![rc(PROMPT, 0.0), rc("PermissionRequest", 1.0).tool("Bash"), rc(PRE, 1.0).tool("Bash").id("b")]);
        assert_eq!(s.show(2.0), "⚠ permission: Bash");
    }

    #[test]
    fn strict_post_edge_cases() {
        let pre = |at, id: Option<&str>, tool: &str| {
            let r = rc(PRE, at).tool(tool);
            if let Some(i) = id { r.id(i) } else { r }
        };
        // A Post whose id matches nothing does nothing, in both modes.
        let s = claude(vec![rc(PROMPT, 0.0), pre(1.0, Some("a"), "A"), rc(POST, 2.0).tool("A").id("zzz")]);
        for strict in [false, true] {
            assert_eq!(s.fold_in(3.0, strict).open_tools.len(), 1, "strict={strict}");
        }
        // A NULL-id Post closes the oldest open Pre of its tool name and nothing older (strict).
        let s = claude(vec![rc(PROMPT, 0.0), pre(1.0, Some("a"), "Edit"), pre(2.0, Some("b"), "Bash"), pre(3.0, Some("c"), "Bash"), rc(POST, 4.0).tool("Bash")]);
        let f = s.fold_in(5.0, true);
        assert_eq!(f.open_tools.iter().filter_map(|t| t.id.as_deref()).collect::<Vec<_>>(), ["a", "c"]);
        // Close-older takes everything older than that Pre too.
        let f = s.fold_in(5.0, false);
        assert_eq!(f.open_tools.iter().filter_map(|t| t.id.as_deref()).collect::<Vec<_>>(), ["c"]);
        // A NULL-id Pre of an open NULL-id name replaces the older one (close-older only).
        let s = claude(vec![rc(PROMPT, 0.0), pre(1.0, None, "A"), pre(2.0, None, "A")]);
        assert_eq!(s.fold_in(3.0, false).open_tools.len(), 1);
        assert_eq!(s.fold_in(3.0, true).open_tools.len(), 2);
    }

    #[test]
    fn text_has_no_double_space_and_marks_are_ordered() {
        let s = claude(vec![rc(PROMPT, 0.0), rc(PRE, 1.0).tool("Bash")]);
        assert_eq!(row(&s.fold(3.0)).text, "Bash");
        let mut s = claude(vec![rc(PROMPT, 0.0), rc(PRE, 1.0).tool("Bash").det("ls")]);
        s.row.failed = true;
        s.row.harness_pid = None;
        let f = s.fold(300.0);
        assert_eq!(show(&f), "⏳ Bash ls · 4m");
        assert_eq!(marks(&f), ["✗", "?", "pid?"]);
    }

    #[test]
    fn render_adds_extra_and_clamps() {
        let row = LiveRow { state: LiveState::Tool, text: "A".into(), age_ms: -400, since_ms: Some(25_000), quiet_ms: 1000, ..Default::default() };
        assert_eq!((render(&row, 0).glyph, render(&row, 6000).glyph), ("▶", "⏳"));
        assert_eq!(render(&row, 6000).text, "A · 31s");
        let none = LiveRow { state: LiveState::Compacting, ..Default::default() };
        assert_eq!(render(&none, 0).text, "compacting · 0s", "since None is 0");
        let prompt = LiveRow { state: LiveState::Prompt, bg: 2, bg_quiet: true, ..Default::default() };
        assert_eq!(render(&prompt, 0).text, "your turn · 2 bg?");
        let quiet = LiveRow { state: LiveState::Working, bg: 1, quiet_ms: 130_000, text: "w".into(), ..Default::default() };
        assert_eq!((render(&quiet, 0).glyph, render(&quiet, 0).text), ("◌", "1 bg · quiet · w".into()));
        let quiet_tool = LiveRow { state: LiveState::Prompt, quiet_ms: 130_000, ..Default::default() };
        assert_eq!((render(&quiet_tool, 0).glyph, render(&quiet_tool, 0).marks), ("○", vec![]), "quiet applies to Working only");
    }

    #[test]
    fn rows_round_trip_and_default() {
        let row = LiveRow { session_id: "s".into(), state: LiveState::Tool, text: "A".into(), since_ms: Some(5), extra_tools: 2, bg: 1, ..Default::default() };
        let json = serde_json::to_string(&row).unwrap();
        assert_eq!(serde_json::from_str::<LiveRow>(&json).unwrap(), row);
        assert_eq!(serde_json::from_str::<LiveRow>("{\"session_id\":\"s\"}").unwrap().state, LiveState::Working);
        assert_eq!(serde_json::from_str::<LiveRow>("{}").unwrap(), LiveRow::default());
        assert_eq!(LIVE_SCHEMA, 1);
    }

    // ---- children ----

    fn with_kids(mut s: Seq, agents: &[&str]) -> Seq {
        s.kids = agents.iter().map(|a| kid(a)).collect();
        s
    }

    #[test]
    fn child_without_receipts_shows_the_transcript_while_open() {
        // Open (started, not stopped): shown with the transcript's `now`.
        let s = with_kids(claude(vec![rc(PROMPT, 0.0), rc("SubagentStart", 1.0).agent("x")]), &["x"]);
        let rows = s.rows(3.0);
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[1].state, rows[1].text.as_str()), (LiveState::Working, "kidnow"));
        // Stopped: hidden.
        let s = with_kids(claude(vec![rc(PROMPT, 0.0), rc("SubagentStart", 1.0).agent("x"), rc("SubagentStop", 2.0).agent("x")]), &["x"]);
        assert_eq!(s.rows(3.0).len(), 1);
        // Never seen in a start/stop and not completed: shown. Completed: hidden.
        let mut s = with_kids(claude(vec![rc(PROMPT, 0.0)]), &["y"]);
        assert_eq!(s.rows(3.0).len(), 2);
        s.kids[0].status = Status::Completed;
        assert_eq!(s.rows(3.0).len(), 1);
        // A child of another parent, or one matched only by suffix, is not attached.
        let mut s = with_kids(claude(vec![rc(PROMPT, 0.0), rc("SubagentStart", 1.0).agent("x")]), &["x"]);
        s.kids[0].parent_id = Some("other".into());
        assert_eq!(s.rows(3.0).len(), 1);
        let mut s = with_kids(claude(vec![rc(PROMPT, 0.0), rc(PRE, 2.0).tool("Read").id("r").agent("x")]), &["x"]);
        s.kids[0].native_id = Some("proj/sess/subagents/agent-yx".into());
        let rows = s.rows(3.0);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].text, "kidnow", "agent-yx is not agent-x, so x's receipts are not its own");
    }

    #[test]
    fn child_fold_ignores_parent_boundaries_and_parent_tools() {
        let s = with_kids(
            claude(vec![
                rc(PROMPT, 0.0),
                rc("SubagentStart", 1.0).agent("x"),
                rc(PRE, 2.0).tool("Read").id("r").agent("x"),
                rc(PRE, 2.5).tool("Edit").id("e"),
                rc("Stop", 3.0),
            ]),
            &["x"],
        );
        let rows = s.rows(4.0);
        let v = render(&rows[1], 0);
        assert_eq!((v.glyph, v.text.as_str()), ("▶", "Read · 2s"));
        assert_eq!(rows[0].state, LiveState::Working, "Stop@3 is inside the 2 s debounce at now=4");
    }

    #[test]
    fn child_ordering_among_siblings() {
        let mut s = with_kids(
            claude(vec![
                rc(PROMPT, 0.0),
                rc("SubagentStart", 1.0).agent("x"),
                rc("SubagentStart", 1.0).agent("y"),
                rc("PermissionRequest", 5.0).tool("Bash").agent("y"),
                rc(PRE, 2.0).tool("Read").id("r").agent("x"),
            ]),
            &["x", "y"],
        );
        s.row.id = "p".into();
        let rows = arrange(vec![s.rows(6.0)]);
        // The parent stays first; its Waiting child sorts before its Working sibling but never bubbles the parent.
        assert_eq!(rows[0].session_id, "p");
        assert_eq!(rows[1].state, LiveState::Waiting);
        assert_eq!(rows[2].state, LiveState::Tool);
    }
}
