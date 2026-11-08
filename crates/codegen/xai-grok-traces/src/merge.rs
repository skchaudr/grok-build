//! One stream per session: spans and receipts are the structure, the transcript is the content.
//!
//! A call opens when a receipt or a span says it started. Text already in the transcript and ordered
//! before that call is emitted first. The row stays running until the result text is in the transcript
//! and an end (span or receipt) has arrived; only then does it complete. A later fill never follows a
//! completion, so the tracker keeps a single row.

use std::collections::{HashMap, HashSet};

use crate::live::{Ev, LiveReceipt, norm};
use crate::phoenix::{PhoenixSpan, SpanStatus};

/// One piece of a harness transcript, in file order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub ord: u64,
    /// The line's `timestamp`, 0 when it has none.
    pub at_ms: i64,
    pub kind: ItemKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemKind {
    User(String),
    Text(String),
    /// Text and the line's `thinkingDurationMs`.
    Thinking(String, Option<i64>),
    Call { id: String, tool: String, input: String },
    Result { id: String, output: String, is_error: bool },
}

/// What the view should append. `Complete { fresh: true }` is the only event for that call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Piece {
    User(String),
    Text(String),
    /// A finished thought: `at_ms` is when it was written, `dur_ms` how long it took.
    Thinking { text: String, at_ms: i64, dur_ms: Option<i64> },
    /// `label` is the receipt's one-line detail, for while the transcript has no input for the call.
    Open { id: String, tool: String, start_ms: i64, input: Option<String>, label: Option<String>, parent: Option<String> },
    Fill { id: String, tool: String, input: Option<String>, output: Option<String> },
    Complete {
        id: String,
        tool: String,
        start_ms: i64,
        end_ms: i64,
        error: bool,
        input: Option<String>,
        label: Option<String>,
        output: String,
        fresh: bool,
        parent: Option<String>,
    },
}

#[derive(Clone, Debug)]
struct Signal {
    call_id: String,
    tool: Option<String>,
    start_ms: Option<i64>,
    end_ms: Option<i64>,
    error: Option<bool>,
    from_span: bool,
    parent: Option<String>,
    detail: Option<String>,
    /// A subagent's call (its receipt has an agent id): its content is in the subagent's own transcript.
    sub: bool,
}

#[derive(Clone, Debug)]
struct Call {
    tool: String,
    start_ms: i64,
    start_span: bool,
    end_ms: Option<i64>,
    end_span: bool,
    error: bool,
    error_set: bool,
    input: Option<String>,
    output: Option<String>,
    out_err: bool,
    input_dirty: bool,
    output_dirty: bool,
    opened: bool,
    done: bool,
    parent: Option<String>,
    /// No receipt or span has named it: the transcript alone opens it, and its result ends it.
    transcript_only: bool,
    detail: Option<String>,
    sub: bool,
}

impl Call {
    fn new(start_ms: i64, transcript_only: bool) -> Self {
        Call {
            tool: String::new(),
            start_ms,
            start_span: false,
            end_ms: None,
            end_span: false,
            error: false,
            error_set: false,
            input: None,
            output: None,
            out_err: false,
            input_dirty: false,
            output_dirty: false,
            opened: false,
            done: false,
            parent: None,
            transcript_only,
            detail: None,
            sub: false,
        }
    }
}

/// Running merge for one session. Feed it signals, then the transcript tail each time.
#[derive(Default)]
pub struct Merger {
    calls: HashMap<String, Call>,
    seen: HashSet<u64>,
}

impl Merger {
    /// A call is starting or ending. `items` is the transcript as it reads right now.
    pub fn on_signal(&mut self, call_id: String, tool: Option<String>, start_ms: Option<i64>, end_ms: Option<i64>, error: Option<bool>, from_span: bool, parent: Option<String>, items: &[Item]) -> Vec<Piece> {
        self.signal(Signal { call_id, tool, start_ms, end_ms, error, from_span, parent, detail: None, sub: false }, items)
    }

    fn signal(&mut self, sig: Signal, items: &[Item]) -> Vec<Piece> {
        let id = self.upsert(sig);
        let before = call_ord(items, &id).unwrap_or(u64::MAX);
        let mut out = self.flow(items, before);
        self.absorb(items, Some(&id));
        out.extend(self.emit(&id));
        out
    }

    /// Transcript grew, or the tail should be flushed after a batch of signals.
    pub fn on_tail(&mut self, items: &[Item]) -> Vec<Piece> {
        let mut out = self.flow(items, u64::MAX);
        let ids: Vec<String> = self.calls.keys().cloned().collect();
        for id in ids {
            out.extend(self.emit(&id));
        }
        out
    }

