//! Search another machine's `cass` index over `ssh` and join each hit to the local trace session.
//!
//! * Host: `--host`, else the first valid line of `~/.grok/traces/cass` (same rules as the `hosts` file).
//! * Transport: `ssh -o BatchMode=yes ... <target> 'sh -s'` with the one-line [`script`] on stdin. The query never
//!   touches the ssh command line: the remote login shell may be fish, `sh -s` is the only thing it parses.
//! * Never blocks: a 15 s wall clock (semantic queries embed first), child killed at the deadline, stdout capped.
//!   Every failure is an [`Error`] value whose `Display` is UI-ready ("sabibp unreachable (...)").
//! * Hits: `serde_json::Value`-parsed, every field optional. cass `line_number` is a message index, not a file
//!   line, and is never used as a trace line.
//! * Join: a local session matches when its file path and the hit path share at least [`MIN_SHARED`] trailing
//!   components (after a `.zst` strip). Most shared wins, a tie is no match. The trace line is the event whose
//!   timestamp equals the hit's `created_at` exactly, else none.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::query::SessionRow;
use crate::remote::{Ssh, label, parse_hosts};
use crate::{TraceIndex, default_db_path};

pub const WALL_TIMEOUT: Duration = Duration::from_secs(15);
pub const DEFAULT_LIMIT: usize = 20;
pub const MAX_LIMIT: usize = 200;
/// Trailing path components a hit and a local session must share to be the same file.
pub const MIN_SHARED: usize = 2;
const MAX_OUTPUT: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// ssh failed to connect/run, or timed out.
    Unreachable { host: String, detail: String },
    /// ssh worked, the remote command failed (`cass` missing, bad flag, ...).
    Remote { host: String, detail: String },
    /// Output was not what we expect.
    Parse { host: String, detail: String },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unreachable { host, detail } => write!(f, "{} unreachable ({detail})", label(host)),
            Error::Remote { host, detail } => write!(f, "{}: {detail}", label(host)),
            Error::Parse { host, detail } => write!(f, "{}: bad response ({detail})", label(host)),
        }
    }
}
impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Lexical,
    Semantic,
    Hybrid,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Lexical => "lexical",
            Mode::Semantic => "semantic",
            Mode::Hybrid => "hybrid",
        }
    }

    pub fn parse(s: &str) -> Option<Mode> {
        [Mode::Lexical, Mode::Semantic, Mode::Hybrid].into_iter().find(|m| m.as_str() == s)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    /// The query words joined by one space.
    pub text: String,
    /// Clamped to 1..=[`MAX_LIMIT`] when the script is built.
    pub limit: usize,
    /// `None` = cass's own default.
    pub mode: Option<Mode>,
}

impl Query {
    pub fn new(words: &[String]) -> Query {
        Query { text: words.join(" "), limit: DEFAULT_LIMIT, mode: None }
    }
}

/// One cass hit. Every field is optional on the wire; unknown fields are ignored.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hit {
    pub agent: Option<String>,
    pub content: Option<String>,
    pub conversation_id: Option<String>,
    /// Epoch ms.
    pub created_at: Option<i64>,
    /// cass's message index: NOT a line of the transcript file.
    pub line_number: Option<i64>,
    pub match_type: Option<String>,
    pub origin_host: Option<String>,
    pub origin_kind: Option<String>,
    pub score: Option<f64>,
    pub snippet: Option<String>,
    pub source_id: Option<String>,
    pub source_path: Option<String>,
    pub title: Option<String>,
    pub workspace: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reply {
    pub hits: Vec<Hit>,
    pub total_matches: Option<i64>,
}

/// A hit and the local session it came from, when this machine has it.
#[derive(Clone, Debug, PartialEq)]
pub struct Joined {
    pub hit: Hit,
    /// Local trace session id.
    pub id: Option<String>,
    /// Trace line of the event stamped at the hit's `created_at`.
    pub line: Option<i64>,
    pub source: Option<String>,
    pub session_title: Option<String>,
}

impl Joined {
    /// A hit with no local match (also what [`join`] gives an unmatched hit).
    pub fn bare(hit: Hit) -> Joined {
        Joined { hit, id: None, line: None, source: None, session_title: None }
    }
}

// ---- host ----

/// `~/.grok/traces/cass`, next to the index.
pub fn cass_path() -> PathBuf {
    default_db_path().parent().map_or_else(|| PathBuf::from("."), PathBuf::from).join("cass")
}

/// First valid line of the host file: `#` comments, nothing starting with `-` or containing whitespace.
pub fn parse_host(text: &str) -> Option<String> {
    parse_hosts(text).into_iter().next()
}

