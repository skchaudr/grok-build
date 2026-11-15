//! `grok-traces search`: script, host file, reply parsing, join against a real (synthetic) index, and ssh
//! failures against fake `ssh` scripts (no network). Synthetic fixtures only.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use rusqlite::Connection;
use serde_json::{Value, json};
use xai_grok_traces::cass::*;
use xai_grok_traces::remote::Ssh;
use xai_grok_traces::{EventFilter, SessionFilter, SessionRow, TraceIndex, default_roots};

const CLAUDE: &str = r#"{"type":"user","timestamp":"2026-10-01T00:00:00Z","cwd":"/w/claude","sessionId":"s","message":{"role":"user","content":"claude task"}}
{"type":"assistant","timestamp":"2026-10-01T00:00:05Z","message":{"role":"assistant","model":"claude-opus-5-5","content":[{"type":"tool_use","name":"Bash","input":{"command":"ls"}}],"usage":{"input_tokens":100,"output_tokens":20}}}
{"type":"user","timestamp":"2026-10-01T00:00:06Z","message":{"role":"user","content":[{"type":"tool_result","content":"a b"}]}}
"#;
const CODEX: &str = r#"{"timestamp":"2026-10-01T00:00:00Z","type":"session_meta","payload":{"cwd":"/w/codex","model":"gpt-5"}}
{"timestamp":"2026-10-01T00:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"codex task"}]}}
{"timestamp":"2026-10-01T00:00:02Z","type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{}"}}
"#;
const PI: &str = r#"{"type":"session","version":3,"id":"p","timestamp":"2026-10-01T00:00:00Z","cwd":"/w/pi"}
{"type":"message","timestamp":"2026-10-01T00:00:01Z","message":{"role":"user","content":[{"type":"text","text":"pi task"}]}}
{"type":"message","timestamp":"2026-10-01T00:00:02Z","message":{"role":"assistant","model":"pi-model","content":[{"type":"text","text":"ok"}]}}
"#;
const GROK: &str = r#"{"type":"system","content":"sys"}
{"type":"user","content":[{"type":"text","text":"grok task"}],"prompt_index":0}
{"type":"assistant","content":"x","model_id":"grok-5","tool_calls":[{"id":"t1","name":"read_file"}]}
{"type":"tool_result","tool_call_id":"t1","content":"ok"}
"#;

const CLAUDE_FILE: &str = ".claude/projects/-Users-sab-mini--config/d99a3941-ca04-4043-9635-814854149e83.jsonl";
const CODEX_FILE: &str = ".codex/sessions/2026/09/30/rollout-2026-09-30T17-34-05-01a0f4e1-fa60-72c3-8b36-f1ef3a924478.jsonl";
const PI_FILE: &str = ".pi/agent/sessions/--Users-sab-mini-repos-aa-cli--/2026-09-30T08-35-24-862Z_01a0f174-483d-70b0-bccd-387f37e6ff93.jsonl";
const GROK_FILE: &str = ".grok/sessions/%2Fw%2Fgrok/019fa941-3fed-71e0-9175-af925e0de274/chat_history.jsonl";

// The four real shapes of cass `source_path`.
const HIT_CLAUDE: &str = "/home/sab/agent-sessions/_cass/all/remotes/sab-air/mirror/.claude_projects_3b80a2ce8168961/projects/-Users-sab-mini--config/d99a3941-ca04-4043-9635-814854149e83.jsonl";
const HIT_CODEX_ZST: &str = "/home/sab/agent-sessions/_cass/all/remotes/sab-air/mirror/.codex_sessions_47a7e0570756d9a9/sessions/2026/09/30/rollout-2026-09-30T17-34-05-01a0f4e1-fa60-72c3-8b36-f1ef3a924478.jsonl.zst";
const HIT_PI: &str = "/home/sab/agent-sessions/_cass/all/remotes/sab-air/mirror/.pi_agent_sessions_fe4de5473d9c540f/sessions/--Users-sab-mini-repos-aa-cli--/2026-09-30T08-35-24-862Z_01a0f174-483d-70b0-bccd-387f37e6ff93.jsonl";
const HIT_GROK: &str = "/home/sab/agent-sessions/sab-dev/.grok/sessions/%2Fhome%2Fsab%2Faa-cli%2Fhub-rs/019fa941-3fed-71e0-9175-af925e0de274/chat_history.jsonl";

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// An index over one session per source, at the paths the hits above point to.
fn indexed() -> (tempfile::TempDir, TraceIndex) {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let grok = home.join(".grok");
    for (file, text) in [(CLAUDE_FILE, CLAUDE), (CODEX_FILE, CODEX), (PI_FILE, PI), (GROK_FILE, GROK)] {
        write(&home.join(file), text);
    }
    let mut idx = TraceIndex::open(&tmp.path().join("idx/traces.sqlite")).unwrap();
    let report = idx.scan(&default_roots(&home, &grok)).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    (tmp, idx)
}

