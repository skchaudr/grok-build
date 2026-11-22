use std::collections::HashSet;
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use crate::acpx::bundle::RunStatus;

#[derive(Debug, Clone, PartialEq)]
pub struct LaunchSpec {
    pub flow_file: PathBuf,
    pub input_json: Option<String>,
    pub default_agent: Option<String>,
    pub approve_all: bool,
    pub cwd: Option<PathBuf>,
    pub acpx_bin: Option<String>,
}

/// The single JSON line acpx prints on stdout when the run ends or stops at a checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowRunResult {
    pub run_id: String,
    pub run_dir: PathBuf,
    pub status: RunStatus,
    pub waiting_on: Option<String>,
    pub status_detail: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LaunchState {
    Starting,
    Running { run_dir: PathBuf },
    Finished(FlowRunResult),
    Failed(String),
    Cancelled,
}

pub struct Launch {
    child: Child,
    stdout_rx: Receiver<String>,
    stderr_rx: Receiver<String>,
    stdout_lines: Vec<String>,
    stderr_lines: Vec<String>,
    runs_base: PathBuf,
    before_dirs: HashSet<PathBuf>,
    flow_file: PathBuf,
    run_dir: Option<PathBuf>,
    terminal_state: Option<LaunchState>,
}

impl Launch {
    pub fn start(spec: &LaunchSpec, runs_base: &Path) -> io::Result<Launch> {
        let resolved_flow_file = resolve_flow_file(&spec.flow_file, spec.cwd.as_deref());
        let mut resolved_spec = spec.clone();
        resolved_spec.flow_file = resolved_flow_file.clone();

        let bin = resolved_spec.acpx_bin.as_deref().unwrap_or("acpx");
        let mut cmd = Command::new(bin);
        cmd.args(argv(&resolved_spec));
        if let Some(cwd) = &resolved_spec.cwd {
            cmd.current_dir(cwd);
        }

        start_command(cmd, runs_base, resolved_flow_file)
    }

    pub fn poll(&mut self) -> LaunchState {
        if let Some(state) = &self.terminal_state {
            return state.clone();
        }

        self.drain_channels();

        if self.run_dir.is_none() {
            self.run_dir = find_new_run(&self.runs_base, &self.before_dirs, &self.flow_file);
        }

        match self.child.try_wait() {
            Ok(Some(status)) => {
                self.drain_channels_until_eof(Duration::from_millis(500));

                if self.run_dir.is_none() {
                    self.run_dir =
                        find_new_run(&self.runs_base, &self.before_dirs, &self.flow_file);
                }

                let stdout_joined = self.stdout_lines.join("\n");
                let state = if let Some(res) = parse_result(&stdout_joined) {
                    if self.run_dir.is_none() {
                        self.run_dir = Some(res.run_dir.clone());
                    }
                    LaunchState::Finished(res)
                } else {
                    let msg = self
                        .run_dir
                        .as_deref()
                        .and_then(read_live_error)
                        .unwrap_or_else(|| {
                            let last_stderr = self
                                .stderr_lines
                                .iter()
                                .rev()
                                .find(|l| !l.trim().is_empty())
                                .map(|s| s.as_str())
                                .unwrap_or("");
                            format!("acpx exited with {status}: {last_stderr}")
                        });
                    LaunchState::Failed(msg)
                };

                self.terminal_state = Some(state.clone());
                state
            }
            Ok(None) => {
                if let Some(dir) = &self.run_dir {
                    LaunchState::Running {
                        run_dir: dir.clone(),
                    }
                } else {
                    LaunchState::Starting
                }
            }
            Err(err) => {
                let state = LaunchState::Failed(format!("failed waiting on acpx: {err}"));
                self.terminal_state = Some(state.clone());
                state
            }
        }
    }

    pub fn run_dir(&self) -> Option<&Path> {
        self.run_dir.as_deref()
    }

    pub fn cancel(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.drain_channels();
        self.terminal_state = Some(LaunchState::Cancelled);
    }

    pub fn stderr_tail(&self) -> String {
        let len = self.stderr_lines.len();
        let start = len.saturating_sub(20);
        self.stderr_lines[start..].join("\n")
    }

    fn drain_channels(&mut self) {
        while let Ok(line) = self.stdout_rx.try_recv() {
            self.stdout_lines.push(line);
        }
        while let Ok(line) = self.stderr_rx.try_recv() {
            self.stderr_lines.push(line);
        }
    }

