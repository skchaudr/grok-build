//! Dispatch tree, noise, real errors, `now`/`detail`, and the rebuild lock. Synthetic fixtures only.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use xai_grok_traces::{EventFilter, Limits, Root, SessionFilter, SessionRow, Status, TraceIndex, default_roots};

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn zstd_write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, zstd::encode_all(text.as_bytes(), 3).unwrap()).unwrap();
}

struct Env {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    grok: PathBuf,
    db: PathBuf,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let grok = home.join(".grok");
    Env { db: tmp.path().join("idx/traces.sqlite"), grok, home, _tmp: tmp }
}

impl Env {
    fn roots(&self) -> Vec<Root> {
        default_roots(&self.home, &self.grok)
    }
    fn scan(&self) -> TraceIndex {
        let mut idx = TraceIndex::open(&self.db).unwrap();
        let r = idx.scan(&self.roots()).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        idx
    }
}

fn rows(idx: &TraceIndex, f: SessionFilter) -> Vec<SessionRow> {
    idx.list_sessions(&f).unwrap()
}

fn by_title(idx: &TraceIndex, title: &str) -> SessionRow {
    rows(idx, SessionFilter::default()).into_iter().find(|s| s.title == title).unwrap_or_else(|| panic!("no session titled {title}"))
}

fn noisy() -> EventFilter {
    EventFilter { include_noise: true, ..Default::default() }
}

const PARENT: &str = r#"{"type":"user","timestamp":"2026-10-01T00:00:00Z","cwd":"/w","message":{"role":"user","content":"dispatch the workers"}}
{"type":"assistant","timestamp":"2026-10-01T00:00:01Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Agent","input":{"description":"Evict at cap","prompt":"a very long brief"}}]}}
{"type":"attachment","timestamp":"2026-10-01T00:00:02Z","attachment":{"type":"hook_success"}}
"#;
const A1: &str = r#"{"type":"user","timestamp":"2026-10-01T00:00:10Z","isSidechain":true,"message":{"role":"user","content":"GIANT INSTRUCTION PROMPT for the first worker"}}
{"type":"assistant","timestamp":"2026-10-01T00:00:11Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash","input":{"command":"cargo test -p x\nsecond line"}}]}}
{"type":"user","timestamp":"2026-10-01T00:00:12Z","message":{"role":"user","content":[{"type":"tool_result","content":"error: could not compile; exception; timeout; denied"}]}}
{"type":"assistant","timestamp":"2026-10-01T00:00:13Z","message":{"role":"assistant","content":[{"type":"text","text":"All green."}]}}
"#;
const A2: &str = r#"{"type":"user","timestamp":"2026-10-01T00:00:20Z","message":{"role":"user","content":"GIANT PROMPT for the second worker"}}
{"type":"assistant","timestamp":"2026-10-01T00:00:21Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/w/crates/x/src/lib.rs","old_string":"a"}}]}}
"#;
const A3: &str = r#"{"type":"user","timestamp":"2026-10-01T00:00:30Z","message":{"role":"user","content":"GIANT PROMPT for the third worker"}}
{"type":"assistant","timestamp":"2026-10-01T00:00:31Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash","input":{"command":"make"}}]}}
{"type":"user","timestamp":"2026-10-01T00:00:32Z","message":{"role":"user","content":[{"type":"tool_result","is_error":true,"content":"Error: boom\nmore detail"}]}}
"#;