/// A target is passed to ssh as an argument, so it must survive the `hosts` file rules unchanged.
pub fn valid_target(t: &str) -> bool {
    matches!(parse_hosts(t).as_slice(), [h] if h == t)
}

/// `flag` wins over the host file text. The `Err` is the message to print (exit 2).
pub fn resolve_host(flag: Option<&str>, file: Option<&str>) -> std::result::Result<String, String> {
    match flag {
        Some(h) if valid_target(h) => Ok(h.to_string()),
        Some(h) => Err(format!("invalid --host {h:?}: expected an ssh target without spaces or a leading -")),
        None => file.and_then(parse_host).ok_or_else(|| "no cass host: put an ssh target in ~/.grok/traces/cass".to_string()),
    }
}

// ---- script ----

/// POSIX single quoting: `'` becomes `'\''`, everything else is literal.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The one-line script piped to `sh -s`. `exec` so the exit code is cass's own.
pub fn script(q: &Query) -> String {
    let mut s = format!(
        "exec env CASS_AUTO_REFRESH=0 CASS_DAEMON_AUTO_SPAWN=0 cass search {} --limit {} --json",
        sh_quote(&q.text),
        q.limit.clamp(1, MAX_LIMIT)
    );
    if let Some(m) = q.mode {
        s.push_str(" --mode ");
        s.push_str(m.as_str());
    }
    s.push('\n');
    s
}

// ---- ssh runner (same shape as `remote::run`, which is private; this one feeds stdin) ----

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(ssh: &Ssh, target: &str, script: &str) -> Result<Out> {
    let unreachable = |detail: String| Error::Unreachable { host: target.into(), detail };
    let mut child = Command::new(&ssh.bin)
        .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=2", "-o", "ServerAliveInterval=2", "-o", "ServerAliveCountMax=2"])
        .arg(target)
        .arg("sh -s")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| unreachable(format!("ssh: {e}")))?;
    // Written off-thread so a stuck remote cannot hold us past the deadline; dropping stdin sends EOF.
    if let Some(mut stdin) = child.stdin.take() {
        let bytes = script.as_bytes().to_vec();
        std::thread::spawn(move || {
            let _ = stdin.write_all(&bytes);
        });
    }
    fn drain(r: impl Read + Send + 'static) -> std::sync::mpsc::Receiver<Vec<u8>> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = r.take(MAX_OUTPUT).read_to_end(&mut b);
            let _ = tx.send(b);
        });
        rx
    }
    let out_rx = drain(child.stdout.take().ok_or_else(|| unreachable("no stdout pipe".into()))?);
    let err_rx = drain(child.stderr.take().ok_or_else(|| unreachable("no stderr pipe".into()))?);
    let deadline = Instant::now() + ssh.timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(unreachable(format!("timed out after {:?}", ssh.timeout)));
            }
            Err(e) => return Err(unreachable(e.to_string())),
        }
    };
    // A pipe that does not reach EOF in time is unknown output, not empty output.
    let grab = |rx: std::sync::mpsc::Receiver<Vec<u8>>| {
        rx.recv_timeout(Duration::from_millis(500)).map_err(|e| unreachable(format!("output not read: {e}")))
    };
    let (stdout, stderr) = (String::from_utf8_lossy(&grab(out_rx)?).into_owned(), String::from_utf8_lossy(&grab(err_rx)?).into_owned());
    let code = status.code().unwrap_or(-1);
    if code == 255 {
        return Err(unreachable(first_line(&stderr, "ssh failed")));
    }
    Ok(Out { code, stdout, stderr })
}

fn first_line(s: &str, empty: &str) -> String {
    let l = s.lines().find(|l| !l.trim().is_empty()).unwrap_or(empty).trim();
    l.chars().take(120).collect()
}

// ---- search ----

pub fn search(target: &str, q: &Query) -> Result<Reply> {
    search_with(&Ssh { bin: "ssh".into(), timeout: WALL_TIMEOUT }, target, q)
}

pub fn search_with(ssh: &Ssh, target: &str, q: &Query) -> Result<Reply> {
    let o = run(ssh, target, &script(q))?;
    match o.code {
        0 => parse_reply(target, &o.stdout),
        127 => Err(Error::Remote { host: target.into(), detail: "exit 127: cass not installed there".into() }),
        c => Err(Error::Remote { host: target.into(), detail: format!("exit {c}: {}", first_line(&o.stderr, "no message")) }),
    }
}

