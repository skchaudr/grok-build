//! `grok-traces follow`: a `tail -f` for one live session (and the subagents it dispatches).
//!
//! The formatters are pure; [`Follower::tick`] indexes just the followed files (the same single-file path
//! hooks use), reads the new rows and returns the blocks to print.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::SystemTime;

use serde_json::Value;

use crate::normalize::{clean_text, normalize};
use crate::roots::resolve;
use crate::{ACTIVE_WINDOW_MS, EventFilter, EventRow, MAX_EVENTS_PER_FILE, Result, Root, SessionRow, Status, TraceIndex};

const WRAP: usize = 100;
const CLIP: usize = 120;
/// Rows shown for a session that is already running when followed (`tail`'s default).
const BACKLOG: usize = 10;
const IDLE_FIRST_S: u64 = 30;
const IDLE_EVERY_S: u64 = 60;

fn indent(depth: usize) -> String {
    "  ".repeat(depth)
}

fn clip(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        None => s.to_string(),
        Some((i, _)) => format!("{}…", &s[..i]),
    }
}

fn first_line(s: &str) -> &str {
    s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("")
}

/// `claude-sonnet-5-5` -> `sonnet-5-5`; a trailing `-YYYYMMDD` is dropped.
pub fn short_model(m: &str) -> String {
    let m = m.strip_prefix("claude-").unwrap_or(m);
    match m.rsplit_once('-') {
        Some((head, tail)) if tail.len() == 8 && tail.bytes().all(|b| b.is_ascii_digit()) => head.to_string(),
        _ => m.to_string(),
    }
}

fn secs(s: u64) -> String {
    if s < 60 { format!("{s}s") } else { format!("{}m{:02}s", s / 60, s % 60) }
}

/// `0.4s`, `12s`, `2m05s`.
fn took(ms: i64) -> String {
    if ms < 10_000 { format!("{:.1}s", ms as f64 / 1000.0) } else { secs((ms as f64 / 1000.0).round() as u64) }
}

/// `<source> · <model> · <title> · <cwd>`, `home` shown as `~`.
pub fn header(s: &SessionRow, home: Option<&str>) -> String {
    let cwd = s.cwd.as_deref().map_or_else(
        || "-".to_string(),
        |c| match home.filter(|h| !h.is_empty()).and_then(|h| c.strip_prefix(h)) {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("~{rest}"),
            _ => c.to_string(),
        },
    );
    let model = s.model.as_deref().map_or_else(|| "-".to_string(), short_model);
    format!("{} · {model} · {} · {cwd}", s.source, first_line(&s.title))
}

pub fn idle_line(s: u64) -> String {
    format!("… idle {}", secs(s))
}

pub fn ended_line(depth: usize) -> String {
    if depth == 0 { "■ ended".into() } else { format!("{}↳ ended", indent(depth)) }
}

pub fn started_line(model: Option<&str>, depth: usize) -> String {
    let model = model.map(|m| format!("{} ", short_model(m))).unwrap_or_default();
    format!("{}↳ {model}started", indent(depth))
}