fn session(idx: &TraceIndex, source: &str) -> SessionRow {
    let rows = idx.list_sessions(&SessionFilter { source: Some(source.into()), ..Default::default() }).unwrap();
    assert_eq!(rows.len(), 1, "{source}: {rows:?}");
    rows.into_iter().next().unwrap()
}

/// (trace line, timestamp) of the last event that has a timestamp.
fn last_stamped(idx: &TraceIndex, id: &str) -> (i64, i64) {
    let evs = idx.list_events(id, &EventFilter { include_noise: true, ..Default::default() }).unwrap();
    let e = evs.iter().rev().find(|e| e.timestamp.is_some()).expect("fixture has stamped events");
    let ts = e.timestamp.unwrap();
    // Expected line is the first event with that stamp, which is what the join must pick.
    (evs.iter().find(|x| x.timestamp == Some(ts)).unwrap().line, ts)
}

/// `join` that must succeed, taking hits by value as the tests build them.
fn joined(idx: &TraceIndex, hits: Vec<Hit>) -> Vec<Joined> {
    join(idx, &hits).unwrap()
}

fn hit(path: &str, created_at: Option<i64>) -> Hit {
    Hit {
        source_path: Some(path.into()),
        created_at,
        source_id: Some("sab-air".into()),
        snippet: Some("a snippet".into()),
        agent: Some("claude_code".into()),
        score: Some(1.5),
        ..Default::default()
    }
}

fn fake(dir: &tempfile::TempDir, body: &str, timeout: Duration) -> Ssh {
    let p = dir.path().join("ssh");
    fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    Ssh { bin: p.to_string_lossy().into(), timeout }
}

fn q(text: &str) -> Query {
    Query { text: text.into(), limit: DEFAULT_LIMIT, mode: None }
}

// ---- script ----

#[test]
fn script_quotes_quote_dollar_and_backslash() {
    let nasty = r"it's $(x) \ `y` done";
    assert_eq!(
        script(&q(nasty)),
        "exec env CASS_AUTO_REFRESH=0 CASS_DAEMON_AUTO_SPAWN=0 cass search 'it'\\''s $(x) \\ `y` done' --limit 20 --json\n"
    );
    // The shell must hand back exactly the query, with nothing expanded or executed.
    let out = Command::new("sh").arg("-c").arg(format!("printf %s {}", sh_quote(nasty))).output().unwrap();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), nasty);
    assert_eq!(sh_quote(""), "''");
}

#[test]
fn script_mode_limit_and_words() {
    let words: Vec<String> = ["auth", "token", "refresh"].map(String::from).to_vec();
    let mut query = Query::new(&words);
    assert_eq!((query.text.as_str(), query.limit, query.mode), ("auth token refresh", 20, None));
    query.mode = Some(Mode::Semantic);
    query.limit = 5;
    assert!(script(&query).ends_with("cass search 'auth token refresh' --limit 5 --json --mode semantic\n"), "{}", script(&query));
    query.limit = 0;
    assert!(script(&query).contains("--limit 1 "), "limit floors at 1");
    query.limit = 9999;
    assert!(script(&query).contains("--limit 200 "), "limit caps at 200");
    assert_eq!(Mode::parse("hybrid"), Some(Mode::Hybrid));
    assert_eq!(Mode::parse("Hybrid"), None);
    assert_eq!(script(&q("x")).lines().count(), 1);
}

// ---- host ----

#[test]
fn host_file_parse_and_resolve() {
    assert_eq!(parse_host("# c\n\n-oProxy=x\nbad host\nsab@sabibp # home server\nother\n").as_deref(), Some("sab@sabibp"));
    assert_eq!(parse_host("# nothing\n\n"), None);
    assert_eq!(parse_host(""), None);
    assert_eq!(resolve_host(Some("u@flag"), Some("sab@sabibp")).unwrap(), "u@flag", "--host wins");
    assert_eq!(resolve_host(None, Some("sab@sabibp\n")).unwrap(), "sab@sabibp");
    assert_eq!(resolve_host(None, None).unwrap_err(), "no cass host: put an ssh target in ~/.grok/traces/cass");
    assert_eq!(resolve_host(None, Some("# only a comment")).unwrap_err(), "no cass host: put an ssh target in ~/.grok/traces/cass");
    for bad in ["-oProxyCommand=x", "a b", "", "a#b", "a\nb"] {
        assert!(resolve_host(Some(bad), Some("sab@sabibp")).is_err(), "{bad:?}");
    }
    assert!(cass_path().ends_with("cass"));
}