fn str_of(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

/// Integers, or a float that is exactly one (`1794393480780.0`): no fraction, within +-2^53 where f64 is exact.
/// `1794393480780.9` is not a millisecond we can name, so it is `None` rather than a truncated near-miss.
fn i64_of(v: &Value, k: &str) -> Option<i64> {
    const EXACT: f64 = 9_007_199_254_740_992.0; // 2^53
    let n = v.get(k)?;
    n.as_i64().or_else(|| n.as_f64().filter(|f| f.fract() == 0.0 && f.abs() <= EXACT).map(|f| f as i64))
}

fn hit_from_json(v: &Value) -> Hit {
    Hit {
        agent: str_of(v, "agent"),
        content: str_of(v, "content"),
        conversation_id: str_of(v, "conversation_id"),
        created_at: i64_of(v, "created_at"),
        line_number: i64_of(v, "line_number"),
        match_type: str_of(v, "match_type"),
        origin_host: str_of(v, "origin_host"),
        origin_kind: str_of(v, "origin_kind"),
        score: v.get("score").and_then(Value::as_f64),
        snippet: str_of(v, "snippet").map(|s| plain_snippet(&s)),
        source_id: str_of(v, "source_id"),
        source_path: str_of(v, "source_path"),
        title: str_of(v, "title"),
        workspace: str_of(v, "workspace"),
    }
}

/// `{"hits":[...], "total_matches":N, ...}`. A missing `hits` array or a hit that is not an object is an error.
pub fn parse_reply(target: &str, stdout: &str) -> Result<Reply> {
    let bad = |detail: String| Error::Parse { host: target.into(), detail };
    let v: Value = serde_json::from_str(stdout.trim()).map_err(|e| bad(e.to_string()))?;
    let arr = v.get("hits").and_then(Value::as_array).ok_or_else(|| bad("no hits array".into()))?;
    let mut hits = Vec::with_capacity(arr.len());
    for (n, h) in arr.iter().enumerate() {
        if !h.is_object() {
            return Err(bad(format!("hit {}: not an object", n + 1)));
        }
        hits.push(hit_from_json(h));
    }
    Ok(Reply { hits, total_matches: i64_of(&v, "total_matches") })
}

// ---- join ----

/// Path components, with a trailing `.zst` removed from the file name.
fn parts(path: &str) -> Vec<&str> {
    path.strip_suffix(".zst").unwrap_or(path).split('/').filter(|c| !c.is_empty()).collect()
}

/// Equal components counted from the end.
fn shared(a: &[&str], b: &[&str]) -> usize {
    a.iter().rev().zip(b.iter().rev()).take_while(|(x, y)| x == y).count()
}

/// The candidate sharing the most trailing components with `hit_path`, at least [`MIN_SHARED`].
/// A tie for the most is `None`: two sessions with the same tail cannot both be it.
pub fn best_match<'a>(hit_path: &str, cands: &'a [SessionRow]) -> Option<&'a SessionRow> {
    let hp = parts(hit_path);
    let mut best: Option<(&SessionRow, usize)> = None;
    let mut tie = false;
    for c in cands {
        let path = c.file_path.to_string_lossy();
        let n = shared(&hp, &parts(&path));
        if n < MIN_SHARED {
            continue;
        }
        match best {
            Some((_, b)) if n < b => {}
            Some((_, b)) if n == b => tie = true,
            _ => {
                best = Some((c, n));
                tie = false;
            }
        }
    }
    if tie { None } else { best.map(|(c, _)| c) }
}

/// `Ok(None)` = this machine does not have that session; `Err` = the index could not answer.
fn local_session(index: &TraceIndex, hit_path: &str) -> crate::Result<Option<SessionRow>> {
    let p = parts(hit_path);
    if p.len() < MIN_SHARED {
        return Ok(None);
    }
    // Candidates by the last two components in SQL: a bare file name like chat_history.jsonl matches thousands.
    let suffix = p[p.len() - MIN_SHARED..].join("/");
    let cands = index.sessions_by_path_suffix(&suffix)?;
    Ok(best_match(hit_path, &cands).cloned())
}

/// Joins each hit to the local session it came from, keeping cass order. A session that is simply not here
/// is `Ok` with a bare hit; an index that cannot answer is `Err` (the caller still has `hits` to show).
pub fn join(index: &TraceIndex, hits: &[Hit]) -> crate::Result<Vec<Joined>> {
    hits.iter()
        .map(|hit| -> crate::Result<Joined> {
            let local = match hit.source_path.as_deref() {
                Some(p) => local_session(index, p)?,
                None => None,
            };
            let Some(row) = local else { return Ok(Joined::bare(hit.clone())) };
            let line = match hit.created_at {
                Some(ts) => pick_line(index, &row.id, &index.lines_at(&row.id, ts)?, hit.content.as_deref()),
                None => None,
            };
            Ok(Joined { id: Some(row.id), line, source: Some(row.source), session_title: Some(row.title), hit: hit.clone() })
        })
        .collect()
}

