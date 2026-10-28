//! `/traces` payload view, what Enter opens: a tool call's full command with line numbers, then its
//! result, or an event's full text. Uncapped and scrollable. The body text is the one the trajectory
//! rows use (`compose_body`); nothing here re-parses harness JSON.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use unicode_width::UnicodeWidthChar;

use crate::render::line_utils::truncate_str;
use crate::theme::Theme;
use crate::views::modal_window;
use crate::views::traces_modal::{RESULT_ERR_SEP, split_result};

/// Below this width the header drops to tool + status and the line-number margin goes.
const NARROW: usize = 60;

/// What the selected event is, which decides how its body is laid out.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) enum Shape {
    /// Message, thinking: wrapped text.
    #[default]
    Prose,
    /// A tool call: command, then its folded result.
    Call,
    /// A tool result on its own.
    Result,
}

/// The payload view's state. `body` is `None` until it is read.
pub(super) struct Payload {
    pub body: Option<Result<String, String>>,
    pub shape: Shape,
    /// `m`: metadata (the raw line, pretty) instead of the payload.
    pub meta: bool,
    /// `w`: soft-wrap; off truncates each line.
    pub wrap: bool,
}

impl Default for Payload {
    fn default() -> Self {
        Self { body: None, shape: Shape::Prose, meta: false, wrap: true }
    }
}

/// Header facts the modal owns: the event's title, whether its session is live, and formatted figures.
pub(super) struct Head {
    pub title: String,
    pub running: bool,
    pub duration: String,
    pub time: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Code,
    Sep,
    SepErr,
    Result,
    Prose,
    Note,
}

#[derive(Debug)]
struct Row {
    text: String,
    num: Option<usize>,
    kind: Kind,
}

impl Payload {
    /// Starts a payload read. The wrap choice stays; metadata goes back off.
    pub fn open(&mut self, body: Option<Result<String, String>>, shape: Shape) {
        *self = Self { body, shape, meta: false, wrap: self.wrap };
    }

    pub fn failed(&self) -> bool {
        matches!(self.body, Some(Err(_)))
    }

    /// `y`: the command (a call), the output (a result) or the text, as read: no margin, no separators.
    pub fn copy_text(&self) -> Option<String> {
        let text = self.body.as_ref()?.as_ref().ok()?;
        Some(if self.shape == Shape::Call { split_result(text).0 } else { text.as_str() }.to_string())
    }

    fn has_result(&self) -> Option<bool> {
        let text = self.body.as_ref()?.as_ref().ok()?;
        (self.shape == Shape::Call).then(|| !split_result(text).1.is_empty())
    }

    fn errored(&self) -> bool {
        matches!(&self.body, Some(Ok(t)) if self.shape == Shape::Call && split_result(t).1.starts_with(RESULT_ERR_SEP))
    }

    fn header(&self, head: &Head, width: usize) -> String {
        let status = match self.has_result() {
            _ if self.errored() => Some("✗ error"),
            Some(true) => Some("ok"),
            Some(false) => Some(if head.running { "running" } else { "no result" }),
            None => None,
        };
        let mut parts = vec![head.title.as_str()];
        parts.extend(status);
        if width >= NARROW {
            parts.extend([head.duration.as_str(), head.time.as_str()]);
        }
        parts.join(" · ")
    }

