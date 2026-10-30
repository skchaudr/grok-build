use unicode_width::UnicodeWidthChar;

use super::traces_modal::{RESULT_ERR_SEP, RESULT_SEP};

pub(crate) fn pretty(raw: Result<Option<String>, String>) -> String {
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
pub(crate) const TEXT_KEYS: [&str; 9] = [
    "text", "thinking", "output", "result", "stdout", "stderr", "command", "cmd", "script",
];
/// Tool inputs under these keys are walked for text. A string value is body text as-is.
pub(crate) const INPUT_KEYS: [&str; 4] = ["input", "arguments", "args", "params"];

/// Text rows in the follow screen before its `… +N lines` tail.
pub(crate) const INLINE_MAX_LINES: usize = 40;
/// Total rendered rows an inline call preview may occupy, wrapping included. Enter opens the full payload.
pub(crate) const INLINE_MAX_ROWS: usize = 10;
/// Below this width the gutter is `│` alone.
const NARROW: usize = 60;
/// Narrowest body the rows are laid out for; anything less is laid out as this and clipped by the caller.
const MIN_BODY_W: usize = 8;

fn gutter(width: usize) -> &'static str {
    if width < NARROW { "│" } else { "│ " }
}

/// A one-row placeholder under its header: `loading…`, `(no body)`, `(could not read, …)`.
pub(crate) fn note(msg: &str, width: usize) -> Vec<String> {
    vec![format!("{}{msg}", gutter(width))]
}

/// Whether `row` is a result separator, gutter included.
pub(crate) fn is_sep(row: &str) -> bool {
    let t = row.trim_start_matches(['│', ' ']);
    t == RESULT_SEP || t == RESULT_ERR_SEP
}

/// The text and failure flag of a result section as `split_result` hands it over (separator line first).
pub(crate) fn section(result: &str) -> Option<(&str, bool)> {
    if result.is_empty() {
        return None;
    }
    let (sep, text) = result.split_once('\n').unwrap_or((result, ""));
    Some((text, sep == RESULT_ERR_SEP))
}

/// A tool call's inline rows: its `command` block (the part its header does not show), then, when the
/// call has a result, a separator row and the result block. Every row carries the gutter; blocks are
/// single-spaced, one blank row between them. Nothing for an empty command and no result.
pub(crate) fn call_rows(command: &str, result: Option<(&str, bool)>, width: usize) -> Vec<String> {
    let width = width.max(MIN_BODY_W);
    let mut rows = Vec::new();
    push_block(&mut rows, command, width);
    if let Some((text, error)) = result {
        if !rows.is_empty() {
            rows.push(String::new());
        }
        rows.push(format!("{}{}", gutter(width), if error { RESULT_ERR_SEP } else { RESULT_SEP }));
        if block_lines(text).is_empty() {
            rows.extend(note("(no output)", width));
        } else {
            push_block(&mut rows, text, width);
        }
    }
    if rows.len() > INLINE_MAX_ROWS {
        let hidden = rows.len() - (INLINE_MAX_ROWS - 1);
        rows.truncate(INLINE_MAX_ROWS - 1);
        rows.push(format!("{}… +{hidden} rows · Enter for all", gutter(width)));
    }
    rows
}

/// `text` as gutter rows, soft-wrapped before the caller applies its total rendered-row cap.
fn push_block(rows: &mut Vec<String>, text: &str, width: usize) {
    let g = gutter(width);
    let avail = width - g.chars().count();
    for line in block_lines(text) {
        if line.is_empty() {
            rows.push(g.trim_end().to_string());
        } else {
            rows.extend(wrap_code(line, avail).into_iter().map(|piece| format!("{g}{piece}")));
        }
    }
}

