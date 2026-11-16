//! Steps page: this session by turn, one line per step.
//!
//! A turn starts at each prompt. Its header is `#turn prompt … duration`; under it one row per tool call of the
//! root agent, numbered from 1 within the turn: `n mark tool target … duration`. Nothing expands. A subagent shows
//! only as the `Agent` call that spawned it.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::{Ctx, Row};
use crate::render::line_utils::truncate_str;
use crate::views::traces_follow::{Body, Call, Follow, took};

/// One prompt and the root agent's calls after it.
struct Turn<'a> {
    prompt: &'a str,
    start: Option<i64>,
    /// Latest item time seen in the turn, ms.
    end: i64,
    calls: Vec<&'a Call>,
}

/// A session counts as live when something happened in it within this long of now; an idle one has no running calls.
const LIVE_MS: i64 = 10 * 60 * 1000;

/// A call with no result is running only in a live session: an old transcript's open call just never got one.
fn running(c: &Call, live: bool) -> bool {
    live && c.result.is_none() && !c.no_result
}

/// The newest sign of life: the index row's times (a snapshot from when the panel opened) or any item that arrived since.
fn live_at(follow: &Follow) -> i64 {
    let r = &follow.root;
    let items = follow.rows().iter().flat_map(|r| [r.ts, match &r.body { Body::Call(c) => Some(c.started_ms), _ => None }]);
    [r.last_event_at, r.updated_at, r.mtime_ms].into_iter().chain(items).flatten().max().unwrap_or(0)
}

fn turns(follow: &Follow) -> Vec<Turn<'_>> {
    let mut out: Vec<Turn> = Vec::new();
    for r in follow.rows().iter().filter(|r| r.agent == follow.root.id) {
        match &r.body {
            Body::User(t) => {
                let start = r.ts;
                out.push(Turn { prompt: t, start, end: start.unwrap_or(0), calls: Vec::new() });
            }
            Body::Call(c) => {
                // The backlog can begin mid-turn: its calls get a turn with no prompt.
                if out.is_empty() {
                    out.push(Turn { prompt: "", start: None, end: 0, calls: Vec::new() });
                }
                let t = out.last_mut().expect("a turn was just pushed");
                t.start.get_or_insert(c.started_ms);
                t.end = t.end.max(c.started_ms + c.result.as_ref().and_then(|r| r.ms).unwrap_or(0));
                t.calls.push(c);
            }
            Body::Text(_) | Body::Reasoning(_) | Body::End => {
                if let (Some(t), Some(ts)) = (out.last_mut(), r.ts) {
                    t.end = t.end.max(ts);
                }
            }
            _ => {}
        }
    }
    out
}

fn first_line(s: &str) -> &str {
    s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("")
}