    fn drain_channels_until_eof(&mut self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let mut stdout_done = false;
        let mut stderr_done = false;

        while (!stdout_done || !stderr_done) && Instant::now() < deadline {
            self.drain_channels();

            if !stdout_done {
                let rem = deadline.saturating_duration_since(Instant::now());
                let wait = rem.min(Duration::from_millis(10));
                match self.stdout_rx.recv_timeout(wait) {
                    Ok(line) => self.stdout_lines.push(line),
                    Err(RecvTimeoutError::Disconnected) => stdout_done = true,
                    Err(RecvTimeoutError::Timeout) => {}
                }
            }

            if !stderr_done {
                let rem = deadline.saturating_duration_since(Instant::now());
                let wait = rem.min(Duration::from_millis(10));
                match self.stderr_rx.recv_timeout(wait) {
                    Ok(line) => self.stderr_lines.push(line),
                    Err(RecvTimeoutError::Disconnected) => stderr_done = true,
                    Err(RecvTimeoutError::Timeout) => {}
                }
            }
        }

        self.drain_channels();
    }
}

fn start_command(mut cmd: Command, runs_base: &Path, flow_file: PathBuf) -> io::Result<Launch> {
    let before_dirs = list_run_dirs(runs_base);

    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn()?;

    let (stdout_tx, stdout_rx) = mpsc::channel();
    if let Some(stdout) = child.stdout.take() {
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        if stdout_tx.send(l).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }

    let (stderr_tx, stderr_rx) = mpsc::channel();
    if let Some(stderr) = child.stderr.take() {
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        if stderr_tx.send(l).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }

    Ok(Launch {
        child,
        stdout_rx,
        stderr_rx,
        stdout_lines: Vec::new(),
        stderr_lines: Vec::new(),
        runs_base: runs_base.to_path_buf(),
        before_dirs,
        flow_file,
        run_dir: None,
        terminal_state: None,
    })
}

pub fn argv(spec: &LaunchSpec) -> Vec<String> {
    let mut args = Vec::new();
    if spec.approve_all {
        args.push("--approve-all".to_string());
    }
    args.push("--format".to_string());
    args.push("json".to_string());
    args.push("flow".to_string());
    args.push("run".to_string());
    args.push(spec.flow_file.to_string_lossy().into_owned());
    if let Some(input) = &spec.input_json {
        args.push("--input-json".to_string());
        args.push(input.clone());
    }
    if let Some(agent) = &spec.default_agent {
        args.push("--default-agent".to_string());
        args.push(agent.clone());
    }
    args
}

pub fn parse_result(stdout: &str) -> Option<FlowRunResult> {
    stdout
        .lines()
        .rev()
        .find_map(parse_result_line)
}

fn parse_result_line(line: &str) -> Option<FlowRunResult> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let val: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    let obj = val.as_object()?;
    if obj.get("action")?.as_str()? != "flow_run_result" {
        return None;
    }
    let run_id = obj.get("runId")?.as_str()?.to_string();
    let run_dir = PathBuf::from(obj.get("runDir")?.as_str()?);
    let status = match obj.get("status").and_then(|v| v.as_str()) {
        Some(s) => parse_run_status(s),
        None => RunStatus::Unknown,
    };
    let waiting_on = obj
        .get("waitingOn")
        .and_then(|v| v.as_str())
        .map(String::from);
    let status_detail = obj
        .get("statusDetail")
        .and_then(|v| v.as_str())
        .map(String::from);
    let error = obj.get("error").and_then(extract_error_string);

    Some(FlowRunResult {
        run_id,
        run_dir,
        status,
        waiting_on,
        status_detail,
        error,
    })
}

fn parse_run_status(s: &str) -> RunStatus {
    match s {
        "running" => RunStatus::Running,
        "waiting" => RunStatus::Waiting,
        "completed" => RunStatus::Completed,
        "failed" => RunStatus::Failed,
        "timed_out" | "timedOut" => RunStatus::TimedOut,
        _ => RunStatus::Unknown,
    }
}

fn extract_error_string(v: &serde_json::Value) -> Option<String> {
    if v.is_null() {
        return None;
    }
    if let Some(s) = v.as_str() {
        return Some(s.to_string());
    }
    if let Some(obj) = v.as_object() {
        if let Some(msg) = obj.get("message").and_then(|m| m.as_str()) {
            return Some(msg.to_string());
        }
    }
    Some(v.to_string())
}