/// Human text of one raw line: every `text`/`thinking` string and string `content`, tool inputs skipped.
fn raw_text(raw: &str) -> String {
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Array(xs) => xs.iter().for_each(|x| walk(x, out)),
            Value::Object(m) => {
                for (k, x) in m {
                    match (k.as_str(), x) {
                        ("input" | "arguments" | "usage", _) => {}
                        ("text" | "thinking" | "content", Value::String(s)) => {
                            let t = s.trim_end();
                            if !t.trim().is_empty() && out.last().is_none_or(|l| l != t) {
                                out.push(t.to_string());
                            }
                        }
                        _ => walk(x, out),
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    if let Ok(v) = serde_json::from_str::<Value>(raw) {
        walk(&v, &mut out);
    }
    clean_text(&out.join("\n\n"))
}

/// The row's full text from its raw line, else its (clipped, flattened) preview.
fn full_text(ev: &EventRow, raw: Option<&str>) -> String {
    let t = raw.map(raw_text).unwrap_or_default();
    if !t.is_empty() {
        return t;
    }
    if ev.preview == ev.title { String::new() } else { ev.preview.trim().to_string() }
}

/// Word-wraps `text` to `width` chars per line; blank lines dropped, a word longer than `width` stays whole.
/// shortcut: width is counted in chars, not terminal cells; wide glyphs overshoot.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()) {
        let mut cur = String::new();
        for word in line.split(' ') {
            if !cur.trim().is_empty() && cur.chars().count() + 1 + word.chars().count() > width {
                out.push(std::mem::take(&mut cur));
                cur = word.to_string();
            } else if cur.is_empty() {
                cur = word.to_string();
            } else {
                cur.push(' ');
                cur.push_str(word);
            }
        }
        out.push(cur);
    }
    out
}

fn body(pfx: &str, mark: &str, text: &str) -> Option<String> {
    let lines = wrap(text, WRAP.saturating_sub(pfx.len() + 3).max(20));
    let (first, rest) = lines.split_first()?;
    let mut s = format!("{pfx}{mark}  {first}");
    for l in rest {
        s.push_str(&format!("\n{pfx}   {l}"));
    }
    Some(s)
}

/// One printed block for a row, or `None` when the row is not shown. `raw` is the row's source line.
pub fn format_event(ev: &EventRow, raw: Option<&str>, depth: usize) -> Option<String> {
    if ev.noise {
        return None;
    }
    let pfx = indent(depth);
    let role = ev.role.as_deref();
    match (ev.kind.as_str(), role) {
        ("tool", Some("tool")) => {
            let what = [ev.intent.as_deref(), ev.detail.as_deref(), Some(ev.preview.as_str())]
                .into_iter()
                .flatten()
                .map(first_line)
                .find(|s| !s.is_empty() && *s != ev.title)
                .unwrap_or("");
            Some(format!("{pfx}→ {}  {}", ev.title, clip(what, CLIP)).trim_end().to_string())
        }
        ("tool", Some("tool_result")) if ev.is_error => {
            let detail = ev.detail.as_deref().map(first_line).unwrap_or("");
            let (exit, msg) = if detail.starts_with("exit ") { (detail, first_line(&ev.preview)) } else { ("", if detail.is_empty() { first_line(&ev.preview) } else { detail }) };
            let parts: Vec<String> = [exit, msg].into_iter().filter(|s| !s.is_empty()).map(|s| clip(s, CLIP)).collect();
            Some(format!("{pfx}   ✗ {}", parts.join("  ")).trim_end().to_string())
        }
        ("tool", Some("tool_result")) => Some(match ev.duration_ms {
            Some(ms) => format!("{pfx}   ✓ {}", took(ms)),
            None => format!("{pfx}   ✓"),
        }),
        ("reasoning", _) | (_, Some("reasoning")) => body(&pfx, "~", &full_text(ev, raw)),
        ("message", Some("assistant")) => body(&pfx, "·", &full_text(ev, raw)),
        ("message", Some("user")) => {
            let text = full_text(ev, raw);
            let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
            let head = clip(lines.next()?, CLIP);
            let more = lines.count();
            let more = if more > 0 { format!(" (+{more} lines)") } else { String::new() };
            Some(format!("\n{pfx}▶ you  {head}{more}"))
        }
        _ => None,
    }
}

struct Followed {
    id: String,
    depth: usize,
    last_line: i64,
    ended: bool,
    /// Printed before this session's first row, so a subagent's start follows its parent's rows.
    announce: Option<String>,
    /// Tool call id -> line of calls with no result yet; the index puts a call's duration on the call row.
    calls: HashMap<String, i64>,
}

pub struct Follower {
    idx: TraceIndex,
    roots: Vec<Root>,
    children: bool,
    home: Option<String>,
    sessions: Vec<Followed>,
    root: SessionRow,
    started: bool,
    last_activity_ms: i64,
    next_idle_s: u64,
}

impl Follower {
    pub fn new(idx: TraceIndex, roots: Vec<Root>, root: SessionRow, children: bool, home: Option<String>) -> Self {
        Self { idx, roots, children, home, sessions: Vec::new(), root, started: false, last_activity_ms: 0, next_idle_s: IDLE_FIRST_S }
    }

    /// Indexes the session's file alone (as a hook would); sources with no file (bb) are left to their hooks.
    fn index(&mut self, path: &Path) -> Result<()> {
        if let Some((root, spelled)) = resolve(path, &self.roots) {
            self.idx.index_file(root, &spelled)?;
        }
        Ok(())
    }

    /// Line to resume after so that `tail` rows are still to print; -1 (lines count from 0) prints everything.
    fn resume_after(&self, id: &str, tail: Option<usize>) -> Result<i64> {
        let Some(n) = tail else { return Ok(-1) };
        let rows = self.idx.list_events(id, &EventFilter { limit: MAX_EVENTS_PER_FILE, ..Default::default() })?;
        // Never start on a result whose call is out of view.
        let mut from = rows.len().saturating_sub(n);
        while from > 0 && rows[from].role.as_deref() == Some("tool_result") {
            from -= 1;
        }
        Ok(from.checked_sub(1).map_or(-1, |i| rows[i].line))
    }

    fn adopt(&mut self, s: &SessionRow, depth: usize, tail: Option<usize>, announce: Option<String>) -> Result<()> {
        let last_line = self.resume_after(&s.id, tail)?;
        self.sessions.push(Followed { id: s.id.clone(), depth, last_line, ended: false, announce, calls: HashMap::new() });
        Ok(())
    }

    /// Subagent transcripts are indexed by scans only; index the live ones beside this session's file so
    /// they show up (Claude: `<stem>/subagents/`, Cursor: `subagents/` next to the transcript).
    fn probe_subagents(&mut self, path: &Path, now_ms: i64) -> Result<()> {
        let dirs = [Some(path.with_extension("").join("subagents")), path.parent().map(|p| p.join("subagents"))];
        for dir in dirs.into_iter().flatten() {
            let Ok(entries) = fs::read_dir(&dir) else { continue };
            for p in entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "jsonl")) {
                let fresh = fs::metadata(&p)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .is_some_and(|t| now_ms - (t.as_millis() as i64) < ACTIVE_WINDOW_MS);
                if fresh {
                    self.index(&p)?;
                }
            }
        }
        Ok(())
    }

    /// Duration of the call that `result_id` answers, from the call's row.
    fn call_duration(&mut self, at: usize, result_id: Option<&str>) -> Result<Option<i64>> {
        let Some(line) = result_id.and_then(|id| self.sessions[at].calls.remove(id)) else { return Ok(None) };
        let filter = EventFilter { since_line: Some(line - 1), limit: 1, include_noise: true, ..Default::default() };
        Ok(self.idx.list_events(&self.sessions[at].id, &filter)?.into_iter().find(|e| e.line == line).and_then(|e| e.duration_ms))
    }

    /// One poll: index the followed files, find new subagents, return everything newly printable.
    pub fn tick(&mut self, now_ms: i64) -> Result<Vec<String>> {
        let mut out = Vec::new();
        let first = !self.started;
        if first {
            self.started = true;
            self.last_activity_ms = now_ms;
            out.push(header(&self.root, self.home.as_deref()));
            let root = self.root.clone();
            self.index(&root.file_path)?;
            self.adopt(&root, 0, Some(BACKLOG), None)?;
        }
        let mut printed = false;
        let mut at = 0;
        while at < self.sessions.len() {
            let (id, depth) = (self.sessions[at].id.clone(), self.sessions[at].depth);
            let Some(row) = self.idx.session(&id)? else { at += 1; continue };
            out.extend(self.sessions[at].announce.take());
            self.index(&row.file_path)?;
            if self.children {
                self.probe_subagents(&row.file_path, now_ms)?;
                for kid in self.idx.children(&id)? {
                    if kid.status == Status::Active && !self.sessions.iter().any(|f| f.id == kid.id) {
                        self.adopt(&kid, depth + 1, first.then_some(BACKLOG), Some(started_line(kid.model.as_deref(), depth + 1)))?;
                    }
                }
            }
            let filter = EventFilter { since_line: Some(self.sessions[at].last_line), limit: MAX_EVENTS_PER_FILE, ..Default::default() };
            for mut ev in self.idx.list_events(&id, &filter)? {
                self.sessions[at].last_line = ev.line;
                let raw = self.idx.raw_line(&id, ev.line).ok().flatten();
                let tool_ids = || raw.as_deref().and_then(|r| serde_json::from_str::<Value>(r).ok()).map(|v| normalize(&v));
                match ev.role.as_deref() {
                    Some("tool") => {
                        if let Some(call_id) = tool_ids().and_then(|n| n.tool_call_id) {
                            self.sessions[at].calls.insert(call_id, ev.line);
                        }
                    }
                    Some("tool_result") if ev.duration_ms.is_none() => {
                        let result_id = tool_ids().and_then(|n| n.tool_result_id);
                        ev.duration_ms = self.call_duration(at, result_id.as_deref())?;
                    }
                    _ => {}
                }
                if let Some(block) = format_event(&ev, raw.as_deref(), depth) {
                    out.push(block);
                    printed = true;
                }
            }
            let live = row.status == Status::Active;
            let f = &mut self.sessions[at];
            if live {
                f.ended = false;
            } else if !f.ended {
                f.ended = true;
                out.push(ended_line(depth));
            }
            at += 1;
        }
        if printed {
            self.last_activity_ms = now_ms;
            self.next_idle_s = IDLE_FIRST_S;
        } else if (now_ms - self.last_activity_ms) / 1000 >= self.next_idle_s as i64 {
            out.push(idle_line(self.next_idle_s));
            self.next_idle_s += IDLE_EVERY_S;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: &str, role: Option<&str>, title: &str, preview: &str) -> EventRow {
        EventRow {
            line: 1,
            event_type: String::new(),
            kind: kind.into(),
            role: role.map(Into::into),
            title: title.into(),
            preview: preview.into(),
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

    fn user(text: &str) -> EventRow {
        ev("message", Some("user"), "User", text)
    }

    #[test]
    fn header_line() {
        let s = SessionRow {
            source: "claude".into(),
            model: Some("claude-sonnet-5-5".into()),
            title: "Fix the build\nsecond line".into(),
            cwd: Some("/Users/me/repos/x".into()),
            ..Default::default()
        };
        assert_eq!(header(&s, Some("/Users/me")), "claude · sonnet-5-5 · Fix the build · ~/repos/x");
        assert_eq!(header(&s, Some("/Users/m")), "claude · sonnet-5-5 · Fix the build · /Users/me/repos/x");
        let s = SessionRow { source: "pi".into(), title: "t".into(), ..Default::default() };
        assert_eq!(header(&s, None), "pi · - · t · -");
        assert_eq!(short_model("claude-haiku-4-5-20251001"), "haiku-4-5");
        assert_eq!(short_model("gpt-5"), "gpt-5");
    }

    #[test]
    fn user_message() {
        assert_eq!(format_event(&user("fix it"), None, 0).as_deref(), Some("\n▶ you  fix it"));
        let raw = r#"{"type":"user","message":{"role":"user","content":"first\nsecond\n\nthird"}}"#;
        assert_eq!(format_event(&user("first second third"), Some(raw), 1).as_deref(), Some("\n  ▶ you  first (+2 lines)"));
        let long = "x".repeat(150);
        let want = format!("\n▶ you  {}…", "x".repeat(120));
        assert_eq!(format_event(&user(&long), None, 0), Some(want));
    }

    #[test]
    fn assistant_text_wraps_and_is_never_clipped() {
        let text = (1..=60).map(|i| format!("w{i:02}")).collect::<Vec<_>>().join(" ");
        let raw = serde_json::json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":text}]}}).to_string();
        let out = format_event(&ev("message", Some("assistant"), "Assistant", "w01 w02…"), Some(&raw), 0).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].starts_with("·  w01 w02 w03 "));
        assert!(lines[1..].iter().all(|l| l.starts_with("   w")));
        assert!(lines.iter().all(|l| l.chars().count() <= 100));
        let words: Vec<&str> = out.split_whitespace().skip(1).collect();
        assert_eq!(words.len(), 60, "every word kept");
        assert_eq!(words[59], "w60");
        let nested = format_event(&ev("message", Some("assistant"), "Assistant", "hi"), Some(r#"{"message":{"content":"a\n\nb"}}"#), 2).unwrap();
        assert_eq!(nested, "    ·  a\n       b");
    }

    #[test]
    fn reasoning_uses_tilde() {
        let raw = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"consider the cache"}]}}"#;
        let r = ev("reasoning", Some("reasoning"), "Reasoning", "consider the…");
        assert_eq!(format_event(&r, Some(raw), 0).as_deref(), Some("~  consider the cache"));
        assert_eq!(format_event(&r, None, 1).as_deref(), Some("  ~  consider the…"));
    }

    #[test]
    fn tool_call_prefers_intent_then_detail_then_preview() {
        let mut c = ev("tool", Some("tool"), "Bash", "cargo test");
        c.detail = Some("cargo test -p x".into());
        c.intent = Some("run the crate tests".into());
        assert_eq!(format_event(&c, None, 0).as_deref(), Some("→ Bash  run the crate tests"));
        c.intent = None;
        assert_eq!(format_event(&c, None, 1).as_deref(), Some("  → Bash  cargo test -p x"));
        c.detail = None;
        assert_eq!(format_event(&c, None, 0).as_deref(), Some("→ Bash  cargo test"));
        c.preview = "Bash".into();
        assert_eq!(format_event(&c, None, 0).as_deref(), Some("→ Bash"));
    }

    #[test]
    fn tool_result_ok_durations() {
        let mut r = ev("tool", Some("tool_result"), "Tool Result", "ok");
        assert_eq!(format_event(&r, None, 0).as_deref(), Some("   ✓"));
        for (ms, want) in [(400, "   ✓ 0.4s"), (12_000, "   ✓ 12s"), (125_000, "   ✓ 2m05s")] {
            r.duration_ms = Some(ms);
            assert_eq!(format_event(&r, None, 0).as_deref(), Some(want));
        }
        assert_eq!(format_event(&r, None, 1).as_deref(), Some("     ✓ 2m05s"));
    }

    #[test]
    fn tool_result_error() {
        let mut r = ev("tool", Some("tool_result"), "Tool Result", "error: boom\nmore");
        r.is_error = true;
        r.detail = Some("error: boom".into());
        assert_eq!(format_event(&r, None, 0).as_deref(), Some("   ✗ error: boom"));
        r.detail = Some("exit 2".into());
        r.preview = "error: no such file".into();
        assert_eq!(format_event(&r, None, 0).as_deref(), Some("   ✗ exit 2  error: no such file"));
        r.detail = None;
        r.preview = "y".repeat(130);
        assert_eq!(format_event(&r, None, 1).as_deref(), Some(format!("     ✗ {}…", "y".repeat(120)).as_str()));
    }

    #[test]
    fn hidden_rows() {
        let mut n = user("x");
        n.noise = true;
        assert_eq!(format_event(&n, None, 0), None);
        for (k, r) in [("system", None), ("turn", None), ("telemetry", None), ("message", Some("system"))] {
            assert_eq!(format_event(&ev(k, r, "T", "p"), None, 0), None, "{k}");
        }
        assert_eq!(format_event(&ev("message", Some("user"), "User", "User"), None, 0), None, "empty prompt");
    }

    #[test]
    fn state_lines() {
        assert_eq!(ended_line(0), "■ ended");
        assert_eq!(ended_line(2), "    ↳ ended");
        assert_eq!(started_line(Some("claude-sonnet-5-5"), 1), "  ↳ sonnet-5-5 started");
        assert_eq!(started_line(None, 1), "  ↳ started");
        assert_eq!(idle_line(30), "… idle 30s");
        assert_eq!(idle_line(90), "… idle 1m30s");
        assert_eq!(idle_line(125), "… idle 2m05s");
    }
}