    /// Receipts and spans the view has now. Calls and prose already emitted stay put.
    pub fn ingest(&mut self, source: &str, transcript: &str, receipts: &[LiveReceipt], spans: Result<&[PhoenixSpan], ()>) -> (Vec<Piece>, bool) {
        let items = parse(source, transcript);
        let mut pieces = Vec::new();
        for sig in receipt_signals(source, receipts) {
            pieces.extend(self.signal(sig, &items));
        }
        let offline = spans.is_err();
        if let Ok(spans) = spans {
            for sig in span_signals(spans) {
                pieces.extend(self.signal(sig, &items));
            }
        }
        if !crate::live::strict_ids(source) {
            self.close_denied();
        }
        pieces.extend(self.on_tail(&items));
        (pieces, offline)
    }

    /// A hook-denied call gets a `Pre` and an error result but never an end. Once a later call has ended, end it at its start.
    fn close_denied(&mut self) {
        let ended: Vec<i64> = self.calls.values().filter(|c| c.end_ms.is_some()).map(|c| c.start_ms).collect();
        for c in self.calls.values_mut() {
            if c.end_ms.is_none() && c.out_err && c.output.is_some() && ended.iter().any(|&s| s > c.start_ms) {
                c.end_ms = Some(c.start_ms);
            }
        }
    }

    /// Call id and tool name of the newest call that is on screen and not finished.
    pub fn running(&self) -> Option<(String, String)> {
        self.calls
            .iter()
            .filter(|(_, c)| c.opened && !c.done)
            .max_by_key(|(id, c)| (c.start_ms, (*id).clone()))
            .map(|(id, c)| (id.clone(), if c.tool.is_empty() { "tool".to_string() } else { c.tool.clone() }))
    }

    fn upsert(&mut self, sig: Signal) -> String {
        let id = sig.call_id.clone();
        let c = self.calls.entry(id.clone()).or_insert_with(|| Call::new(sig.start_ms.unwrap_or(0), false));
        if c.transcript_only {
            c.transcript_only = false;
            // The transcript's guess gives way to the receipt or span.
            if c.end_ms.is_some() && !c.done {
                c.end_ms = None;
            }
        }
        if c.tool.is_empty() {
            if let Some(t) = sig.tool.clone() {
                c.tool = t;
            }
        }
        if let Some(ms) = sig.start_ms {
            if sig.from_span || !c.start_span {
                c.start_ms = ms;
                c.start_span = sig.from_span;
            }
        }
        if let Some(ms) = sig.end_ms {
            if sig.from_span || !c.end_span {
                c.end_ms = Some(ms);
                c.end_span = sig.from_span;
            }
        }
        if let Some(e) = sig.error {
            if sig.from_span || !c.end_span {
                c.error = e;
                c.error_set = true;
            }
        }
        if c.parent.is_none() {
            c.parent = sig.parent;
        }
        if c.detail.is_none() {
            c.detail = sig.detail;
        }
        c.sub |= sig.sub;
        id
    }

    /// Everything ordered before `before`, in transcript order. A call no receipt or span has named still opens
    /// from its transcript line, so it never holds back what follows it.
    fn flow(&mut self, items: &[Item], before: u64) -> Vec<Piece> {
        let items = &items[..items.partition_point(|it| it.ord < before)];
        for it in items {
            if let ItemKind::Call { id, .. } = &it.kind {
                self.calls.entry(id.clone()).or_insert_with(|| Call::new(it.at_ms, true));
            }
        }
        self.absorb(items, None);
        let mut out = Vec::new();
        for it in items {
            if let ItemKind::Call { id, .. } = &it.kind {
                out.extend(self.emit(id));
                continue;
            }
            if matches!(it.kind, ItemKind::Result { .. }) || !self.seen.insert(it.ord) {
                continue;
            }
            match &it.kind {
                ItemKind::User(t) => out.push(Piece::User(t.clone())),
                ItemKind::Text(t) => out.push(Piece::Text(t.clone())),
                ItemKind::Thinking(t, dur) => out.push(Piece::Thinking { text: t.clone(), at_ms: it.at_ms, dur_ms: *dur }),
                ItemKind::Call { .. } | ItemKind::Result { .. } => {}
            }
        }
        out
    }