pub fn find_new_run(
    runs_base: &Path,
    before: &HashSet<PathBuf>,
    flow_file: &Path,
) -> Option<PathBuf> {
    let entries = fs::read_dir(runs_base).ok()?;
    let mut matches: Vec<PathBuf> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || before.contains(&path) {
            continue;
        }
        let manifest_path = path.join("manifest.json");
        let Ok(bytes) = fs::read(&manifest_path) else {
            continue;
        };
        let Ok(val) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        let Some(flow_path_str) = val.get("flowPath").and_then(|v| v.as_str()) else {
            continue;
        };
        let manifest_flow_path = Path::new(flow_path_str);
        if paths_match(manifest_flow_path, flow_file) {
            matches.push(path);
        }
    }

    matches.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    matches.pop()
}

pub fn parse_run_args(args: &str) -> Result<(PathBuf, Option<String>), String> {
    let s = args.trim();
    if s.is_empty() {
        return Err("usage: /acpx run <flow file> [input json]".to_string());
    }

    let (file_str, rest) = if let Some( quote) = s.chars().next().filter(|&c| c == '"' || c == '\'') {
        let after_open = &s[quote.len_utf8()..];
        let mut file = String::new();
        let mut chars = after_open.char_indices();
        let mut end_idx = None;
        while let Some((idx, ch)) = chars.next() {
            if ch == '\\' && quote == '"' {
                if let Some((_, next_ch)) = chars.next() {
                    file.push(next_ch);
                } else {
                    file.push('\\');
                }
            } else if ch == quote {
                end_idx = Some(idx + quote.len_utf8());
                break;
            } else {
                file.push(ch);
            }
        }
        let Some(end) = end_idx else {
            return Err("unterminated quote in flow file path".to_string());
        };
        (file, after_open[end..].trim())
    } else {
        match s.find(char::is_whitespace) {
            Some(idx) => (s[..idx].to_string(), s[idx..].trim()),
            None => (s.to_string(), ""),
        }
    };

    if file_str.is_empty() {
        return Err("usage: /acpx run <flow file> [input json]".to_string());
    }

    let input_json = if rest.is_empty() {
        None
    } else {
        serde_json::from_str::<serde_json::Value>(rest)
            .map_err(|e| format!("invalid input JSON: {e}"))?;
        Some(rest.to_string())
    };

    Ok((PathBuf::from(file_str), input_json))
}

fn resolve_flow_file(flow_file: &Path, cwd: Option<&Path>) -> PathBuf {
    if flow_file.is_absolute() {
        flow_file.to_path_buf()
    } else if let Some(base) = cwd {
        base.join(flow_file)
    } else if let Ok(current) = std::env::current_dir() {
        current.join(flow_file)
    } else {
        flow_file.to_path_buf()
    }
}