/// A cass snippet as the text was written: cass HTML-escapes it and wraps the matched words in `**`, and
/// `content` carries neither.
fn plain_snippet(snippet: &str) -> String {
    let mut text = snippet.replace("**", "");
    // `&amp;` last, so `&amp;lt;` stays the `&lt;` that was written.
    for (from, to) in [("&#x27;", "'"), ("&#39;", "'"), ("&quot;", "\""), ("&lt;", "<"), ("&gt;", ">"), ("&amp;", "&")] {
        text = text.replace(from, to);
    }
    text
}

/// The line among `lines` (the events of one session that share the hit's timestamp) that holds the hit's text.
/// With one candidate, no usable text, or no candidate holding it, the first line stands; a transcript that
/// cannot be re-read is not an error here, the hit keeps the first line.
fn pick_line(index: &TraceIndex, id: &str, lines: &[i64], content: Option<&str>) -> Option<i64> {
    let first = *lines.first()?;
    if lines.len() == 1 {
        return Some(first);
    }
    let Some(needle) = content.and_then(needle) else { return Some(first) };
    let holds = |line: &i64| index.raw_line(id, *line).ok().flatten().is_some_and(|raw| haystack(&raw).contains(&needle));
    Some(lines.iter().copied().find(|l| holds(l)).unwrap_or(first))
}

/// Shortest text accepted as proof that a line holds a hit; anything shorter could sit in any line.
const MIN_NEEDLE: usize = 12;

/// The longest run of up to 8 consecutive plain words of a hit's `content`, single-spaced. cass wraps tool calls
/// and reasoning in its own tags (`[Tool: Bash - …]`, `[thinking] …`, `command=…`), which are not in the
/// transcript, so words carrying `[`, `]` or `=` end a run.
fn needle(content: &str) -> Option<String> {
    let words: Vec<&str> = content.split_whitespace().collect();
    let mut best = String::new();
    for run in words.split(|w| w.contains(['[', ']', '='])) {
        let text = run.iter().take(8).copied().collect::<Vec<_>>().join(" ");
        if text.len() > best.len() {
            best = text;
        }
    }
    (best.len() >= MIN_NEEDLE).then_some(best)
}

/// Every string of a transcript line, single-spaced: the JSON strings decoded, then the line itself with its
/// escapes loosened, since tool arguments are JSON inside a JSON string.
fn haystack(raw: &str) -> String {
    fn strings(v: &Value, out: &mut String) {
        match v {
            Value::String(s) => {
                out.push_str(s);
                out.push(' ');
            }
            Value::Array(a) => a.iter().for_each(|x| strings(x, out)),
            Value::Object(o) => o.values().for_each(|x| strings(x, out)),
            _ => {}
        }
    }
    let mut text = String::new();
    if let Ok(v) = serde_json::from_str::<Value>(raw) {
        strings(&v, &mut text);
    }
    text.push_str(&raw.replace("\\n", " ").replace("\\\"", "\"").replace("\\", ""));
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---- output ----

pub fn joined_json(j: &Joined) -> Value {
    json!({
        "id": j.id, "line": j.line, "source": j.source, "session_title": j.session_title,
        "agent": j.hit.agent, "origin": j.hit.source_id, "score": j.hit.score, "created_at": j.hit.created_at,
        "snippet": j.hit.snippet, "cass_path": j.hit.source_path,
    })
}

/// `<id or ->\t<line or ->\t<origin>\t<snippet>`: one line, snippet whitespace flattened and cut to 120 chars.
pub fn joined_line(j: &Joined) -> String {
    let snippet: String = j
        .hit
        .snippet
        .as_deref()
        .unwrap_or("")
        .chars()
        .map(|c| if matches!(c, '\n' | '\r' | '\t') { ' ' } else { c })
        .take(120)
        .collect();
    format!(
        "{}\t{}\t{}\t{snippet}",
        j.id.as_deref().unwrap_or("-"),
        j.line.map_or_else(|| "-".to_string(), |l| l.to_string()),
        j.hit.source_id.as_deref().unwrap_or("-")
    )
}

/// The stderr line after the hits.
pub fn summary(joined: &[Joined], total_matches: Option<i64>) -> String {
    let local = joined.iter().filter(|j| j.id.is_some()).count();
    format!("{} hits, {local} on this machine, {} total", joined.len(), total_matches.unwrap_or(joined.len() as i64))
}