/// The lines of `text` minus trailing blanks, with runs of blank lines collapsed to one.
fn block_lines(text: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for line in text.lines().map(str::trim_end) {
        if line.is_empty() && out.last().is_none_or(|p| p.is_empty()) {
            continue;
        }
        out.push(line);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out
}

/// `line` cut at `avail` columns, nothing dropped; each later piece starts with the `↪` marker.
fn wrap_code(line: &str, avail: usize) -> Vec<String> {
    let (mut out, mut cur, mut w) = (Vec::new(), String::new(), 0);
    for c in line.replace('\t', "    ").chars() {
        let cw = UnicodeWidthChar::width(c).unwrap_or(0);
        if w + cw > avail && !cur.is_empty() {
            out.push(std::mem::replace(&mut cur, "↪".to_string()));
            w = 1;
        }
        cur.push(c);
        w += cw;
    }
    out.push(cur);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::traces_modal::{compose_body, split_result};

    const W: usize = 80;

    fn lines(n: usize) -> String {
        (1..=n).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn placeholders_carry_the_gutter() {
        // Loading, read failed and no body are one gutter row each.
        assert_eq!(note("loading…", W), ["│ loading…"]);
        assert_eq!(note("(could not read, press l to retry)", W), ["│ (could not read, press l to retry)"]);
        assert_eq!(note("(no body)", 59), ["│(no body)"]);
    }

    #[test]
    fn a_call_without_a_result_is_its_command_only() {
        let rows = call_rows("import os\nprint(1)", None, W);
        assert_eq!(rows, ["│ import os", "│ print(1)"]);
        assert!(!rows.iter().any(|r| is_sep(r)));
        // The header already held the whole command: nothing to add.
        assert!(call_rows("", None, W).is_empty());
    }

    #[test]
    fn an_ok_result_follows_the_command_under_one_separator() {
        let rows = call_rows("print(1)", Some(("out a\nout b", false)), W);
        assert_eq!(rows, ["│ print(1)", "", &format!("│ {RESULT_SEP}"), "│ out a", "│ out b"]);
    }

    #[test]
    fn an_error_result_uses_the_error_separator() {
        let rows = call_rows("false", Some(("boom", true)), W);
        assert_eq!(rows, ["│ false", "", &format!("│ {RESULT_ERR_SEP}"), "│ boom"]);
        assert!(rows.iter().filter(|r| is_sep(r)).count() == 1);
    }

    #[test]
    fn an_empty_result_says_no_output() {
        assert_eq!(call_rows("true", Some(("\n\n", false)), W), ["│ true", "", &format!("│ {RESULT_SEP}"), "│ (no output)"]);
        // No command rows: no blank row before the separator either.
        assert_eq!(call_rows("", Some(("", false)), W), [format!("│ {RESULT_SEP}"), "│ (no output)".to_string()]);
    }

    #[test]
    fn blank_runs_collapse_and_edges_drop() {
        assert_eq!(call_rows("\n\na\n\n\n\nb\n\n", None, W), ["│ a", "│", "│ b"]);
        let rows = call_rows("a\n\n\nb", Some(("x\n\n\n\ny", false)), W);
        assert!(!rows.windows(2).any(|w| w[0].is_empty() && w[1].is_empty()), "{rows:?}");
        assert_eq!(rows.iter().filter(|r| r.as_str() == "│").count(), 2, "{rows:?}");
    }

    #[test]
    fn the_preview_caps_total_rendered_rows_after_wrapping() {
        let tail = |n: usize| call_rows(&lines(n), None, W);
        assert_eq!(tail(10).len(), 10);
        assert!(!tail(10).iter().any(|r| r.contains("Enter for all")));
        let rows = tail(11);
        assert_eq!(rows.len(), 10);
        assert!(rows.last().is_some_and(|r| r.contains("… +2 rows · Enter for all")), "{rows:?}");

        // A giant inline script and its output get one total budget, not forty source lines each.
        let rows = call_rows(&lines(100), Some((&lines(100), false)), W);
        assert_eq!(rows.len(), 10);
        assert_eq!(rows.iter().filter(|r| r.contains("Enter for all")).count(), 1);

        // One long source line can wrap beyond the preview budget too.
        let rows = call_rows(&"x".repeat(1_000), None, 20);
        assert_eq!(rows.len(), 10);
        assert!(rows.last().is_some_and(|r| r.contains("Enter for all")));
    }

    #[test]
    fn narrow_widths_drop_the_gutter_space_and_wrap_with_a_marker() {
        let long = "x".repeat(30);
        assert_eq!(call_rows("ab", None, 60), ["│ ab"]);
        assert_eq!(call_rows("ab", None, 59), ["│ab"]);
        // 20 wide, gutter `│`: 19 columns a row, continuations led by `↪`, nothing cut.
        let rows = call_rows(&long, None, 20);
        assert_eq!(rows, [format!("│{}", "x".repeat(19)), format!("│↪{}", "x".repeat(11))]);
        let rows = call_rows(&long, None, 59);
        assert_eq!(rows, [format!("│{}", "x".repeat(30))]);
        let rows = call_rows(&"y".repeat(130), None, 70);
        assert_eq!(rows.iter().map(|r| r.chars().filter(|&c| c == 'y').count()).sum::<usize>(), 130);
        assert!(rows.iter().all(|r| cols_of(r) <= 70), "{rows:?}");
    }

    fn cols_of(s: &str) -> usize {
        s.chars().map(|c| UnicodeWidthChar::width(c).unwrap_or(0)).sum()
    }

    #[test]
    fn narrow_previews_stay_bounded_and_wrap_code_loses_nothing() {
        let cmd = "python3 -c \"print('中文 ok')\"\n\n\tindented";
        for w in 0..=12 {
            let width = w.max(MIN_BODY_W);
            let rows = call_rows(cmd, Some(("some output here", true)), w);
            assert!(rows.len() <= INLINE_MAX_ROWS, "w={w} {rows:?}");

            // Full-fidelity wrapping is independent of the inline preview's intentional truncation.
            let avail = width - gutter(width).chars().count();
            let text: String = block_lines(cmd)
                .iter()
                .flat_map(|line| wrap_code(line, avail))
                .map(|r| r.strip_prefix('↪').unwrap_or(&r).to_string())
                .collect();
            assert_eq!(text, "python3 -c \"print('中文 ok')\"    indented", "w={w}");
        }
    }

    #[test]
    fn sections_split_the_separator_from_the_text() {
        assert_eq!(section(""), None);
        assert_eq!(section(&format!("{RESULT_SEP}\nok\nmore")), Some(("ok\nmore", false)));
        assert_eq!(section(&format!("{RESULT_ERR_SEP}\nboom")), Some(("boom", true)));
        assert_eq!(section(RESULT_SEP), Some(("", false)));
    }

    #[test]
    fn accept_an_inline_script_reads_as_code_and_single_spaced_output() {
        let command = "python3 -c \"\nimport json\nrows = json.load(open('x'))\nprint(len(rows))\n\"";
        let call = serde_json::json!({"type":"assistant","message":{"role":"assistant","content":[
            {"type":"tool_use","name":"Bash","input":{"command": command, "description": "count rows"}}]}});
        let result = serde_json::json!({"type":"user","message":{"role":"user","content":[
            {"type":"tool_result","content":[{"type":"text","text":"rows: 3"},{"type":"text","text":"done"}]}]},
            "usage":{"input":3005,"output":0}});
        let body = compose_body(Ok(Some(call.to_string())), Some((Ok(Some(result.to_string())), false))).unwrap();
        let (own, res) = split_result(&body);
        // The header showed the first line; the rest is what the body adds.
        let rest = own.strip_prefix("python3 -c \"\n").unwrap();
        let rows = call_rows(rest, section(res), 120);
        let screen = rows.join("\n");
        assert!(!screen.contains("\\n"), "{screen}");
        assert!(rows.iter().all(|r| r.trim_start_matches('│').trim() != "3005" && r.trim_start_matches('│').trim() != "0"), "{screen}");
        assert_eq!(
            rows,
            ["│ import json", "│ rows = json.load(open('x'))", "│ print(len(rows))", "│ \"", "", &format!("│ {RESULT_SEP}"), "│ rows: 3", "│ done"]
        );
    }
}