// ---- reply parsing ----

#[test]
fn reply_tolerates_missing_and_null_fields() {
    let r = parse_reply(
        "u@h",
        r#"{"hits":[{"agent":"claude_code","created_at":1794393480780,"line_number":248,"score":0.5,"snippet":"s","source_id":"sab-air","source_path":"/p","extra":{"a":1}},
                    {"created_at":null,"score":null,"snippet":null},
                    {},
                    {"created_at":1794393480780.0,"line_number":"not a number"}],"total_matches":4,"cursor":null}"#,
    )
    .unwrap();
    assert_eq!(r.total_matches, Some(4));
    assert_eq!(r.hits.len(), 4);
    let h = &r.hits[0];
    assert_eq!((h.agent.as_deref(), h.created_at, h.line_number, h.score), (Some("claude_code"), Some(1794393480780), Some(248), Some(0.5)));
    assert_eq!((h.source_id.as_deref(), h.source_path.as_deref(), h.title.as_deref()), (Some("sab-air"), Some("/p"), None));
    assert_eq!(r.hits[1], Hit::default());
    assert_eq!(r.hits[2], Hit::default());
    assert_eq!((r.hits[3].created_at, r.hits[3].line_number), (Some(1794393480780), None));
    assert_eq!(parse_reply("u@h", r#"{"hits":[]}"#).unwrap(), Reply { hits: vec![], total_matches: None });
}

#[test]
fn reply_snippet_loses_cass_highlights_and_html_escapes() {
    let r = parse_reply(
        "u@h",
        r#"{"hits":[{"snippet":"Oh **shit**, it&#x27;s &quot;a &amp;&amp; b&quot; &lt;tag&gt; &amp;lt;", "content":"kept **as is** &amp;"}]}"#,
    )
    .unwrap();
    assert_eq!(r.hits[0].snippet.as_deref(), Some(r#"Oh shit, it's "a && b" <tag> &lt;"#));
    assert_eq!(r.hits[0].content.as_deref(), Some("kept **as is** &amp;"));
}

#[test]
fn reply_errors_are_parse_values() {
    for bad in ["", "not json", "[]", r#"{"total_matches":1}"#, r#"{"hits":3}"#, r#"{"hits":[1]}"#, r#"{"hits":[{}"#] {
        let e = parse_reply("u@sabibp", bad).unwrap_err();
        assert!(matches!(e, Error::Parse { .. }), "{bad}: {e}");
        assert!(e.to_string().starts_with("sabibp: bad response ("), "{e}");
    }
}

// ---- join ----

#[test]
fn join_resolves_each_path_shape() {
    let (_tmp, idx) = indexed();
    let hits = vec![hit(HIT_CLAUDE, None), hit(HIT_CODEX_ZST, None), hit(HIT_PI, None), hit(HIT_GROK, None)];
    let j = joined(&idx, hits);
    for (got, source) in j.iter().zip(["claude", "codex", "pi", "grok"]) {
        let s = session(&idx, source);
        assert_eq!(got.id.as_deref(), Some(s.id.as_str()), "{source}");
        assert_eq!(got.source.as_deref(), Some(source));
        assert_eq!(got.session_title.as_deref(), Some(s.title.as_str()));
        assert_eq!(got.line, None, "no created_at, no line");
    }
}

#[test]
fn join_keeps_order_and_leaves_unknown_hits_bare() {
    let (_tmp, idx) = indexed();
    let j = joined(
        &idx,
        vec![
            hit(HIT_GROK, None),
            hit("/elsewhere/other/x.jsonl", None),
            Hit { source_path: None, ..Default::default() },
            hit(HIT_CLAUDE, None),
        ],
    );
    let ids: Vec<_> = j.iter().map(|x| x.id.is_some()).collect();
    assert_eq!(ids, [true, false, false, true]);
    assert_eq!(j[0].source.as_deref(), Some("grok"));
    assert_eq!(j[3].source.as_deref(), Some("claude"));
    assert_eq!(j[1], Joined::bare(hit("/elsewhere/other/x.jsonl", None)));
}

#[test]
fn join_needs_two_components_not_just_the_file_name() {
    let (_tmp, idx) = indexed();
    // Same file name as the local grok session, different parent: shares one component.
    assert_eq!(joined(&idx, vec![hit("/home/sab/other-dir/chat_history.jsonl", None)])[0].id, None);
    // Bare name, and a one-component path.
    assert_eq!(joined(&idx, vec![hit("chat_history.jsonl", None)])[0].id, None);
    assert_eq!(joined(&idx, vec![hit("/chat_history.jsonl", None)])[0].id, None);
    // Right parent, wrong file name.
    assert_eq!(joined(&idx, vec![hit("/x/019fa941-3fed-71e0-9175-af925e0de274/updates.jsonl", None)])[0].id, None);
    // Parent + name alone is enough, whatever lies above.
    assert!(joined(&idx, vec![hit("019fa941-3fed-71e0-9175-af925e0de274/chat_history.jsonl", None)])[0].id.is_some());
}

#[test]
fn join_line_is_the_event_with_equal_timestamp() {
    let (_tmp, idx) = indexed();
    for (source, path) in [("claude", HIT_CLAUDE), ("pi", HIT_PI), ("codex", HIT_CODEX_ZST)] {
        let s = session(&idx, source);
        let (line, ts) = last_stamped(&idx, &s.id);
        let got = &joined(&idx, vec![hit(path, Some(ts))])[0];
        assert_eq!((got.id.as_deref(), got.line), (Some(s.id.as_str()), Some(line)), "{source}");
        // One ms off is no event: null, but the session still joins.
        let off = &joined(&idx, vec![hit(path, Some(ts + 1))])[0];
        assert_eq!((off.id.as_deref(), off.line), (Some(s.id.as_str()), None), "{source} ts+1");
    }
    // cass line_number is a message index and never becomes the trace line.
    let s = session(&idx, "claude");
    let (line, ts) = last_stamped(&idx, &s.id);
    let h = Hit { line_number: Some(line + 1000), ..hit(HIT_CLAUDE, Some(ts)) };
    assert_eq!(joined(&idx, vec![h])[0].line, Some(line));
    // A hit that matches no session has no line even with a timestamp.
    assert_eq!(joined(&idx, vec![hit("/a/b/c.jsonl", Some(ts))])[0].line, None);
}

// A tool result and the user message after it, written with one timestamp (as pi does).
const PI_TIED: &str = r#"{"type":"session","version":3,"id":"p","timestamp":"2026-10-01T00:00:00Z","cwd":"/w/pi"}
{"type":"message","timestamp":"2026-10-01T00:00:01Z","message":{"role":"assistant","model":"pi-model","content":[{"type":"text","text":"sending"}]}}
{"type":"message","timestamp":"2026-10-01T00:00:02Z","message":{"role":"toolResult","toolCallId":"c1","toolName":"intercom","content":[{"type":"text","text":"Reply sent to the subagent chat"}]}}
{"type":"message","timestamp":"2026-10-01T00:00:02Z","message":{"role":"user","content":[{"type":"text","text":"Oh no, is there\nan API key missing?"}]}}
"#;

#[test]
fn join_line_among_equal_timestamps_is_the_one_holding_the_hit_text() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    write(&home.join(PI_FILE), PI_TIED);
    let mut idx = TraceIndex::open(&tmp.path().join("idx/traces.sqlite")).unwrap();
    assert!(idx.scan(&default_roots(&home, &home.join(".grok"))).unwrap().errors.is_empty());
    let s = session(&idx, "pi");
    let evs = idx.list_events(&s.id, &EventFilter { include_noise: true, ..Default::default() }).unwrap();
    let ts = evs.last().unwrap().timestamp.unwrap();
    let tied: Vec<i64> = evs.iter().filter(|e| e.timestamp == Some(ts)).map(|e| e.line).collect();
    assert_eq!(tied.len(), 2, "the fixture has two events on one stamp: {evs:?}");
    let (result, user) = (tied[0], tied[1]);
    let line = |content: Option<&str>| {
        let h = Hit { content: content.map(str::to_string), ..hit(HIT_PI, Some(ts)) };
        joined(&idx, vec![h])[0].line
    };
    // The text decides, across the line break cass keeps and the transcript escapes.
    assert_eq!(line(Some("Oh no, is there\nan API key missing?")), Some(user));
    assert_eq!(line(Some("Reply sent to the subagent chat")), Some(result));
    // cass's own tags are not in the transcript and are skipped.
    assert_eq!(line(Some("[thinking] Oh no, is there an API key missing?")), Some(user));
    // No text, text too short to tell lines apart, or text in neither line: the first line stands.
    assert_eq!(line(None), Some(result));
    assert_eq!(line(Some("missing?")), Some(result));
    assert_eq!(line(Some("nothing either line says at all")), Some(result));
    // A transcript that is gone cannot break the tie and does not fail the join.
    fs::remove_file(home.join(PI_FILE)).unwrap();
    assert_eq!(line(Some("Oh no, is there an API key missing?")), Some(result));
}

#[test]
fn best_match_prefers_more_shared_components_and_refuses_ties() {
    let row = |id: &str, path: &str| SessionRow { id: id.into(), file_path: PathBuf::from(path), ..Default::default() };
    let a = row("a", "/Users/a/.grok/sessions/%2Fw/u1/chat_history.jsonl");
    let b = row("b", "/Users/b/.grok/sessions/%2Fx/u1/chat_history.jsonl");
    let hit_path = "/srv/s/.grok/sessions/%2Fw/u1/chat_history.jsonl";
    let cands = [a.clone(), b.clone()];
    // Both share u1/chat_history.jsonl; only `a` also shares %2Fw and more above it: a wins.
    assert_eq!(best_match(hit_path, &cands).map(|r| r.id.as_str()), Some("a"));
    // Both share exactly two: a tie is no match, in either order.
    let tie = "/srv/s/.grok/sessions/%2Fz/u1/chat_history.jsonl";
    assert!(best_match(tie, &cands).is_none());
    assert!(best_match(tie, &[b.clone(), a.clone()]).is_none());
    // A tie below the best does not matter.
    let c = row("c", "/q/u1/chat_history.jsonl");
    assert_eq!(best_match(hit_path, &[b.clone(), c.clone(), a.clone()]).map(|r| r.id.as_str()), Some("a"));
    // .zst is stripped on both sides.
    let z = row("z", "/m/2026/09/30/rollout-x.jsonl.zst");
    assert_eq!(best_match("/h/sessions/2026/09/30/rollout-x.jsonl", &[z.clone()]).map(|r| r.id.as_str()), Some("z"));
    assert_eq!(best_match("/h/sessions/2026/09/30/rollout-x.jsonl.zst", &[z]).map(|r| r.id.as_str()), Some("z"));
    // One shared component, an empty candidate list, and a one-component hit are all no match.
    assert!(best_match("/other/u2/chat_history.jsonl", &cands).is_none());
    assert!(best_match(hit_path, &[]).is_none());
    assert!(best_match("chat_history.jsonl", &cands).is_none());
}

// ---- output ----

#[test]
fn output_json_and_text_shapes() {
    let (_tmp, idx) = indexed();
    let s = session(&idx, "claude");
    let (line, ts) = last_stamped(&idx, &s.id);
    let j = joined(&idx, vec![hit(HIT_CLAUDE, Some(ts)), hit("/nowhere/x/y.jsonl", None)]);

    let v = joined_json(&j[0]);
    assert_eq!(
        v,
        json!({"id": s.id, "line": line, "source": "claude", "session_title": s.title, "agent": "claude_code", "origin": "sab-air",
               "score": 1.5, "created_at": ts, "snippet": "a snippet", "cass_path": HIT_CLAUDE})
    );
    let v = joined_json(&j[1]);
    for k in ["id", "line", "source", "session_title", "created_at"] {
        assert!(v[k].is_null(), "{k}");
    }
    assert_eq!(v["cass_path"], json!("/nowhere/x/y.jsonl"));

    assert_eq!(joined_line(&j[0]), format!("{}\t{line}\tsab-air\ta snippet", s.id));
    assert_eq!(joined_line(&j[1]), "-\t-\tsab-air\ta snippet");
    assert_eq!(summary(&j, Some(37)), "2 hits, 1 on this machine, 37 total");
    assert_eq!(summary(&j, None), "2 hits, 1 on this machine, 2 total");
}

#[test]
fn text_line_flattens_whitespace_and_cuts_snippet() {
    let h = Hit { snippet: Some(format!("a\nb\r\nc\td{}", "x".repeat(300))), ..Default::default() };
    let line = joined_line(&Joined::bare(h));
    let cols: Vec<_> = line.split('\t').collect();
    assert_eq!(cols.len(), 4, "{line:?}");
    assert_eq!(&cols[..3], ["-", "-", "-"]);
    assert_eq!(cols[3].chars().count(), 120);
    assert!(cols[3].starts_with("a b  c d"), "{}", cols[3]);
    // Cuts on chars, not bytes.
    let wide = Hit { snippet: Some("é".repeat(200)), ..Default::default() };
    assert_eq!(joined_line(&Joined::bare(wide)).split('\t').last().unwrap().chars().count(), 120);
}

// ---- ssh ----

const REPLY: &str = r#"{"hits":[{"agent":"claude_code","created_at":null,"score":2.0,"snippet":"it's a hit","source_id":"sab-air","source_path":"/x/y/z.jsonl"}],"total_matches":9}"#;

#[test]
fn search_pipes_the_script_on_stdin_and_keeps_the_query_off_the_command_line() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().display();
    fs::write(d.path().join("reply.json"), REPLY).unwrap();
    let ssh = fake(&d, &format!("cat > '{p}/stdin'\necho \"$*\" > '{p}/args'\ncat '{p}/reply.json'"), WALL_TIMEOUT);
    let query = Query { text: r"it's $(x) \ done".into(), limit: 7, mode: Some(Mode::Hybrid) };
    let r = search_with(&ssh, "sab@sabibp", &query).unwrap();
    assert_eq!((r.hits.len(), r.total_matches), (1, Some(9)));
    assert_eq!(r.hits[0].snippet.as_deref(), Some("it's a hit"));
    let args = fs::read_to_string(d.path().join("args")).unwrap();
    assert_eq!(
        args.trim_end(),
        "-o BatchMode=yes -o ConnectTimeout=2 -o ServerAliveInterval=2 -o ServerAliveCountMax=2 sab@sabibp sh -s"
    );
    assert_eq!(fs::read_to_string(d.path().join("stdin")).unwrap(), script(&query));
    assert!(!args.contains("done") && !args.contains("cass"), "query leaked onto the command line: {args}");
}

#[test]
fn unreachable_remote_failures_and_garbage_are_values() {
    let d = tempfile::tempdir().unwrap();
    let run = |body: &str| search_with(&fake(&d, body, WALL_TIMEOUT), "sab@sabibp", &q("x")).unwrap_err();

    let e = run("cat >/dev/null; echo 'ssh: Could not resolve hostname sabibp' >&2; exit 255");
    assert!(matches!(e, Error::Unreachable { .. }), "{e}");
    assert_eq!(e.to_string(), "sabibp unreachable (ssh: Could not resolve hostname sabibp)");

    let e = run("cat >/dev/null; echo 'fish: Unknown command: cass' >&2; exit 127");
    assert!(matches!(e, Error::Remote { .. }), "{e}");
    assert_eq!(e.to_string(), "sabibp: exit 127: cass not installed there");

    let e = run("cat >/dev/null; echo 'error: unexpected argument' >&2; echo ignored; exit 2");
    assert!(matches!(e, Error::Remote { .. }), "{e}");
    assert_eq!(e.to_string(), "sabibp: exit 2: error: unexpected argument");

    let e = run("cat >/dev/null; exit 1");
    assert_eq!(e.to_string(), "sabibp: exit 1: no message");

    let e = run("cat >/dev/null; echo 'this is not json'");
    assert!(matches!(e, Error::Parse { .. }), "{e}");
    assert!(e.to_string().starts_with("sabibp: bad response ("), "{e}");

    let e = run("cat >/dev/null; echo '{\"nothing\":1}'");
    assert!(matches!(e, Error::Parse { .. }), "{e}");

    let missing = Ssh { bin: "/no/such/ssh".into(), timeout: WALL_TIMEOUT };
    let e = search_with(&missing, "sab@sabibp", &q("x")).unwrap_err();
    assert!(matches!(e, Error::Unreachable { .. }), "{e}");
    assert!(e.to_string().starts_with("sabibp unreachable (ssh:"), "{e}");
}

#[test]
fn hung_ssh_is_killed_at_the_deadline() {
    let d = tempfile::tempdir().unwrap();
    let ssh = fake(&d, "exec sleep 30", Duration::from_millis(300));
    let t = Instant::now();
    let e = search_with(&ssh, "sab@sabibp", &q("x")).unwrap_err();
    assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    assert!(matches!(e, Error::Unreachable { .. }), "{e}");
    assert_eq!(e.to_string(), "sabibp unreachable (timed out after 300ms)");
}

#[test]
fn search_then_join_end_to_end() {
    let (_tmp, idx) = indexed();
    let s = session(&idx, "claude");
    let (line, ts) = last_stamped(&idx, &s.id);
    let reply = json!({
        "hits": [
            {"agent": "claude_code", "created_at": ts, "line_number": 3, "score": 9.5, "snippet": "first", "source_id": "sab-air", "source_path": HIT_CLAUDE},
            {"agent": "codex", "created_at": null, "snippet": "second", "source_id": "sab-dev", "source_path": "/not/here.jsonl"},
        ],
        "total_matches": 42,
    });
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("reply.json"), reply.to_string()).unwrap();
    let ssh = fake(&d, &format!("cat >/dev/null\ncat '{}/reply.json'", d.path().display()), WALL_TIMEOUT);
    let r = search_with(&ssh, "sab@sabibp", &q("first")).unwrap();
    let total = r.total_matches;
    let j = joined(&idx, r.hits);
    assert_eq!(joined_line(&j[0]), format!("{}\t{line}\tsab-air\tfirst", s.id));
    assert_eq!(joined_line(&j[1]), "-\t-\tsab-dev\tsecond");
    assert_eq!(summary(&j, total), "2 hits, 1 on this machine, 42 total");
}