/// `lead` + `body` cut to `width - tail` columns with a `…`, then `tail` right-aligned; one line, exactly `width` wide
/// unless the tail does not fit, in which case it is dropped.
fn line(lead: Vec<Span<'static>>, body: Span<'static>, tail: Span<'static>, width: usize) -> Line<'static> {
    let lead_w: usize = lead.iter().map(|s| s.content.width()).sum();
    let tail_w = tail.content.width();
    let (tail, tail_w) = if tail_w == 0 || lead_w + tail_w + 2 > width { (Span::raw(""), 0) } else { (tail, tail_w + 1) };
    let body = Span::styled(truncate_str(&body.content, width.saturating_sub(lead_w + tail_w)), body.style);
    let pad = width.saturating_sub(lead_w + body.content.width() + tail.content.width());
    let mut spans = lead;
    spans.extend([body, Span::raw(" ".repeat(pad)), tail]);
    Line::from(spans)
}

pub(super) fn rows(follow: &Follow, c: &Ctx) -> Vec<Row> {
    let th = c.theme;
    let fg = |color| Style::default().fg(color);
    let turns = turns(follow);
    let live_session = c.now_ms - live_at(follow) <= LIVE_MS;
    let mut out = Vec::new();
    for (n, t) in turns.iter().enumerate() {
        let live = n + 1 == turns.len() && t.calls.iter().any(|c| running(c, live_session));
        let dur = match t.start {
            Some(s) if live => took((c.now_ms - s).max(0)),
            Some(s) => took((t.end - s).max(0)),
            None => String::new(),
        };
        let prompt = if t.prompt.is_empty() { "(earlier)" } else { first_line(t.prompt) };
        let head = Style::default().fg(th.accent_user).add_modifier(Modifier::BOLD);
        out.push(Row::new(
            line(
                vec![Span::styled(format!("#{} ", n + 1), head)],
                Span::styled(prompt.to_string(), head),
                Span::styled(dur, fg(th.gray)),
                c.width,
            ),
            t.prompt.trim_end(),
        ));
        for (i, call) in t.calls.iter().enumerate() {
            // shortcut: the index's tool title is all there is (the generic `Tool` is what some transcripts record); a better name needs the raw line.
            let head = format!("{} {}", call.tool, first_line(&call.what)).trim_end().to_string();
            let (mark, color, state, dur) = match &call.result {
                Some(r) if !r.ok => ("✗", th.accent_error, format!("failed {}", first_line(&r.error)), r.ms),
                Some(r) => ("✓", th.accent_success, "ok".to_string(), r.ms),
                None if !running(call, live_session) => ("–", th.gray_dim, "no result".to_string(), None),
                None => ("▸", th.accent_running, "running".to_string(), None),
            };
            let shown = match &call.result {
                Some(_) => dur.map(took).unwrap_or_default(),
                None if !running(call, live_session) => String::new(),
                None => took((c.now_ms - call.started_ms).max(0)),
            };
            let text = if mark == "✗" { fg(th.accent_error) } else { fg(th.text_primary) };
            out.push(Row::new(
                line(
                    vec![
                        Span::styled(format!("{:>3} ", i + 1), fg(th.gray_dim)),
                        Span::styled(format!("{mark} "), fg(color)),
                    ],
                    Span::styled(head.clone(), text),
                    Span::styled(shown.clone(), fg(if mark == "▸" { th.accent_running } else { th.gray })),
                    c.width,
                ),
                format!("{head}  {state}  {shown}").trim_end(),
            ));
        }
    }
    if out.is_empty() {
        out.push(Row::plain(Line::from(Span::styled("no steps yet", fg(th.gray)))));
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use xai_grok_traces::SessionRow;

    use super::super::{Info, paint};
    use super::*;
    use crate::theme::Theme;

    const NOW: i64 = 100_000;

    fn item(kind: &str, agent: &str, depth: usize, ts: i64, extra: Value) -> Value {
        let mut v = json!({"v": 1, "kind": kind, "agent": agent, "depth": depth, "line": 0, "ts": ts});
        v.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
        v
    }

    fn call(agent: &str, ts: i64, id: &str, tool: &str, what: &str) -> Value {
        item("call", agent, 0, ts, json!({"call_id": id, "tool": tool, "what": what}))
    }

    fn done(agent: &str, ts: i64, id: &str, ok: bool, error: &str, ms: i64) -> Value {
        item("result", agent, 0, ts, json!({"call_id": id, "ok": ok, "error": error, "ms": ms}))
    }

    fn follow(items: &[Value]) -> Follow {
        let mut f = Follow::bare(SessionRow { id: "root".into(), source: "grok".into(), ..Default::default() });
        f.apply(items, NOW);
        f
    }

    /// Two turns; a failed step, an Agent step with subagent internals, a running step.
    fn filled() -> Follow {
        follow(&[
            item("user", "root", 0, 10_000, json!({"text": "fix the parser in src/lib.rs\nand then add a test for it"})),
            call("root", 10_500, "a", "Bash", "cargo test -p parser -- --nocapture"),
            done("root", 12_900, "a", true, "", 2_400),
            call("root", 13_000, "b", "Read", "src/lib.rs"),
            done("root", 13_100, "b", false, "no such file or directory", 100),
            call("root", 13_200, "c", "Agent", "review the fix"),
            item("agent_start", "sub", 1, 13_300, json!({"model": "grok-4.5", "label": "reviewer"})),
            call("sub", 13_400, "s1", "Grep", "inside the subagent"),
            done("sub", 13_500, "s1", true, "", 100),
            done("root", 20_200, "c", true, "", 7_000),
            item("user", "root", 0, 60_000, json!({"text": "now run the whole suite"})),
            call("root", 61_000, "d", "Bash", "cargo test --release"),
        ])
    }

    fn text(r: &Row) -> String {
        r.line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn at(f: &Follow, w: usize) -> Vec<Row> {
        let info = Info::default();
        rows(f, &Ctx { info: &info, theme: &Theme::current(), width: w, now_ms: NOW })
    }

    #[test]
    fn empty_43() {
        insta::assert_snapshot!(paint(&at(&follow(&[]), 43), 43));
    }

    #[test]
    fn empty_21() {
        insta::assert_snapshot!(paint(&at(&follow(&[]), 21), 21));
    }

    #[test]
    fn filled_43() {
        insta::assert_snapshot!(paint(&at(&filled(), 43), 43));
    }

    #[test]
    fn filled_21() {
        insta::assert_snapshot!(paint(&at(&filled(), 21), 21));
    }

    #[test]
    fn numbering_restarts_each_turn_and_subagent_internals_are_left_out() {
        let r = at(&filled(), 43);
        let nums: Vec<String> = r
            .iter()
            .map(text)
            .filter(|t| !t.starts_with('#'))
            .map(|t| t.split_whitespace().next().unwrap().to_string())
            .collect();
        assert_eq!(nums, ["1", "2", "3", "1"]);
        assert!(r.iter().all(|r| !r.copy.contains("inside the subagent")));
    }

    #[test]
    fn rows_are_one_line_exactly_as_wide_as_the_panel() {
        for w in [21, 43] {
            for r in at(&filled(), w) {
                assert_eq!(text(&r).width(), w, "{:?}", text(&r));
            }
        }
    }

    #[test]
    fn copy_is_the_untruncated_step_and_the_whole_prompt() {
        let r = at(&filled(), 21);
        assert_eq!(r[0].copy, "fix the parser in src/lib.rs\nand then add a test for it");
        assert_eq!(r[1].copy, "Bash cargo test -p parser -- --nocapture  ok  2.4s");
        assert_eq!(r[2].copy, "Read src/lib.rs  failed no such file or directory  0.1s");
        assert!(text(&r[1]).contains('…'), "the line is cut, the copy is not");
        assert!(r.last().unwrap().copy.ends_with("running  39s"), "{}", r.last().unwrap().copy);
    }

    #[test]
    fn an_idle_session_has_no_running_step_and_its_turn_ends_at_its_last_item() {
        let mut f = filled();
        f.root.updated_at = Some(61_000);
        let info = Info::default();
        let stale = NOW + 28 * 24 * 3600 * 1000;
        let r = rows(&f, &Ctx { info: &info, theme: &Theme::current(), width: 43, now_ms: stale });
        let last = r.last().unwrap();
        assert!(text(last).contains('–') && !text(last).contains('▸'), "{}", text(last));
        assert_eq!(last.copy, "Bash cargo test --release  no result");
        assert!(text(&r[r.len() - 2]).trim_end().ends_with("1.0s"), "turn 2 header: {}", text(&r[r.len() - 2]));
        let live = rows(&f, &Ctx { info: &info, theme: &Theme::current(), width: 43, now_ms: 61_000 + 5_000 });
        assert!(text(live.last().unwrap()).contains('▸'), "a recent session still shows its running step");
    }

    #[test]
    fn a_call_with_no_target_shows_the_tool_name_alone() {
        let f = follow(&[
            item("user", "root", 0, 10_000, json!({"text": "go"})),
            call("root", 10_100, "a", "Tool", ""),
            done("root", 10_200, "a", true, "", 100),
        ]);
        let r = at(&f, 43);
        assert_eq!(r[1].copy, "Tool  ok  0.1s");
        assert!(text(&r[1]).contains("✓ Tool "), "{}", text(&r[1]));
    }
}