fn claude_tree(e: &Env) -> PathBuf {
    let proj = e.home.join(".claude/projects/-w");
    write(&proj.join("p1.jsonl"), PARENT);
    let sub = proj.join("p1/subagents");
    write(&sub.join("agent-a1.jsonl"), A1);
    write(&sub.join("agent-a1.meta.json"), r#"{"agentType":"general-purpose","description":"Evict at cap","finished":true,"model":"sonnet"}"#);
    write(&sub.join("agent-a2.jsonl"), A2);
    write(&sub.join("agent-a2.meta.json"), r#"{"agentType":"Explore","description":"Map the crate","finished":false}"#);
    write(&sub.join("agent-a3.jsonl"), A3);
    write(&sub.join("agent-a3.meta.json"), r#"{"agentType":"general-purpose","description":"Fix the build","finished":true}"#);
    sub
}

#[test]
fn claude_subagents_nest_under_their_session() {
    let e = env();
    claude_tree(&e);
    let idx = e.scan();
    let parent = by_title(&idx, "dispatch the workers");
    assert_eq!((parent.parent_id.clone(), parent.children, parent.label.clone()), (None, 3, None));

    let kids = idx.children(&parent.id).unwrap();
    assert_eq!(kids.len(), 3);
    assert_eq!(kids.iter().map(|k| k.label.as_deref().unwrap()).collect::<Vec<_>>(), ["Evict at cap", "Map the crate", "Fix the build"], "started order");
    assert!(kids.iter().all(|k| k.parent_id.as_deref() == Some(parent.id.as_str()) && k.children == 0));
    assert_eq!(kids[1].agent_type.as_deref(), Some("Explore"));
    assert_eq!(kids[0].title, "GIANT INSTRUCTION PROMPT for the first worker", "title stays the prompt; the UI prefers label");

    // finished (meta) -> done; unfinished + fresh -> active; trailing tool error -> failed.
    assert_eq!((kids[0].status, kids[0].failed), (Status::Completed, false));
    assert_eq!((kids[1].status, kids[1].failed), (Status::Active, false));
    assert_eq!((kids[2].status, kids[2].failed), (Status::Completed, true));
    assert_eq!(kids[0].now.as_deref(), Some("All green."));
    assert_eq!(kids[1].now.as_deref(), Some("Edit: src/lib.rs"));
    assert_eq!(kids[2].now.as_deref(), Some("Bash: make"));

    // roots_only drops the dispatched ones.
    let roots = rows(&idx, SessionFilter { roots_only: true, ..Default::default() });
    assert_eq!(roots.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), ["dispatch the workers"]);
    assert_eq!(rows(&idx, SessionFilter::default()).len(), 4);
    // A child whose parent is not indexed is a root.
    assert!(idx.children(&kids[0].id).unwrap().is_empty());
    assert!(idx.children("claude:nope").unwrap().is_empty());
}

#[test]
fn claude_meta_change_refreshes_without_reparsing_the_transcript() {
    let e = env();
    let sub = claude_tree(&e);
    let mut idx = e.scan();
    let kid = by_title(&idx, "GIANT PROMPT for the second worker");
    assert_eq!((kid.status, kid.label.as_deref()), (Status::Active, Some("Map the crate")));

    // The harness marks it finished after its last transcript write.
    std::thread::sleep(Duration::from_millis(20));
    write(&sub.join("agent-a2.meta.json"), r#"{"agentType":"Explore","description":"Map the crate v2","finished":true}"#);
    let r = idx.scan(&e.roots()).unwrap();
    assert_eq!(r.indexed, 0, "transcript untouched");
    let kid = by_title(&idx, "GIANT PROMPT for the second worker");
    assert_eq!((kid.status, kid.label.as_deref()), (Status::Completed, Some("Map the crate v2")));
    assert_eq!(kid.event_count, 2);
}

#[test]
fn injected_context_is_noise_and_never_a_title() {
    let e = env();
    write(
        &e.home.join(".claude/projects/-w/i1.jsonl"),
        concat!(
            r#"{"type":"user","timestamp":"2026-10-01T00:00:00Z","isMeta":true,"message":{"role":"user","content":"<local-command-caveat>Caveat: ignore</local-command-caveat>"}}"#, "\n",
            r#"{"type":"user","timestamp":"2026-10-01T00:00:01Z","message":{"role":"user","content":"<command-name>/clear</command-name>\n<command-args></command-args>"}}"#, "\n",
            r#"{"type":"user","timestamp":"2026-10-01T00:00:02Z","message":{"role":"user","content":[{"type":"text","text":"<system-reminder>\nbig injected context\n</system-reminder>"},{"type":"text","text":"fix the parser"}]}}"#, "\n",
            r#"{"type":"user","timestamp":"2026-10-01T00:00:03Z","message":{"role":"user","content":"<system-reminder>only a reminder</system-reminder>"}}"#, "\n",
            r#"{"type":"user","timestamp":"2026-10-01T00:00:04Z","message":{"role":"user","content":"The beginning of the above subagent result is already visible to the user."}}"#, "\n",
            r#"{"type":"attachment","timestamp":"2026-10-01T00:00:05Z","attachment":{"type":"hook_success"}}"#, "\n",
            r#"{"type":"last-prompt","lastPrompt":"x"}"#, "\n",
            r#"{"type":"assistant","timestamp":"2026-10-01T00:00:06Z","message":{"role":"assistant","content":[{"type":"text","text":"On it."}]}}"#, "\n",
        ),
    );
    let idx = e.scan();
    let s = rows(&idx, SessionFilter::default()).remove(0);
    assert_eq!(s.title, "fix the parser");
    assert_eq!((s.event_count, s.user_count), (8, 5));
    let shown = idx.list_events(&s.id, &EventFilter::default()).unwrap();
    assert_eq!(shown.iter().map(|ev| ev.preview.as_str()).collect::<Vec<_>>(), ["fix the parser", "On it."], "wrapper stripped from the real message");
    assert_eq!(shown.iter().map(|ev| ev.line).collect::<Vec<_>>(), [2, 7]);
    let all = idx.list_events(&s.id, &noisy()).unwrap();
    assert_eq!(all.len(), 8);
    assert_eq!(all.iter().filter(|ev| ev.noise).count(), 6);
    assert_eq!(all[3].preview, "<system-reminder>", "a wrapper-only message says what it was");
}

#[test]
fn errors_come_from_flags_and_exit_codes_never_from_words() {
    let e = env();
    // Claude: the word "error" in a successful result; a flagged one; an input that mentions is_error.
    write(
        &e.home.join(".claude/projects/-w/c.jsonl"),
        concat!(
            r#"{"type":"user","timestamp":"2026-10-01T00:00:00Z","message":{"role":"user","content":"go"}}"#, "\n",
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/a/b.rs","new_string":"{\"is_error\": true}"}}]}}"#, "\n",
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"error exception failed denied timeout"}]}}"#, "\n",
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","is_error":true,"content":"Error: real"}]}}"#, "\n",
            r#"{"type":"assistant","isApiErrorMessage":true,"error":"rate_limit","message":{"role":"assistant","content":[{"type":"text","text":"API Error"}]}}"#, "\n",
        ),
    );
    // Codex: exit headers.
    let call = |n: u32| format!(r#"{{"timestamp":"2026-10-01T00:00:0{n}Z","type":"response_item","payload":{{"type":"function_call","name":"exec_command","arguments":"{{\"cmd\":\"ls\"}}"}}}}"#);
    let out = |n: u32, body: &str| format!(r#"{{"timestamp":"2026-10-01T00:00:0{n}Z","type":"response_item","payload":{{"type":"function_call_output","output":{}}}}}"#, serde_json::to_string(body).unwrap());
    write(
        &e.home.join(".codex/sessions/2026/10/01/rollout-a.jsonl"),
        &([
            call(1), out(2, "Process exited with code 0\nerror: failed denied exception timeout"),
            call(3), out(4, "Process exited with code 2\nboom"),
            call(5), out(6, "Exit code: 1\nWall time: 1s"),
        ].join("\n") + "\n"),
    );
    // Pi/OMP: isError on a toolResult message. DSH: error field, but "outcome unknown" is not a failure.
    write(
        &e.home.join(".pi/agent/sessions/--w--/p.jsonl"),
        concat!(
            r#"{"type":"message","message":{"role":"toolResult","toolName":"bash","isError":false,"content":[{"type":"text","text":"error error"}]}}"#, "\n",
            r#"{"type":"message","message":{"role":"toolResult","toolName":"bash","isError":true,"content":[{"type":"text","text":"command not found"}]}}"#, "\n",
            r#"{"type":"message","message":{"role":"assistant","stopReason":"error","errorMessage":"503","content":[]}}"#, "\n",
            r#"{"type":"message","message":{"role":"assistant","stopReason":"aborted","content":[]}}"#, "\n",
        ),
    );
    zstd_write(
        &e.home.join(".dsh/sessions/w/s/session.jsonl.zstd"),
        concat!(
            r#"{"type":"session","id":"d1","cwd":"/w"}"#, "\n",
            r#"{"type":"tool/result","time":1759276801000,"data":{"error":{"name":"Boom","code":"X"},"message":{"role":"user","content":[{"type":"tool-result","isError":true,"content":[{"type":"text","text":"nope"}]}]}}}"#, "\n",
            r#"{"type":"tool/result","time":1759276802000,"data":{"error":{"name":"ToolOutcomeUnknownError","code":"TOOL_OUTCOME_UNKNOWN"},"message":{"role":"user","content":[{"type":"tool-result","isError":true,"content":[{"type":"text","text":"unknown"}]}]}}}"#, "\n",
        ),
    );
    let idx = e.scan();
    let count = |src: &str| rows(&idx, SessionFilter { source: Some(src.into()), ..Default::default() }).remove(0);
    assert_eq!(count("claude").error_count, 2, "is_error result + API error, not the word or the input flag");
    let x = count("codex");
    assert_eq!(x.error_count, 2, "exit 2 and exit 1");
    let ev = idx.list_events(&x.id, &EventFilter::default()).unwrap();
    let details: Vec<_> = ev.iter().filter(|v| v.role.as_deref() == Some("tool_result")).map(|v| v.detail.as_deref().unwrap()).collect();
    assert_eq!(details, ["exit 0", "exit 2", "exit 1"]);
    assert_eq!(count("pi").error_count, 2, "isError result + stopReason error, not aborted or words");
    assert_eq!(count("dsh").error_count, 1);
    assert_eq!(rows(&idx, SessionFilter { errors_only: true, ..Default::default() }).len(), 4);
}

#[test]
fn now_detail_and_last_event_at() {
    let e = env();
    write(
        &e.home.join(".codex/sessions/2026/10/01/rollout-n.jsonl"),
        concat!(
            r#"{"timestamp":"2026-10-01T00:00:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"list files"}]}}"#, "\n",
            r#"{"timestamp":"2026-10-01T00:00:05Z","type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"git status --short\",\"workdir\":\"/w\"}"}}"#, "\n",
            r#"{"timestamp":"2026-10-01T00:00:09Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1,"output_tokens":1}}}}"#, "\n",
        ),
    );
    write(
        &e.home.join(".claude/projects/-w/m.jsonl"),
        concat!(
            r#"{"type":"user","timestamp":"2026-10-01T00:00:00Z","message":{"role":"user","content":"look around"}}"#, "\n",
            r#"{"type":"assistant","timestamp":"2026-10-01T00:00:02Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Grep","input":{"pattern":"fn main","path":"/w"}}]}}"#, "\n",
            r#"{"type":"assistant","timestamp":"2026-10-01T00:00:03Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"mcp__github__create_issue","input":{"title":"t"}}]}}"#, "\n",
            r#"{"type":"assistant","timestamp":"2026-10-01T00:00:04Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Agent","input":{"description":"Go map it","prompt":"long"}}]}}"#, "\n",
            r#"{"type":"assistant","timestamp":"2026-10-01T00:00:06Z","message":{"role":"assistant","content":[{"type":"text","text":"x"}]}}"#, "\n",
            r#"{"type":"attachment","timestamp":"2026-10-01T00:09:00Z","attachment":{"type":"hook_success"}}"#, "\n",
        ),
    );
    let idx = e.scan();
    let codex = rows(&idx, SessionFilter { source: Some("codex".into()), ..Default::default() }).remove(0);
    assert_eq!(codex.now.as_deref(), Some("exec_command: git status --short"));
    let t = |s: &str| chrono_ms(s);
    assert_eq!(codex.last_event_at, Some(t("2026-10-01T00:00:05Z")), "telemetry does not count");

    let claude = rows(&idx, SessionFilter { source: Some("claude".into()), ..Default::default() }).remove(0);
    assert_eq!(claude.now.as_deref(), Some("x"), "last assistant text wins over the earlier tool calls");
    assert_eq!(claude.last_event_at, Some(t("2026-10-01T00:00:06Z")), "a trailing attachment is not an event");
    let ev = idx.list_events(&claude.id, &EventFilter::default()).unwrap();
    let d: Vec<_> = ev.iter().filter_map(|v| v.detail.clone()).collect();
    assert_eq!(d, ["fn main", "github.create_issue", "Go map it"]);
    assert!(ev.iter().all(|v| v.detail.as_deref().is_none_or(|s| s.chars().count() <= 101)));
}

fn chrono_ms(s: &str) -> i64 {
    // 2026-10-01T00:00:SSZ -> ms, without pulling chrono into the test crate.
    let day = 1_790_812_800_000_i64; // 2026-10-01T00:00:00Z
    assert!(s.starts_with("2026-10-01T00:00:"));
    day + s[17..19].parse::<i64>().unwrap() * 1000
}

#[test]
fn a_trailing_error_is_failed_until_anything_follows_it() {
    let e = env();
    let user = r#"{"type":"user","timestamp":"2026-10-01T00:00:00Z","message":{"role":"user","content":"go"}}"#;
    let tool = r#"{"type":"assistant","timestamp":"2026-10-01T00:00:01Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash","input":{"command":"x"}}]}}"#;
    let bad = r#"{"type":"user","timestamp":"2026-10-01T00:00:02Z","message":{"role":"user","content":[{"type":"tool_result","is_error":true,"content":"nope"}]}}"#;
    let done = r#"{"type":"assistant","timestamp":"2026-10-01T00:00:03Z","message":{"role":"assistant","content":[{"type":"text","text":"recovered"}]}}"#;
    write(&e.home.join(".claude/projects/-w/f1.jsonl"), &format!("{user}\n{tool}\n{bad}\n"));
    write(&e.home.join(".claude/projects/-w/f2.jsonl"), &format!("{user}\n{tool}\n{bad}\n{done}\n"));
    let mut idx = e.scan();
    let failed = |idx: &TraceIndex, id: &str| idx.session(id).unwrap().unwrap().failed;
    let f1 = rows(&idx, SessionFilter::default()).into_iter().find(|s| s.event_count == 3).unwrap();
    let f2 = rows(&idx, SessionFilter::default()).into_iter().find(|s| s.event_count == 4).unwrap();
    assert!(failed(&idx, &f1.id));
    assert!(!failed(&idx, &f2.id), "the agent carried on");
    // Appending a later turn clears it incrementally.
    let mut f = fs::OpenOptions::new().append(true).open(e.home.join(".claude/projects/-w/f1.jsonl")).unwrap();
    std::io::Write::write_all(&mut f, format!("{done}\n").as_bytes()).unwrap();
    drop(f);
    idx.scan(&e.roots()).unwrap();
    assert!(!failed(&idx, &f1.id));
}

#[test]
fn hide_trivial_drops_pings_but_not_tool_sessions() {
    let e = env();
    write(
        &e.home.join(".pi/agent/sessions/--w--/ping.jsonl"),
        concat!(
            r#"{"type":"session","id":"x","cwd":"/w"}"#, "\n",
            r#"{"type":"message","message":{"role":"user","content":"reply with exactly: ok"}}"#, "\n",
            r#"{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"ok"}]}}"#, "\n",
        ),
    );
    write(
        &e.home.join(".pi/agent/sessions/--w--/work.jsonl"),
        concat!(
            r#"{"type":"message","message":{"role":"user","content":"do a thing"}}"#, "\n",
            r#"{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","name":"bash","arguments":{"command":"ls"}}]}}"#, "\n",
        ),
    );
    write(
        &e.home.join(".pi/agent/sessions/--w--/talk.jsonl"),
        &(0..3).map(|i| format!(r#"{{"type":"message","message":{{"role":"user","content":"msg {i}"}}}}"#)).collect::<Vec<_>>().join("\n"),
    );
    fs::write(e.home.join(".pi/agent/sessions/--w--/talk.jsonl"), fs::read_to_string(e.home.join(".pi/agent/sessions/--w--/talk.jsonl")).unwrap() + "\n").unwrap();
    let idx = e.scan();
    let all = rows(&idx, SessionFilter::default());
    assert_eq!(all.len(), 3);
    let shown = rows(&idx, SessionFilter { hide_trivial: true, ..Default::default() });
    let mut titles: Vec<_> = shown.iter().map(|s| s.title.as_str()).collect();
    titles.sort();
    assert_eq!(titles, ["do a thing", "msg 0"], "the ping (2 non-noise events, no tools) is hidden");
}

#[test]
fn dispatch_parents_for_codex_omp_dsh_and_grok() {
    let e = env();
    let meta = |id: &str, extra: &str| format!(r#"{{"timestamp":"2026-10-01T00:00:00Z","type":"session_meta","payload":{{"id":"{id}","cwd":"/w"{extra}}}}}"#);
    let user = r#"{"timestamp":"2026-10-01T00:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"CODEX TASK"}]}}"#;
    let reply = r#"{"timestamp":"2026-10-01T00:00:02Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}}"#;
    let tool = r#"{"timestamp":"2026-10-01T00:00:03Z","type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"ls\"}"}}"#;
    write(&e.home.join(".codex/sessions/2026/10/01/rollout-parent.jsonl"), &format!("{}\n{user}\n{reply}\n{tool}\n", meta("cx-parent", "")));
    write(
        &e.home.join(".codex/sessions/2026/10/01/rollout-child.jsonl"),
        &format!(
            "{}\n{user}\n{reply}\n{tool}\n",
            meta("cx-child", r#","parent_thread_id":"cx-parent","source":{"subagent":{"thread_spawn":{"parent_thread_id":"cx-parent","depth":1,"agent_path":"/root/review_it","agent_nickname":"Linnaeus","agent_role":"explorer"}}}"#)
        ),
    );

    let msg = |t: &str| format!(r#"{{"type":"message","message":{{"role":"user","content":"{t}"}}}}"#);
    let two = |t: &str| format!("{}\n{}\n{}\n", msg(t), msg("b"), msg("c"));
    write(&e.home.join(".omp/agent/sessions/-w/2026_aaa.jsonl"), &two("OMP PARENT"));
    write(&e.home.join(".omp/agent/sessions/-w/2026_aaa/Fixer.jsonl"), &two("OMP CHILD"));

    let dsh = |id: &str, extra: &str, t: &str| {
        format!(
            "{{\"type\":\"session\",\"id\":\"{id}\",\"cwd\":\"/w\"{extra}}}\n{{\"type\":\"user/message\",\"time\":1759276800000,\"data\":{{\"role\":\"user\",\"content\":\"{t}\"}}}}\n{{\"type\":\"user/message\",\"time\":1759276800001,\"data\":{{\"role\":\"user\",\"content\":\"b\"}}}}\n{{\"type\":\"user/message\",\"time\":1759276800002,\"data\":{{\"role\":\"user\",\"content\":\"c\"}}}}\n"
        )
    };
    zstd_write(&e.home.join(".dsh/sessions/w/dp/session.jsonl.zstd"), &dsh("dsh-parent", "", "DSH PARENT"));
    zstd_write(&e.home.join(".dsh/sessions/w/dc/session.jsonl.zstd"), &dsh("dsh-child", r#","parentSession":"dsh-parent","seedLength":3"#, "DSH CHILD"));

    let gdir = |id: &str| e.grok.join(format!("sessions/%2Fw/{id}"));
    let chat = |t: &str| format!("{}\n{}\n{}\n", format!(r#"{{"type":"user","content":"{t}"}}"#), r#"{"type":"assistant","content":"x"}"#, r#"{"type":"assistant","content":"y"}"#);
    write(&gdir("g-parent").join("chat_history.jsonl"), &chat("GROK PARENT"));
    write(&gdir("g-parent").join("summary.json"), r#"{"info":{"id":"g-parent","cwd":"/w"}}"#);
    write(&gdir("g-child").join("chat_history.jsonl"), &chat("GROK CHILD"));
    write(&gdir("g-child").join("summary.json"), r#"{"info":{"id":"g-child","cwd":"/w"},"session_kind":"subagent","parent_session_id":"g-parent","agent_name":"explore"}"#);
    write(&gdir("g-fork").join("chat_history.jsonl"), &chat("GROK FORK"));
    write(&gdir("g-fork").join("summary.json"), r#"{"info":{"id":"g-fork","cwd":"/w"},"session_kind":"fork","parent_session_id":"g-parent"}"#);

    let idx = e.scan();
    let pairs = [("CODEX TASK", "codex"), ("OMP PARENT", "omp"), ("DSH PARENT", "dsh"), ("GROK PARENT", "grok")];
    for (title, src) in pairs {
        let all = rows(&idx, SessionFilter { source: Some(src.into()), ..Default::default() });
        let parent = if src == "codex" { all.iter().find(|s| s.parent_id.is_none()).unwrap() } else { all.iter().find(|s| s.title == title).unwrap() };
        assert_eq!(parent.children, 1, "{src}: {all:?}");
        let kids = idx.children(&parent.id).unwrap();
        assert_eq!(kids.len(), 1, "{src}");
        assert_eq!(kids[0].parent_id.as_deref(), Some(parent.id.as_str()), "{src}");
    }
    let kid = |src: &str| rows(&idx, SessionFilter { source: Some(src.into()), ..Default::default() }).into_iter().find(|s| s.parent_id.is_some()).unwrap();
    let (c, o, d, g) = (kid("codex"), kid("omp"), kid("dsh"), kid("grok"));
    assert_eq!((c.label.as_deref(), c.agent_type.as_deref()), (Some("review_it"), Some("explorer")));
    assert_eq!((o.label.as_deref(), o.agent_type.as_deref()), (Some("Fixer"), Some("task")));
    assert_eq!(d.agent_type.as_deref(), Some("fork"));
    assert_eq!(g.agent_type.as_deref(), Some("explore"));
    assert_eq!(rows(&idx, SessionFilter { source: Some("grok".into()), ..Default::default() }).iter().filter(|s| s.parent_id.is_some()).count(), 1, "a fork is not a dispatch");
    assert_eq!(rows(&idx, SessionFilter { roots_only: true, ..Default::default() }).len(), 4 + 1, "four parents plus the fork");
}

// ---- first-scan growth and the rebuild lock ----

fn pi_files(e: &Env, n: usize, lines: usize) {
    for i in 0..n {
        let line = format!("{{\"type\":\"message\",\"message\":{{\"role\":\"user\",\"content\":\"session {i} some text to index\"}}}}\n");
        write(&e.home.join(format!(".pi/agent/sessions/--w--/b{i:03}.jsonl")), &line.repeat(lines));
    }
}

#[test]
fn hooks_before_the_first_full_scan_never_sample_or_trip() {
    let e = env();
    let limits = Limits { growth_bytes: 1024, ..Default::default() };
    let mut idx = TraceIndex::open_with_limits(&e.db, None, limits.clone()).unwrap();
    pi_files(&e, 30, 100);
    let pi: Vec<Root> = e.roots().into_iter().filter(|r| r.source == "pi").collect();
    // A hook-driven fill: receipts every few ms, one file at a time, far past the 1KB limit.
    let rc = xai_grok_traces::Receipt { source: "pi".into(), event: "tool_result".into(), ..Default::default() };
    for i in 0..30 {
        let path = e.home.join(format!(".pi/agent/sessions/--w--/b{i:03}.jsonl"));
        idx.index_file(&pi[0], &path).unwrap();
        for _ in 0..40 {
            idx.record_receipt(&rc).unwrap();
        }
    }
    assert_eq!(idx.health(), None, "no sampling before the first full scan finished");
    // The first full scan is a legitimate build: it sets the baseline instead of tripping.
    let r = idx.scan(&pi).unwrap();
    assert_eq!(r.tripped, None);
    assert_eq!(idx.health(), None);
    drop(idx);
    // A rebuilt db (fresh file) starts unbuilt again: a big first scan right after is fine too.
    for f in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{f}", e.db.display()));
    }
    let mut idx = TraceIndex::open_with_limits(&e.db, None, limits).unwrap();
    assert_eq!(idx.scan(&pi).unwrap().tripped, None);
    // After it, growth counts: more data than the limit trips.
    pi_files(&e, 60, 200);
    assert!(idx.scan(&pi).unwrap().tripped.unwrap().contains("growth rate"));
}

#[test]
fn a_limited_first_scan_is_not_built() {
    let e = env();
    pi_files(&e, 6, 50);
    let pi: Vec<Root> = e.roots().into_iter().filter(|r| r.source == "pi").collect();
    let limits = Limits { growth_bytes: 1024, ..Default::default() };
    let mut idx = TraceIndex::open_with_limits(&e.db, None, limits).unwrap();
    idx.scan_with_limit(&pi, 2).unwrap();
    pi_files(&e, 20, 200);
    // Still unbuilt, so this bigger scan is a build too and does not trip.
    assert_eq!(idx.scan(&pi).unwrap().tripped, None);
}

fn old_schema_db(db: &Path) {
    fs::create_dir_all(db.parent().unwrap()).unwrap();
    let c = rusqlite::Connection::open(db).unwrap();
    c.execute_batch(
        "CREATE TABLE trace_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         INSERT INTO trace_meta VALUES ('schema_version', '5'), ('parser_version', '4');
         CREATE TABLE trace_events (summary TEXT); INSERT INTO trace_events VALUES ('stale');",
    )
    .unwrap();
}

fn leftovers(db: &Path) -> Vec<String> {
    fs::read_dir(db.parent().unwrap())
        .unwrap()
        .map(|d| d.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".old.") || n.contains(".corrupt.") || n.ends_with(".lock"))
        .collect()
}

#[test]
fn concurrent_opens_of_an_old_schema_db_rebuild_it_once() {
    for round in 0..5 {
        let e = env();
        old_schema_db(&e.db);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let (db, b) = (e.db.clone(), barrier.clone());
                std::thread::spawn(move || {
                    b.wait();
                    // Half hooks (short busy timeout), half pagers.
                    let busy = (i % 2 == 0).then_some(Duration::from_millis(250));
                    TraceIndex::open_with(&db, busy).map(|idx| idx.stats().unwrap().schema_version)
                })
            })
            .collect();
        for h in handles {
            let schema = h.join().unwrap().unwrap_or_else(|err| panic!("round {round}: {err}"));
            assert_eq!(schema.as_deref(), Some("6"));
        }
        assert_eq!(leftovers(&e.db), Vec::<String>::new(), "round {round}: old db and lock cleaned up");
        // And the rebuilt db is usable and consistent.
        let mut idx = TraceIndex::open(&e.db).unwrap();
        pi_files(&e, 2, 5);
        let pi: Vec<Root> = e.roots().into_iter().filter(|r| r.source == "pi").collect();
        idx.scan(&pi).unwrap();
        assert_eq!(idx.stats().unwrap().sessions, 2);
    }
}

#[test]
fn a_held_rebuild_lock_makes_the_loser_give_up_within_the_wait() {
    let e = env();
    old_schema_db(&e.db);
    let lock = PathBuf::from(format!("{}.rebuild.lock", e.db.display()));
    fs::write(&lock, "999999").unwrap();
    let t = std::time::Instant::now();
    let err = TraceIndex::open_with(&e.db, Some(Duration::from_millis(250))).err().expect("must not block or rebuild");
    assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    assert!(err.to_string().contains("rebuilt by another process"), "{err}");
    assert!(lock.exists(), "someone else's lock is left alone");
    // The old db was not touched.
    let c = rusqlite::Connection::open(&e.db).unwrap();
    let stale: i64 = c.query_row("SELECT COUNT(*) FROM trace_events", [], |r| r.get(0)).unwrap();
    assert_eq!(stale, 1);
    drop(c);
    // A lock left by a crashed process is taken over.
    let old = SystemTime::now() - Duration::from_secs(600);
    fs::File::options().write(true).open(&lock).unwrap().set_modified(old).unwrap();
    let idx = TraceIndex::open_with(&e.db, Some(Duration::from_millis(250))).unwrap();
    assert_eq!(idx.stats().unwrap().schema_version.as_deref(), Some("6"));
    assert!(!lock.exists());
}

#[test]
fn an_index_from_a_newer_version_is_left_alone() {
    let e = env();
    old_schema_db(&e.db);
    {
        let c = rusqlite::Connection::open(&e.db).unwrap();
        c.execute("UPDATE trace_meta SET value = '99' WHERE key = 'schema_version'", []).unwrap();
    }
    let err = TraceIndex::open(&e.db).err().expect("refuses");
    assert!(err.to_string().contains("newer"), "{err}");
    let c = rusqlite::Connection::open(&e.db).unwrap();
    let v: String = c.query_row("SELECT value FROM trace_meta WHERE key = 'schema_version'", [], |r| r.get(0)).unwrap();
    assert_eq!(v, "99", "not rebuilt, so two versions cannot rebuild each other forever");
    assert_eq!(leftovers(&e.db), Vec::<String>::new());
}