    fn absorb(&mut self, items: &[Item], only: Option<&str>) {
        for it in items {
            match &it.kind {
                ItemKind::Call { id, tool, input } if only.is_none_or(|o| o == id) => {
                    let Some(c) = self.calls.get_mut(id) else { continue };
                    if c.tool.is_empty() {
                        c.tool = tool.clone();
                    }
                    if c.input.is_none() && !input.is_empty() {
                        c.input = Some(input.clone());
                        c.input_dirty = true;
                    }
                }
                ItemKind::Result { id, output, is_error } if only.is_none_or(|o| o == id) => {
                    let Some(c) = self.calls.get_mut(id) else { continue };
                    if c.output.is_none() {
                        c.output = Some(output.clone());
                        c.out_err = *is_error;
                        c.output_dirty = true;
                        if c.transcript_only {
                            c.end_ms = Some(it.at_ms.max(c.start_ms));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn emit(&mut self, id: &str) -> Vec<Piece> {
        let Some(c) = self.calls.get_mut(id) else { return Vec::new() };
        if c.done {
            return Vec::new();
        }
        // A subagent's result is in its own transcript, never this one: its end is enough.
        let ready = (c.output.is_some() || c.sub) && c.end_ms.is_some();
        if !c.opened && ready {
            c.opened = true;
            c.done = true;
            c.input_dirty = false;
            c.output_dirty = false;
            return vec![complete_piece(id, c, true)];
        }
        let mut out = Vec::new();
        if !c.opened {
            c.opened = true;
            out.push(Piece::Open {
                id: id.to_string(),
                tool: c.tool.clone(),
                start_ms: c.start_ms,
                input: c.input.clone(),
                label: c.detail.clone(),
                parent: c.parent.clone(),
            });
            c.input_dirty = false;
        }
        if c.input_dirty || c.output_dirty {
            out.push(Piece::Fill { id: id.to_string(), tool: c.tool.clone(), input: c.input.clone(), output: c.output.clone() });
            c.input_dirty = false;
            c.output_dirty = false;
        }
        if ready {
            c.done = true;
            out.push(complete_piece(id, c, false));
        }
        out
    }
}

fn complete_piece(id: &str, c: &Call, fresh: bool) -> Piece {
    Piece::Complete {
        id: id.to_string(),
        tool: c.tool.clone(),
        start_ms: c.start_ms,
        end_ms: c.end_ms.unwrap_or(c.start_ms),
        error: if c.error_set { c.error } else { c.out_err },
        input: c.input.clone(),
        label: c.detail.clone(),
        output: c.output.clone().unwrap_or_default(),
        fresh,
        parent: c.parent.clone(),
    }
}

fn call_ord(items: &[Item], id: &str) -> Option<u64> {
    items.iter().find_map(|it| match &it.kind {
        ItemKind::Call { id: cid, .. } if cid == id => Some(it.ord),
        _ => None,
    })
}

/// Alive, ended or failed from the receipts and the harness pid. Transcript silence is not a signal.
/// No receipts and no pid answer is "unknown", never "alive".
pub fn session_life(source: &str, receipts: &[LiveReceipt], pid_alive: Option<bool>) -> &'static str {
    let last = receipts.iter().rev().find_map(|r| norm(source, &r.event, r.detail.as_deref()));
    match last {
        Some(Ev::End { .. }) => "ended",
        Some(Ev::Idle { failed: true }) => "failed",
        _ if pid_alive == Some(false) => "ended",
        _ if receipts.is_empty() && pid_alive.is_none() => "unknown",
        _ => "alive",
    }
}

/// Replay what the view has now. `spans` `Err` means Phoenix did not answer; receipts and the transcript still merge.
pub fn load(source: &str, transcript: &str, receipts: &[LiveReceipt], spans: Result<&[PhoenixSpan], ()>) -> (Vec<Piece>, Merger, bool) {
    let mut merger = Merger::default();
    let (pieces, offline) = merger.ingest(source, transcript, receipts, spans);
    (pieces, merger, offline)
}

/// Phoenix project is the harness name. The session key is the id the emitter put on `session.id`.
pub fn phoenix_session_id(file_name: &str, native_id: Option<&str>) -> String {
    let stem = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    let stem = stem.strip_suffix(".jsonl").unwrap_or(stem);
    if !stem.is_empty() {
        return stem.to_string();
    }
    native_id.and_then(|n| n.rsplit(['/', '\\']).next()).unwrap_or("").to_string()
}

fn receipt_signals(source: &str, receipts: &[LiveReceipt]) -> Vec<Signal> {
    let mut out = Vec::new();
    for r in receipts {
        let Some(id) = r.tool_use_id.clone().filter(|s| !s.is_empty()) else { continue };
        let parent = r.parent_tool_use_id.clone();
        let (detail, sub) = (r.detail.clone().filter(|d| !d.is_empty()), r.agent_id.as_deref().is_some_and(|a| !a.is_empty()));
        match norm(source, &r.event, r.detail.as_deref()) {
            Some(Ev::Pre) => out.push(Signal {
                call_id: id,
                tool: r.tool.clone(),
                start_ms: Some(r.at_ms),
                end_ms: None,
                error: None,
                from_span: false,
                parent,
                detail,
                sub,
            }),
            Some(Ev::Post) => out.push(Signal {
                call_id: id,
                tool: r.tool.clone(),
                start_ms: None,
                end_ms: Some(r.at_ms),
                error: r.is_error,
                from_span: false,
                parent,
                detail,
                sub,
            }),
            _ => {}
        }
    }
    out
}

fn span_signals(spans: &[PhoenixSpan]) -> Vec<Signal> {
    let mut call_of: HashMap<&str, &str> = HashMap::new();
    for s in spans {
        if let Some(id) = s.call_id.as_deref() {
            call_of.insert(s.span_id.as_str(), id);
        }
    }
    let mut grouped: HashMap<String, Signal> = HashMap::new();
    for s in spans {
        let Some(id) = s.call_id.clone() else { continue };
        let parent = s.parent_id.as_deref().and_then(|p| call_of.get(p)).map(|c| (*c).to_string());
        let e = grouped.entry(id.clone()).or_insert_with(|| Signal {
            call_id: id,
            tool: None,
            start_ms: None,
            end_ms: None,
            error: None,
            from_span: true,
            parent: None,
            detail: None,
            sub: false,
        });
        let start = s.start.timestamp_millis();
        let end = s.end.timestamp_millis();
        e.start_ms = Some(e.start_ms.map_or(start, |x| x.min(start)));
        e.end_ms = Some(e.end_ms.map_or(end, |x| x.max(end)));
        if s.tool_name.is_some() && e.tool.is_none() {
            e.tool = s.tool_name.clone();
        }
        if e.parent.is_none() {
            e.parent = parent;
        }
        match s.status {
            SpanStatus::Error => e.error = Some(true),
            SpanStatus::Ok if e.error.is_none() => e.error = Some(false),
            _ => {}
        }
    }
    let mut v: Vec<Signal> = grouped.into_values().collect();
    v.sort_by_key(|s| s.start_ms.unwrap_or(0));
    v
}

/// Claude jsonl or Pi jsonl, content items only.
pub fn parse(source: &str, transcript: &str) -> Vec<Item> {
    if source == "pi" {
        parse_pi(transcript)
    } else {
        parse_claude(transcript)
    }
}

fn parse_claude(text: &str) -> Vec<Item> {
    let mut out = Vec::new();
    let mut ord = 0u64;
    let push = |out: &mut Vec<Item>, ord: &mut u64, at_ms: i64, kind: ItemKind| {
        out.push(Item { ord: *ord, at_ms, kind });
        *ord += 1;
    };
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let at = ts_ms(&v);
        if v.get("isMeta").and_then(|x| x.as_bool()) == Some(true) {
            continue;
        }
        let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let content = v.pointer("/message/content").cloned().unwrap_or(serde_json::Value::Null);
        if kind == "user" {
            if let Some(blocks) = content.as_array() {
                for b in blocks {
                    if b.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                        let Some(id) = b.get("tool_use_id").and_then(|t| t.as_str()).filter(|s| !s.is_empty()) else { continue };
                        push(&mut out, &mut ord, at, ItemKind::Result {
                            id: id.to_string(),
                            output: block_text(b.get("content").unwrap_or(&serde_json::Value::Null)),
                            is_error: b.get("is_error").and_then(|x| x.as_bool()).unwrap_or(false),
                        });
                    }
                }
            }
            let text = block_text(&content);
            if let Some(text) = user_text(&text) {
                push(&mut out, &mut ord, at, ItemKind::User(text));
            }
            continue;
        }
        if kind != "assistant" {
            continue;
        }
        let Some(blocks) = content.as_array() else { continue };
        for b in blocks {
            match b.get("type").and_then(|t| t.as_str()) {
                Some("thinking") => {
                    let body = b.get("thinking").and_then(|t| t.as_str()).unwrap_or("");
                    let body = if body.trim().is_empty() { "\u{200b}".to_string() } else { body.to_string() };
                    push(&mut out, &mut ord, at, ItemKind::Thinking(body, v.get("thinkingDurationMs").and_then(|d| d.as_i64())));
                }
                Some("text") => {
                    let body = b.get("text").and_then(|t| t.as_str()).unwrap_or("").trim();
                    if !body.is_empty() {
                        push(&mut out, &mut ord, at, ItemKind::Text(body.to_string()));
                    }
                }
                Some("tool_use") => {
                    let id = b.get("id").and_then(|t| t.as_str()).unwrap_or("");
                    if id.is_empty() {
                        continue;
                    }
                    let tool = b.get("name").and_then(|t| t.as_str()).unwrap_or("tool").to_string();
                    let input = b.get("input").cloned().unwrap_or(serde_json::Value::Null).to_string();
                    push(&mut out, &mut ord, at, ItemKind::Call { id: id.to_string(), tool, input });
                }
                _ => {}
            }
        }
    }
    out
}

fn ts_ms(v: &serde_json::Value) -> i64 {
    v.get("timestamp")
        .and_then(|t| t.as_str())
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map_or(0, |t| t.timestamp_millis())
}

fn user_text(text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    const HARNESS: [&str; 5] = ["<task-notification>", "<local-command-", "<system-reminder>", "<command-message>", "[Request interrupted by user"];
    if HARNESS.iter().any(|h| t.starts_with(h)) {
        return None;
    }
    Some(text.to_string())
}

fn block_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(a) => a
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()).or_else(|| b.as_str()))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn parse_pi(text: &str) -> Vec<Item> {
    let all: Vec<serde_json::Value> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    let entries = active_branch(all);
    let mut out = Vec::new();
    let mut ord = 0u64;
    let push = |out: &mut Vec<Item>, ord: &mut u64, at_ms: i64, kind: ItemKind| {
        out.push(Item { ord: *ord, at_ms, kind });
        *ord += 1;
    };
    for e in entries {
        let at = ts_ms(&e);
        let kind = e.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if kind == "custom_message" {
            let t = block_text(e.get("content").unwrap_or(&serde_json::Value::Null));
            if !t.trim().is_empty() {
                push(&mut out, &mut ord, at, ItemKind::User(t));
            }
            continue;
        }
        if kind != "message" {
            continue;
        }
        let role = e.pointer("/message/role").and_then(|r| r.as_str()).unwrap_or("");
        let content = e.pointer("/message/content").cloned().unwrap_or(serde_json::Value::Null);
        if role == "toolResult" {
            let m = &e["message"];
            let Some(id) = m.get("toolCallId").and_then(|t| t.as_str()).filter(|s| !s.is_empty()) else { continue };
            push(&mut out, &mut ord, at, ItemKind::Result {
                id: id.to_string(),
                output: block_text(m.get("content").unwrap_or(&serde_json::Value::Null)),
                is_error: m.get("isError").and_then(|x| x.as_bool()).unwrap_or(false),
            });
            continue;
        }
        if role == "user" {
            let t = block_text(&content);
            if !t.trim().is_empty() {
                push(&mut out, &mut ord, at, ItemKind::User(t));
            }
            continue;
        }
        if role != "assistant" {
            continue;
        }
        let Some(blocks) = content.as_array() else { continue };
        for b in blocks {
            match b.get("type").and_then(|t| t.as_str()) {
                Some("thinking") => {
                    let body = b.get("thinking").and_then(|t| t.as_str()).unwrap_or("");
                    let body = if body.trim().is_empty() { "\u{200b}".to_string() } else { body.to_string() };
                    push(&mut out, &mut ord, at, ItemKind::Thinking(body, None));
                }
                Some("text") => {
                    let body = b.get("text").and_then(|t| t.as_str()).unwrap_or("").trim();
                    if !body.is_empty() {
                        push(&mut out, &mut ord, at, ItemKind::Text(body.to_string()));
                    }
                }
                Some("toolCall") => {
                    let id = b.get("id").and_then(|t| t.as_str()).unwrap_or("");
                    if id.is_empty() {
                        continue;
                    }
                    let tool = b.get("name").and_then(|t| t.as_str()).unwrap_or("tool").to_string();
                    let input = b.get("arguments").cloned().unwrap_or(serde_json::Value::Null).to_string();
                    push(&mut out, &mut ord, at, ItemKind::Call { id: id.to_string(), tool, input });
                }
                _ => {}
            }
        }
    }
    out
}

/// The chain of `parentId`s from the last entry, oldest first. A file with no ids is the whole file.
fn active_branch(all: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    let id = |e: &serde_json::Value| e.get("id").and_then(|v| v.as_str()).map(str::to_string);
    let Some(last) = all.iter().rposition(|e| id(e).is_some()) else {
        return all;
    };
    let index: HashMap<String, usize> = all.iter().enumerate().filter_map(|(i, e)| id(e).map(|id| (id, i))).collect();
    let (mut chain, mut seen, mut cur) = (vec![last], HashSet::from([last]), last);
    while let Some(p) = all[cur].get("parentId").and_then(|v| v.as_str()).and_then(|p| index.get(p).copied()).filter(|p| seen.insert(*p)) {
        chain.push(p);
        cur = p;
    }
    chain.into_iter().rev().map(|i| all[i].clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phoenix::{PhoenixSpan, SpanStatus};
    use chrono::{TimeZone, Utc};
    fn pre(id: &str, tool: &str, at: i64) -> LiveReceipt {
        LiveReceipt { event: "PreToolUse".into(), tool: Some(tool.into()), tool_use_id: Some(id.into()), at_ms: at, ..Default::default() }
    }
    fn post(id: &str, at: i64, error: Option<bool>) -> LiveReceipt {
        LiveReceipt { event: "PostToolUse".into(), tool_use_id: Some(id.into()), at_ms: at, is_error: error, ..Default::default() }
    }
    #[test]
    fn receipt_before_transcript_opens_one_running_row_and_completes_only_with_content() {
        let mut m = Merger::default();
        let open = m.on_signal("c1".into(), Some("Bash".into()), Some(10), None, None, false, None, &[]);
        assert!(matches!(open.as_slice(), [Piece::Open { id, .. }] if id == "c1"));
        assert_eq!(m.running().map(|(_, t)| t).as_deref(), Some("Bash"));

        let text = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"looking"},{"type":"tool_use","id":"c1","name":"Bash","input":{"command":"ls"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"c1","content":"a.txt"}]}}"#;
        let items = parse("claude", text);
        let filled = m.on_tail(&items);
        assert!(filled.iter().any(|p| matches!(p, Piece::Text(t) if t == "looking")));
        assert!(filled.iter().any(|p| matches!(p, Piece::Fill { output: Some(o), .. } if o == "a.txt")));
        assert!(!filled.iter().any(|p| matches!(p, Piece::Complete { .. })), "result without an end stays running");