fn list_run_dirs(runs_base: &Path) -> HashSet<PathBuf> {
    let Ok(entries) = fs::read_dir(runs_base) else {
        return HashSet::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect()
}

fn paths_match(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}

fn read_live_error(run_dir: &Path) -> Option<String> {
    let live_path = run_dir.join("projections").join("live.json");
    let bytes = fs::read(live_path).ok()?;
    let val: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    val.get("error").and_then(extract_error_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_spec() -> LaunchSpec {
        LaunchSpec {
            flow_file: PathBuf::from("/tmp/feel.flow.ts"),
            input_json: None,
            default_agent: None,
            approve_all: false,
            cwd: None,
            acpx_bin: None,
        }
    }

    #[test]
    fn argv_minimal() {
        let spec = base_spec();
        assert_eq!(
            argv(&spec),
            vec!["--format", "json", "flow", "run", "/tmp/feel.flow.ts"]
        );
    }

    #[test]
    fn argv_with_approve_all_before_flow() {
        let mut spec = base_spec();
        spec.approve_all = true;
        assert_eq!(
            argv(&spec),
            vec![
                "--approve-all",
                "--format",
                "json",
                "flow",
                "run",
                "/tmp/feel.flow.ts"
            ]
        );
    }

    #[test]
    fn argv_with_input_and_agent_and_approve_all() {
        let mut spec = base_spec();
        spec.approve_all = true;
        spec.input_json = Some(r#"{"x":1}"#.to_string());
        spec.default_agent = Some("claude".to_string());
        assert_eq!(
            argv(&spec),
            vec![
                "--approve-all",
                "--format",
                "json",
                "flow",
                "run",
                "/tmp/feel.flow.ts",
                "--input-json",
                r#"{"x":1}"#,
                "--default-agent",
                "claude",
            ]
        );
    }

    #[test]
    fn parse_result_sample_waiting_line() {
        let line = r#"{"action":"flow_run_result","runId":"2026-11-18T191214880Z-feel-4832e848","flowName":"feel","status":"waiting","statusDetail":"human review of the description","waitingOn":"review","runDir":"/Users/x/.acpx/flows/runs/2026-11-18T191214880Z-feel-4832e848","outputs":{},"sessionBindings":{}}"#;
        let res = parse_result(line).unwrap();
        assert_eq!(res.run_id, "2026-11-18T191214880Z-feel-4832e848");
        assert_eq!(
            res.run_dir,
            PathBuf::from("/Users/x/.acpx/flows/runs/2026-11-18T191214880Z-feel-4832e848")
        );
        assert_eq!(res.status, RunStatus::Waiting);
        assert_eq!(res.waiting_on.as_deref(), Some("review"));
        assert_eq!(
            res.status_detail.as_deref(),
            Some("human review of the description")
        );
        assert_eq!(res.error, None);
    }

    #[test]
    fn parse_result_with_noise_and_multiple_lines() {
        let out = concat!(
            "some initial log\n",
            r#"{"action":"other","runId":"ignored"}"#,
            "\n",
            r#"{"action":"flow_run_result","runId":"run-1","status":"completed","runDir":"/tmp/r1"}"#,
            "\n",
            r#"{"action":"flow_run_result","runId":"run-2","status":"completed","runDir":"/tmp/r2"}"#,
            "\ntrailing noise\n"
        );
        let res = parse_result(out).unwrap();
        assert_eq!(res.run_id, "run-2");
        assert_eq!(res.run_dir, PathBuf::from("/tmp/r2"));
        assert_eq!(res.status, RunStatus::Completed);
        assert_eq!(res.waiting_on, None);
        assert_eq!(res.status_detail, None);
        assert_eq!(res.error, None);
    }

    #[test]
    fn parse_result_without_optional_fields_and_with_error() {
        let line = r#"{"action":"flow_run_result","runId":"r3","status":"failed","runDir":"/tmp/r3","error":"boom"}"#;
        let res = parse_result(line).unwrap();
        assert_eq!(res.run_id, "r3");
        assert_eq!(res.status, RunStatus::Failed);
        assert_eq!(res.waiting_on, None);
        assert_eq!(res.status_detail, None);
        assert_eq!(res.error.as_deref(), Some("boom"));
    }

    #[test]
    fn parse_result_garbage_returns_none() {
        assert!(parse_result("").is_none());
        assert!(parse_result("not json at all\n{broken").is_none());
        assert!(parse_result(r#"{"action":"not_flow_run_result","runId":"1"}"#).is_none());
    }

    #[test]
    fn find_new_run_filters_and_picks_newest() {
        let tmp = tempfile::tempdir().unwrap();
        let runs_base = tmp.path();
        let target_flow = runs_base.join("my.flow.ts");
        let other_flow = runs_base.join("other.flow.ts");
        fs::write(&target_flow, "// target").unwrap();
        fs::write(&other_flow, "// other").unwrap();

        let write_manifest = |dir: &Path, flow: &Path| {
            fs::create_dir_all(dir).unwrap();
            let m = serde_json::json!({
                "schema": "acpx.flow-run-bundle.v1",
                "flowPath": flow.to_string_lossy()
            });
            fs::write(dir.join("manifest.json"), m.to_string()).unwrap();
        };

        // 1. Existing dir in `before`
        let old_dir = runs_base.join("2026-11-18T100000000Z-feel-old");
        write_manifest(&old_dir, &target_flow);
        let mut before = HashSet::new();
        before.insert(old_dir);

        // 2. Dir for another flow
        let other_dir = runs_base.join("2026-11-18T199999999Z-other-1");
        write_manifest(&other_dir, &other_flow);

        // 3. Dir with missing manifest & dir with half-written manifest
        let missing_manifest_dir = runs_base.join("2026-11-18T200000000Z-feel-missing");
        fs::create_dir_all(&missing_manifest_dir).unwrap();
        let broken_manifest_dir = runs_base.join("2026-11-18T200000001Z-feel-broken");
        fs::create_dir_all(&broken_manifest_dir).unwrap();
        fs::write(broken_manifest_dir.join("manifest.json"), "{not valid").unwrap();

        assert_eq!(find_new_run(runs_base, &before, &target_flow), None);

        // 4. Two matching new dirs; picks newest by name
        let match_1 = runs_base.join("2026-11-18T191214000Z-feel-1111");
        let match_2 = runs_base.join("2026-11-18T191215000Z-feel-2222");
        write_manifest(&match_1, &target_flow);
        write_manifest(&match_2, &target_flow);

        assert_eq!(
            find_new_run(runs_base, &before, &target_flow),
            Some(match_2)
        );
    }

    #[test]
    fn parse_run_args_cases() {
        assert_eq!(
            parse_run_args("my.flow.ts"),
            Ok((PathBuf::from("my.flow.ts"), None))
        );

        assert_eq!(
            parse_run_args(r#"my.flow.ts   {"a": 1, "b": [2, 3]}"#),
            Ok((
                PathBuf::from("my.flow.ts"),
                Some(r#"{"a": 1, "b": [2, 3]}"#.to_string())
            ))
        );

        assert_eq!(
            parse_run_args(r#""path with spaces/my.flow.ts" {"ok":true}"#),
            Ok((
                PathBuf::from("path with spaces/my.flow.ts"),
                Some(r#"{"ok":true}"#.to_string())
            ))
        );

        assert_eq!(
            parse_run_args("'single quoted/my.flow.ts'"),
            Ok((PathBuf::from("single quoted/my.flow.ts"), None))
        );

        assert_eq!(
            parse_run_args("   "),
            Err("usage: /acpx run <flow file> [input json]".to_string())
        );

        let bad_json = parse_run_args("my.flow.ts {not_json}");
        assert!(bad_json.is_err());
        assert!(bad_json.unwrap_err().contains("JSON"));
    }

    #[cfg(unix)]
    fn poll_until<F>(launch: &mut Launch, pred: F) -> LaunchState
    where
        F: Fn(&LaunchState) -> bool,
    {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let state = launch.poll();
            if pred(&state) || Instant::now() >= deadline {
                return state;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(unix)]
    #[test]
    fn process_prints_result_and_exits_finished() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg(
            r#"printf '%s\n' '{"action":"flow_run_result","runId":"r-ok","status":"completed","runDir":"/tmp/r-ok"}'"#,
        );

        let mut launch = start_command(cmd, tmp.path(), PathBuf::from("/tmp/f.flow.ts")).unwrap();
        let state = poll_until(&mut launch, |s| matches!(s, LaunchState::Finished(_)));
        assert_eq!(
            state,
            LaunchState::Finished(FlowRunResult {
                run_id: "r-ok".to_string(),
                run_dir: PathBuf::from("/tmp/r-ok"),
                status: RunStatus::Completed,
                waiting_on: None,
                status_detail: None,
                error: None,
            })
        );
        assert_eq!(launch.run_dir(), Some(Path::new("/tmp/r-ok")));
        // Subsequent poll returns the same terminal state
        assert_eq!(launch.poll(), state);
    }

    #[cfg(unix)]
    #[test]
    fn process_exits_nonzero_with_stderr_failed() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c")
            .arg("printf 'first line\\nfatal flow error\\n' >&2; exit 3");

        let mut launch = start_command(cmd, tmp.path(), PathBuf::from("/tmp/f.flow.ts")).unwrap();
        let state = poll_until(&mut launch, |s| matches!(s, LaunchState::Failed(_)));
        match &state {
            LaunchState::Failed(msg) => {
                assert!(
                    msg.contains("fatal flow error"),
                    "expected stderr in message, got: {msg}"
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        assert!(launch.stderr_tail().contains("fatal flow error"));
        assert_eq!(launch.poll(), state);
    }

    #[cfg(unix)]
    #[test]
    fn process_sleeps_starting_then_cancel() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg("sleep 30");

        let mut launch = start_command(cmd, tmp.path(), PathBuf::from("/tmp/f.flow.ts")).unwrap();
        assert_eq!(launch.poll(), LaunchState::Starting);

        launch.cancel();
        assert_eq!(launch.poll(), LaunchState::Cancelled);
        assert_eq!(launch.poll(), LaunchState::Cancelled);
    }
}