// ---- review fixes: candidates past 200, fractional timestamps, index errors ----

#[test]
fn join_sees_every_candidate_even_past_200() {
    const N: usize = 205;
    const UUID: &str = "0198a000-0000-7000-8000-000000000001";
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    for i in 0..N {
        write(&home.join(format!(".grok/sessions/%2Fw{i}/{UUID}/chat_history.jsonl")), GROK);
    }
    let mut idx = TraceIndex::open(&tmp.path().join("idx/traces.sqlite")).unwrap();
    let report = idx.scan(&default_roots(&home, &home.join(".grok"))).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let rows = idx.list_sessions(&SessionFilter { limit: 1000, ..Default::default() }).unwrap();
    assert_eq!(rows.len(), N, "the index holds every one of the {N} sessions sharing the last two components");

    // Hits sharing three components with exactly one session join to that one, whichever order SQLite scans in.
    for row in &rows {
        let enc = row.file_path.parent().and_then(Path::parent).and_then(Path::file_name).unwrap().to_string_lossy().into_owned();
        let path = format!("/srv/x/{enc}/{UUID}/chat_history.jsonl");
        let got = &joined(&idx, vec![hit(&path, None)])[0];
        assert_eq!(got.id.as_deref(), Some(row.id.as_str()), "{enc}");
    }
    // Sharing only two with all of them is a tie across all of them: no match.
    let tie = format!("/srv/x/%2Fnowhere/{UUID}/chat_history.jsonl");
    assert_eq!(joined(&idx, vec![hit(&tie, None)])[0].id, None);
}