        let done = m.on_signal("c1".into(), None, None, Some(40), Some(false), false, None, &items);
        assert!(matches!(done.as_slice(), [Piece::Complete { fresh: false, output, error: false, .. }] if output == "a.txt"));
        assert!(m.running().is_none());
        assert!(m.on_signal("c1".into(), Some("Bash".into()), Some(10), None, None, false, None, &items).is_empty());
    }

    #[test]
    fn text_already_in_the_tail_is_emitted_before_the_open() {
        let text = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"first"},{"type":"tool_use","id":"c1","name":"Read","input":{"file_path":"a"}}]}}"#;
        let (pieces, _, _) = load("claude", text, &[pre("c1", "Read", 5)], Ok(&[]));
        let kinds: Vec<&str> = pieces
            .iter()
            .map(|p| match p {
                Piece::Text(_) => "text",
                Piece::Open { .. } => "open",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, vec!["text", "open"]);
    }

    #[test]
    fn late_text_follows_the_row() {
        let mut m = Merger::default();
        let open = m.on_signal("c1".into(), Some("Bash".into()), Some(1), None, None, false, None, &[]);
        assert!(matches!(open[0], Piece::Open { .. }));
        let items = parse("claude", r#"{"type":"assistant","message":{"content":[{"type":"text","text":"late"},{"type":"tool_use","id":"c1","name":"Bash","input":{}}]}}"#);
        let next = m.on_tail(&items);
        assert!(matches!(next.first(), Some(Piece::Text(t)) if t == "late"), "{next:?}");
    }

    #[test]
    fn span_with_content_is_one_completed_row_and_a_dead_phoenix_still_opens_from_the_receipt() {
        let text = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"c1","name":"Bash","input":{"command":"ls"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"c1","content":"ok"}]}}"#;
        let span = PhoenixSpan {
            span_id: "s".into(),
            trace_id: "t".into(),
            call_id: Some("c1".into()),
            name: "claude tool:Bash".into(),
            hook_event: None,
            tool_name: Some("Bash".into()),
            start: Utc.timestamp_millis_opt(1_000).unwrap(),
            end: Utc.timestamp_millis_opt(2_500).unwrap(),
            status: SpanStatus::Ok,
            parent_id: None,
            session_id: Some("sess".into()),
            agent_id: None,
            transcript_path: None,
        };
        let (pieces, _, offline) = load("claude", text, &[], Ok(std::slice::from_ref(&span)));
        assert!(!offline);
        assert!(matches!(pieces.as_slice(), [Piece::Complete { fresh: true, start_ms: 1000, end_ms: 2500, .. }]));

        let (pieces, merger, offline) = load("claude", "", &[pre("c1", "Bash", 5)], Err(()));
        assert!(offline);
        assert!(matches!(pieces.as_slice(), [Piece::Open { .. }]));
        assert_eq!(merger.running().map(|(_, t)| t).as_deref(), Some("Bash"));
        assert_eq!(session_life("claude", &[pre("c1", "Bash", 5)], Some(true)), "alive");
    }

    #[test]
    fn one_call_id_never_opens_twice_and_end_without_content_waits() {
        let text = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"c1","name":"Bash","input":{"command":"ls"}}]}}"#;
        let items = parse("claude", text);
        let mut m = Merger::default();
        m.on_signal("c1".into(), Some("Bash".into()), Some(1), Some(9), Some(true), true, None, &items);
        let again = m.on_signal("c1".into(), Some("Bash".into()), Some(1), Some(9), Some(true), true, None, &items);
        assert!(again.is_empty(), "{again:?}");
        assert!(m.running().is_some(), "ended span with no result stays open");
    }

    /// Session 9643f8cd: a hook-denied call (Pre, error result, no Post), then a Bash call the rtk hook rewrote
    /// between Pre and Post whose result lands in the transcript after its Post, then later calls.
    #[test]
    fn rewritten_command_completes_on_its_result_and_a_denied_call_does_not_stay_running() {
        let detail = |r: LiveReceipt, d: &str| LiveReceipt { detail: Some(d.into()), ..r };
        let denied = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_D","name":"Bash","input":{"command":"grep -r x ."}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","content":"PreToolUse:Bash hook error: (Recursive grep blocked.)","is_error":true,"tool_use_id":"toolu_D"}]}}"#;
        let call = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_C","name":"Bash","input":{"command":"cd /r; cat docs/leader.md","description":"Read leader external-agents proof and ACP matrix"}}]}}"#;
        let mut receipts = vec![
            detail(pre("toolu_D", "Bash", 1_000), "grep -r x ."),
            detail(pre("toolu_C", "Bash", 31_000), "cd /r; cat docs/leader.md"),
            detail(post("toolu_C", 31_118, Some(false)), "cd /r; rtk read docs/leader.md"),
        ];
        let t1 = format!("{denied}\n{call}");
        let (pieces, mut m, _) = load("claude", &t1, &receipts, Ok(&[]));
        assert!(pieces.iter().any(|p| matches!(p, Piece::Complete { id, error: true, .. } if id == "toolu_D")), "{pieces:?}");
        assert!(pieces.iter().any(|p| matches!(p, Piece::Open { id, .. } if id == "toolu_C")), "{pieces:?}");
        assert!(!pieces.iter().any(|p| matches!(p, Piece::Complete { id, .. } if id == "toolu_C")), "no result yet");

        let rest = r##"{"type":"user","message":{"content":[{"tool_use_id":"toolu_C","type":"tool_result","content":"# Remote external ACP proof"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"next"},{"type":"tool_use","id":"toolu_L","name":"Read","input":{"file_path":"a"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_L","content":"ok"}]}}"##;
        receipts.extend([pre("toolu_L", "Read", 40_000), post("toolu_L", 40_050, Some(false))]);
        let (next, _) = m.ingest("claude", &format!("{t1}\n{rest}"), &receipts, Ok(&[]));
        assert!(next.iter().any(|p| matches!(p, Piece::Complete { id, fresh: false, output, .. } if id == "toolu_C" && output.starts_with("# Remote"))), "{next:?}");
        assert!(next.iter().any(|p| matches!(p, Piece::Text(t) if t == "next")), "{next:?}");
        assert!(next.iter().any(|p| matches!(p, Piece::Complete { id, .. } if id == "toolu_L")), "{next:?}");
        assert!(m.running().is_none());
    }

    #[test]
    fn a_call_with_no_receipt_or_span_opens_from_the_transcript_and_holds_nothing_back() {
        let text = r#"{"type":"assistant","timestamp":"2026-11-08T22:38:00.000Z","message":{"content":[{"type":"tool_use","id":"c1","name":"Bash","input":{"command":"ls"}}]}}
{"type":"assistant","timestamp":"2026-11-08T22:38:01.000Z","message":{"content":[{"type":"text","text":"after"}]}}"#;
        let (pieces, mut m, _) = load("claude", text, &[], Ok(&[]));
        let kinds: Vec<&str> = pieces.iter().map(|p| match p { Piece::Open { .. } => "open", Piece::Text(_) => "text", _ => "other" }).collect();
        assert_eq!(kinds, ["open", "text"], "{pieces:?}");
        let done = format!("{text}\n{}", r#"{"type":"user","timestamp":"2026-11-08T22:38:02.500Z","message":{"content":[{"type":"tool_result","tool_use_id":"c1","content":"a.txt"}]}}"#);
        let (next, _) = m.ingest("claude", &done, &[], Ok(&[]));
        assert!(next.iter().any(|p| matches!(p, Piece::Complete { id, end_ms, start_ms, .. } if id == "c1" && end_ms - start_ms == 2_500)), "{next:?}");
        assert!(m.running().is_none());
    }

    /// Session 40e362e8: a subagent's Bash calls land on the parent's receipts (agent_id set) but never in its transcript.
    #[test]
    fn a_subagent_call_is_labelled_from_its_receipt_and_ends_on_its_post() {
        let sub = |r: LiveReceipt| LiveReceipt { agent_id: Some("a7c7757506b19c0c4".into()), detail: Some("git status --short".into()), ..r };
        let (pieces, m, _) = load("claude", "", &[sub(pre("toolu_S", "Bash", 1_000)), sub(post("toolu_S", 1_300, Some(false)))], Ok(&[]));
        assert!(matches!(pieces.as_slice(), [Piece::Open { .. }, Piece::Complete { id, label: Some(l), input: None, .. }] if id == "toolu_S" && l == "git status --short"), "{pieces:?}");
        assert!(m.running().is_none());
        let (pieces, _, _) = load("claude", "", &[sub(pre("toolu_S", "Bash", 1_000))], Ok(&[]));
        assert!(matches!(pieces.as_slice(), [Piece::Open { label: Some(l), .. }] if l == "git status --short"), "{pieces:?}");
    }

    #[test]
    fn calls_without_receipts_keep_their_place_before_a_receipted_call_and_thoughts_keep_their_duration() {
        let text = r#"{"type":"assistant","timestamp":"2026-11-08T22:38:00.000Z","message":{"content":[{"type":"tool_use","id":"a","name":"Bash","input":{"command":"ls"}}]}}
{"type":"user","timestamp":"2026-11-08T22:38:01.000Z","message":{"content":[{"type":"tool_result","tool_use_id":"a","content":"x"}]}}
{"type":"assistant","timestamp":"2026-11-08T22:38:03.000Z","thinkingDurationMs":1193,"message":{"content":[{"type":"thinking","thinking":"","signature":"s"}]}}
{"type":"assistant","timestamp":"2026-11-08T22:38:04.000Z","message":{"content":[{"type":"tool_use","id":"b","name":"Bash","input":{"command":"pwd"}}]}}"#;
        let (pieces, _, _) = load("claude", text, &[pre("b", "Bash", 1)], Ok(&[]));
        let order: Vec<String> = pieces
            .iter()
            .map(|p| match p {
                Piece::Complete { id, .. } | Piece::Open { id, .. } => id.clone(),
                Piece::Thinking { dur_ms, .. } => format!("thought {dur_ms:?}"),
                _ => "other".into(),
            })
            .collect();
        assert_eq!(order, ["a", "thought Some(1193)", "b"], "{pieces:?}");
    }

    #[test]
    fn running_is_keyed_on_the_call_id_not_the_tool_name() {
        let mut m = Merger::default();
        m.on_signal("b1".into(), Some("Bash".into()), Some(1), None, None, false, None, &[]);
        assert_eq!(m.running(), Some(("b1".to_string(), "Bash".to_string())));
        m.on_signal("b2".into(), Some("Bash".into()), Some(2), None, None, false, None, &[]);
        assert_eq!(m.running().map(|(id, _)| id).as_deref(), Some("b2"), "a second Bash call is a new running call");
    }

    #[test]
    fn session_life_uses_the_end_receipt_or_a_dead_pid() {
        let end = LiveReceipt { event: "SessionEnd".into(), at_ms: 9, ..Default::default() };
        assert_eq!(session_life("claude", &[pre("c", "Bash", 1), end.clone()], Some(true)), "ended");
        let fail = LiveReceipt { event: "StopFailure".into(), at_ms: 3, ..Default::default() };
        assert_eq!(session_life("claude", &[fail], Some(true)), "failed");
        assert_eq!(session_life("claude", &[pre("c", "Bash", 1)], Some(false)), "ended");
        assert_eq!(session_life("claude", &[], None), "unknown");
        assert_eq!(session_life("claude", &[], Some(true)), "alive");
    }
}