    /// Display rows at `width`, and the width of the line-number column (0 = no margin).
    fn layout(&self, width: usize) -> (Vec<Row>, usize) {
        let mut rows = Vec::new();
        let text = match &self.body {
            Some(Ok(t)) => t.as_str(),
            Some(Err(e)) => {
                rows.push(Row { text: format!("(could not read: {e}, press r to retry)"), num: None, kind: Kind::Note });
                return (rows, 0);
            }
            None => return (rows, 0),
        };
        let (own, res) = if self.shape == Shape::Call { split_result(text) } else { (text, "") };
        let (sep, res) = res.split_once('\n').unwrap_or((res, ""));
        let res = res.trim_matches('\n');
        let numbered = self.shape != Shape::Prose && width >= NARROW;
        let digits = own.lines().count().max(res.lines().count()).max(1).to_string().len();
        let margin = if numbered { digits + 3 } else { 0 };
        let avail = width.saturating_sub(margin).max(1);
        let prose = self.shape == Shape::Prose;
        let own_kind = match self.shape {
            Shape::Prose => Kind::Prose,
            Shape::Call => Kind::Code,
            Shape::Result => Kind::Result,
        };
        push_block(&mut rows, own, own_kind, numbered, avail, self.wrap, prose);
        if !sep.is_empty() {
            rows.push(Row { text: String::new(), num: None, kind: Kind::Note });
            let kind = if sep == RESULT_ERR_SEP { Kind::SepErr } else { Kind::Sep };
            rows.push(Row { text: sep.to_string(), num: None, kind });
            if res.trim().is_empty() {
                rows.push(Row { text: "(no output)".into(), num: None, kind: Kind::Note });
            } else {
                push_block(&mut rows, res, Kind::Result, numbered, avail, self.wrap, false);
            }
        }
        (rows, if numbered { digits } else { 0 })
    }
}

/// One block's lines: numbered from 1, each wrapped (words for prose, columns for code) or truncated.
fn push_block(rows: &mut Vec<Row>, text: &str, kind: Kind, numbered: bool, avail: usize, wrap: bool, prose: bool) {
    for (n, line) in text.lines().enumerate() {
        let pieces = match (wrap, prose) {
            (false, _) => vec![truncate_str(&line.replace('\t', "    "), avail)],
            (true, true) => prose_wrap(line, avail),
            (true, false) => hard_wrap(line, avail),
        };
        for (k, text) in pieces.into_iter().enumerate() {
            rows.push(Row { text, num: (numbered && k == 0).then_some(n + 1), kind });
        }
    }
}

/// `line` cut into `avail`-column pieces, nothing dropped (tabs read as four spaces).
pub(super) fn hard_wrap(line: &str, avail: usize) -> Vec<String> {
    let avail = avail.max(1);
    let (mut out, mut cur, mut w) = (Vec::new(), String::new(), 0);
    for c in line.replace('\t', "    ").chars() {
        let cw = UnicodeWidthChar::width(c).unwrap_or(0);
        if w + cw > avail && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
            w = 0;
        }
        cur.push(c);
        w += cw;
    }
    out.push(cur);
    out
}

/// `line` wrapped at words, its indent kept on every piece.
fn prose_wrap(line: &str, avail: usize) -> Vec<String> {
    let line = line.replace('\t', "    ");
    let body = line.trim_start();
    if body.is_empty() {
        return vec![String::new()];
    }
    let indent = &line[..line.len() - body.len()];
    // An indent that leaves no room for text is dropped.
    let (indent, room) = if indent.len() < avail { (indent, avail - indent.len()) } else { ("", avail) };
    modal_window::word_wrap(body, room).into_iter().map(|r| format!("{indent}{r}")).collect()
}