#[test]
fn timestamps_must_be_exact_integers() {
    let created = |ts: &str| parse_reply("u@h", &format!(r#"{{"hits":[{{"created_at":{ts}}}]}}"#)).unwrap().hits[0].created_at;
    assert_eq!(created("1794393480780"), Some(1794393480780));
    assert_eq!(created("1794393480780.0"), Some(1794393480780));
    assert_eq!(created("1794393480780.9"), None, "no truncation to a neighbouring millisecond");
    assert_eq!(created("-1794393480780.5"), None);
    assert_eq!(created("1e300"), None);
    assert_eq!(created("1e19"), None);
    assert_eq!(created("18446744073709551615"), None);
    assert_eq!(created("null"), None);
    assert_eq!(created(r#""1794393480780""#), None);
}

#[test]
fn fractional_created_at_joins_the_session_but_not_a_line() {
    let (_tmp, idx) = indexed();
    let s = session(&idx, "claude");
    let (line, ts) = last_stamped(&idx, &s.id);
    let reply = |frac: &str| {
        parse_reply("u@h", &format!(r#"{{"hits":[{{"source_path":"{HIT_CLAUDE}","created_at":{ts}.{frac}}}]}}"#)).unwrap()
    };
    let got = &joined(&idx, reply("9").hits)[0];
    assert_eq!((got.id.as_deref(), got.line), (Some(s.id.as_str()), None), "{ts}.9 is not the event at {ts}");
    let got = &joined(&idx, reply("0").hits)[0];
    assert_eq!((got.id.as_deref(), got.line), (Some(s.id.as_str()), Some(line)), "{ts}.0 is");
}

/// Drops a table through a second connection: the open `TraceIndex` still holds its handle but can no longer answer.
fn drop_table(idx: &TraceIndex, table: &str) {
    Connection::open(idx.db_path()).unwrap().execute_batch(&format!("DROP TABLE {table};")).unwrap();
}

#[test]
fn join_reports_a_broken_index_not_an_unmatched_hit() {
    let (_tmp, idx) = indexed();
    let hits = [hit(HIT_CLAUDE, None)];
    assert!(join(&idx, &hits).unwrap()[0].id.is_some());
    drop_table(&idx, "trace_sessions");
    let e = join(&idx, &hits).unwrap_err();
    assert!(e.to_string().contains("trace_sessions"), "{e}");
    // A hit with no path, or a path too short to name a session, never asks the index.
    assert_eq!(join(&idx, &[Hit::default(), hit("chat_history.jsonl", None)]).unwrap().len(), 2);
}

#[test]
fn join_reports_a_failed_line_lookup() {
    let (_tmp, idx) = indexed();
    let s = session(&idx, "claude");
    let (_, ts) = last_stamped(&idx, &s.id);
    drop_table(&idx, "trace_events");
    let e = join(&idx, &[hit(HIT_CLAUDE, Some(ts))]).unwrap_err();
    assert!(e.to_string().contains("trace_events"), "{e}");
    // No created_at: no line lookup, so the session still joins.
    assert_eq!(join(&idx, &[hit(HIT_CLAUDE, None)]).unwrap()[0].id.as_deref(), Some(s.id.as_str()));
}

// ---- the binary ----

/// `tmp/home` is GROK_HOME; `tmp/bin/ssh` prints one hit that no local session can match.
fn bin_fixture() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let bin = tmp.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let ssh = bin.join("ssh");
    fs::write(
        &ssh,
        "#!/bin/sh\ncat >/dev/null\necho '{\"hits\":[{\"agent\":\"claude_code\",\"created_at\":null,\"score\":1.5,\"snippet\":\"a hit\",\"source_id\":\"sab-air\",\"source_path\":\"/x/y/z.jsonl\"}],\"total_matches\":1}'\n",
    )
    .unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
    (tmp, home)
}

fn run_search(tmp: &tempfile::TempDir, home: &Path) -> Output {
    let path = format!("{}:{}", tmp.path().join("bin").display(), std::env::var("PATH").unwrap_or_default());
    Command::new(env!("CARGO_BIN_EXE_grok-traces"))
        .args(["search", "x", "--host", "u@h", "--json"])
        .env("GROK_HOME", home)
        .env("PATH", path)
        .output()
        .unwrap()
}

fn assert_one_unjoined_hit(out: &Output) {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "{stdout}");
    let v: Value = serde_json::from_str(lines[0]).unwrap();
    assert!(v["id"].is_null() && v["line"].is_null(), "{v}");
    assert_eq!(v["snippet"], json!("a hit"));
    assert_eq!(v["cass_path"], json!("/x/y/z.jsonl"));
}

#[test]
fn bin_prints_unjoined_hits_and_exits_1_when_the_index_cannot_open() {
    let (tmp, home) = bin_fixture();
    // `traces` is a file, so the index directory cannot be created.
    fs::write(home.join("traces"), "not a directory").unwrap();
    let out = run_search(&tmp, &home);
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert_one_unjoined_hit(&out);
    assert!(err.contains("grok-traces: local index unavailable (") && err.contains("); hits are not joined"), "{err}");
}

#[test]
fn bin_prints_unjoined_hits_and_exits_1_when_the_join_fails() {
    let (tmp, home) = bin_fixture();
    {
        // A valid index that opens, then loses its sessions table.
        let idx = TraceIndex::open(&home.join("traces/traces.sqlite")).unwrap();
        drop_table(&idx, "trace_sessions");
    }
    let out = run_search(&tmp, &home);
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert_one_unjoined_hit(&out);
    assert!(err.contains("grok-traces: local index unavailable (") && err.contains("trace_sessions"), "{err}");
    assert!(err.contains("); hits are not joined"), "{err}");
}

#[test]
fn bin_exits_0_when_the_index_answers_even_with_no_match() {
    let (tmp, home) = bin_fixture();
    drop(TraceIndex::open(&home.join("traces/traces.sqlite")).unwrap());
    let out = run_search(&tmp, &home);
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{err}");
    assert_one_unjoined_hit(&out);
    assert!(err.contains("1 hits, 0 on this machine, 1 total") && !err.contains("not joined"), "{err}");
}