/// Draws the payload into `area`: header, a blank row, the body from `scroll` (clamped here) and, when it
/// does not fit, `first-last/total` on the last row.
pub(super) fn render(buf: &mut Buffer, area: Rect, p: &Payload, head: &Head, scroll: &mut usize, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let width = area.width as usize;
    let dim = Style::default().fg(theme.gray);
    let head_style = Style::default().fg(if p.errored() { theme.accent_error } else { theme.text_primary }).add_modifier(Modifier::BOLD);
    buf.set_span(area.x, area.y, &Span::styled(truncate_str(&p.header(head, width), width).as_str(), head_style), area.width);
    let height = area.height.saturating_sub(3) as usize;
    let top = area.y + 2;
    if p.body.is_none() {
        if height > 0 {
            let note = "loading…";
            let x = area.x + (width.saturating_sub(note.chars().count()) / 2) as u16;
            buf.set_span(x, top + (height / 2) as u16, &Span::styled(note, dim), area.width);
        }
        return;
    }
    let (rows, digits) = p.layout(width);
    *scroll = (*scroll).min(rows.len().saturating_sub(height));
    let margin = if digits > 0 { digits + 3 } else { 0 };
    for (i, row) in rows.iter().skip(*scroll).take(height).enumerate() {
        let y = top + i as u16;
        if let Some(n) = row.num {
            buf.set_span(area.x, y, &Span::styled(format!("{n:>digits$} │ "), dim), area.width);
        }
        let style = match row.kind {
            Kind::Code => Style::default().fg(theme.text_primary),
            Kind::Prose | Kind::Result => Style::default().fg(theme.text_primary),
            Kind::Sep => dim,
            Kind::SepErr => Style::default().fg(theme.accent_error),
            Kind::Note => dim,
        };
        let x = area.x + margin.min(width) as u16;
        buf.set_span(x, y, &Span::styled(row.text.as_str(), style), area.width.saturating_sub(margin as u16));
    }
    if rows.len() > height && area.height >= 4 {
        let pos = format!("{}-{}/{}", *scroll + 1, (*scroll + height).min(rows.len()), rows.len());
        let x = area.x + width.saturating_sub(pos.len()) as u16;
        buf.set_span(x, area.y + area.height - 1, &Span::styled(pos, dim), area.width);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::traces_modal::{RESULT_SEP, compose_body};

    fn call(cmd: &str, res: Option<(&str, bool)>) -> Payload {
        let own = Ok(Some(format!("{{\"input\":{{\"command\":{cmd:?}}}}}")));
        let res = res.map(|(r, err)| (Ok(Some(format!("{{\"output\":{r:?}}}"))), err));
        Payload { body: Some(compose_body(own, res)), shape: Shape::Call, meta: false, wrap: true }
    }

    fn texts(p: &Payload, width: usize) -> Vec<String> {
        p.layout(width).0.into_iter().map(|r| r.text).collect()
    }

    fn screen(p: &Payload, w: u16, h: u16, scroll: &mut usize) -> Vec<String> {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let head = Head { title: "Bash".into(), running: false, duration: "2.0s".into(), time: "12:00:00".into() };
        render(&mut buf, area, p, &head, scroll, &Theme::current());
        (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string()).collect()
    }

    #[test]
    fn call_renders_command_then_separator_then_the_whole_result() {
        let script = (1..=3).map(|i| format!("print({i})")).collect::<Vec<_>>().join("\n");
        let result = (1..=300).map(|i| format!("out {i}")).collect::<Vec<_>>().join("\n");
        let p = call(&script, Some((&result, false)));
        let rows = texts(&p, 80);
        let at = |s: &str| rows.iter().position(|r| r == s).unwrap_or_else(|| panic!("missing {s}"));
        assert!(at("print(1)") < at("print(3)") && at("print(3)") < at(RESULT_SEP) && at(RESULT_SEP) < at("out 1"));
        assert_eq!(at("out 300") - at("out 1"), 299, "every result line, none cut");
        assert!(rows.iter().all(|r| !r.contains('…')), "no truncation tail");
        let numbered: Vec<_> = p.layout(80).0.into_iter().filter(|r| r.kind == Kind::Result).map(|r| r.num).collect();
        assert_eq!((numbered.first(), numbered.last()), (Some(&Some(1)), Some(&Some(300))));
    }

    #[test]
    fn error_result_uses_the_error_separator() {
        let p = call("false", Some(("boom", true)));
        let (rows, _) = p.layout(80);
        assert!(rows.iter().any(|r| r.kind == Kind::SepErr && r.text == RESULT_ERR_SEP));
        assert!(!rows.iter().any(|r| r.kind == Kind::Sep));
        assert!(screen(&p, 80, 10, &mut 0)[0].contains("✗ error"));
    }

    #[test]
    fn call_without_result_is_the_command_alone() {
        let p = call("sleep 9", None);
        assert_eq!(texts(&p, 80), ["sleep 9"]);
        assert!(p.header(&Head { title: "Bash".into(), running: true, duration: "-".into(), time: "t".into() }, 80).contains("running"));
    }

    #[test]
    fn wrap_toggles_the_line_count_and_keeps_every_column() {
        let long = "x".repeat(150);
        let mut p = call(&long, None);
        assert_eq!(p.layout(60).0.len(), 3, "wrapped at the margin");
        assert_eq!(texts(&p, 60).concat(), long);
        p.wrap = false;
        assert_eq!(p.layout(60).0.len(), 1, "truncated to one row");
    }

    #[test]
    fn copy_text_is_the_command_only() {
        let p = call("a\nb", Some(("out", false)));
        assert_eq!(p.copy_text().as_deref(), Some("a\nb"));
        let prose = Payload { body: Some(Ok("hello  world".into())), shape: Shape::Prose, ..Payload::default() };
        assert_eq!(prose.copy_text().as_deref(), Some("hello  world"));
        assert_eq!(Payload::default().copy_text(), None);
    }

    #[test]
    fn prose_wraps_and_empty_result_says_so() {
        let p = Payload { body: Some(Ok("word ".repeat(40))), shape: Shape::Prose, ..Payload::default() };
        let rows = texts(&p, 30);
        assert!(rows.len() > 4 && rows.iter().all(|r| r.chars().count() <= 30), "{rows:?}");
        assert!(p.layout(30).0.iter().all(|r| r.num.is_none()));
        let blank = Payload { body: Some(Ok(format!("true\n\n{RESULT_SEP}\n\n"))), shape: Shape::Call, ..Payload::default() };
        assert!(texts(&blank, 80).contains(&"(no output)".to_string()));
    }

    #[test]
    fn scroll_clamps_and_the_footer_counts() {
        let p = call("c", Some((&(1..=100).map(|i| format!("l{i}")).collect::<Vec<_>>().join("\n"), false)));
        let mut scroll = 10_000;
        let rows = screen(&p, 80, 12, &mut scroll);
        assert_eq!(scroll, 103 - 9, "clamped to the last page");
        assert!(rows[11].ends_with("103/103"), "{:?}", rows[11]);
        assert!(rows[2].ends_with("l92"), "{:?}", rows[2]);
    }

    #[test]
    fn narrow_width_never_panics() {
        let mut scroll = 0;
        for w in 1..=60u16 {
            for p in [call("echo é世界\nsecond", Some(("out\n\n", true))), call("x", None), Payload::default(), Payload { body: Some(Err("e".into())), ..Payload::default() }] {
                for wrap in [true, false] {
                    screen(&Payload { wrap, ..p_clone(&p) }, w, 6, &mut scroll);
                }
            }
        }
        assert!(screen(&call("ls", None), 19, 6, &mut 0)[0].starts_with("Bash"));
    }

    fn p_clone(p: &Payload) -> Payload {
        Payload { body: p.body.clone(), shape: p.shape, meta: p.meta, wrap: p.wrap }
    }

    #[test]
    fn loading_and_failure_states() {
        assert!(screen(&Payload::default(), 40, 8, &mut 0).iter().any(|r| r.contains("loading…")));
        let failed = Payload { body: Some(Err("io".into())), ..Payload::default() };
        assert!(failed.failed());
        assert!(screen(&failed, 80, 8, &mut 0).iter().any(|r| r.contains("press r to retry")));
    }

    #[test]
    fn narrow_header_is_tool_and_status_and_the_margin_goes() {
        let p = call("ls", Some(("a", false)));
        assert_eq!(screen(&p, 59, 8, &mut 0)[0], "Bash · ok");
        assert_eq!(p.layout(59).1, 0);
        assert!(screen(&p, 80, 8, &mut 0)[0].contains("2.0s"));
        assert_eq!(p.layout(80).1, 1);
    }
}
