use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::acpx::bundle::{NodeKind, Outcome, Run, Step};

const REPO_ROOT_PREFIXES: &[&str] = &[
    "crates/",
    "src/",
    "task-scripts/",
    ".pi/",
    "docs/",
    "tests/",
    "tui/",
];

const KNOWN_SOURCE_EXTENSIONS: &[&str] = &[
    ".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".toml", ".json", ".md", ".sh", ".yml", ".yaml",
];

const KNOWN_SPECIAL_BASENAMES: &[&str] = &["Cargo.lock", ".gitignore"];

const REJECTION_MARKER: &str = "The workflow rejected your submission";
const REJECTION_RETRY_SUFFIX: &str = "Call the `workflow` tool again";
const PERMISSION_DENIED_MARKER: &str = "was not executed: Denied by permission policy";
const CHANNEL_UNAVAILABLE_MARKER: &str = "workflow step channel is unavailable";

/// One merged tool call from a step attempt's `sessions/main/events.ndjson` slice (`event_start_seq..=event_end_seq`).
#[derive(Debug, Clone, PartialEq)]
pub struct MergedToolCall {
    pub tool_call_id: String,
    pub title: String,
    pub kind: String,
    pub status: String,
    pub raw_input: Value,
    pub raw_output: Option<Value>,
    pub content: Option<Value>,
    pub truncated: bool,
}

/// File-level progress and scope check for one step attempt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StepFileDigest {
    pub expected_files: Vec<String>,
    pub touched_files: Vec<String>,
    pub out_of_scope_files: Vec<String>,
    pub current_file: Option<String>,
}

impl StepFileDigest {
    pub fn summary_line(&self) -> String {
        if self.expected_files.is_empty() && self.touched_files.is_empty() {
            return String::new();
        }
        let mut line = if !self.expected_files.is_empty() {
            format!(
                "files {}/{}",
                self.touched_files
                    .len()
                    .saturating_sub(self.out_of_scope_files.len()),
                self.expected_files.len()
            )
        } else {
            format!("files {}", self.touched_files.len())
        };
        if !self.out_of_scope_files.is_empty() {
            line.push_str(&format!(" (!{} out-of-scope)", self.out_of_scope_files.len()));
        }
        if let Some(curr) = &self.current_file {
            line.push_str(&format!(" · {curr}"));
        }
        line
    }
}

/// (b) Validator rejections and correction attempts for one step attempt, extracted from the step's transcript slice.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationDigest {
    pub rejection_count: usize,
    pub correction_attempts: usize,
    pub resolved: bool,
    pub last_error: Option<String>,
    pub last_rejected_output: Option<String>,
}

impl ValidationDigest {
    /// Returns:
    /// - `None` when `self.rejection_count == 0`,
    /// - `Some(format!("1 validation rejected -> corrected ({err})"))` (or `"N validations rejected -> corrected ({err})"`)
    ///   when `self.resolved` is `true` (omitting `" ({err})"` when `self.last_error` is `None`),
    /// - `Some(format!("1 validation rejected ({err})"))` (or `"N validations rejected ({err})"`)
    ///   when `self.resolved` is `false` (omitting `" ({err})"` when `self.last_error` is `None`).
    pub fn badge_label(&self) -> Option<String> {
        if self.rejection_count == 0 {
            return None;
        }
        let noun = if self.rejection_count == 1 {
            "validation"
        } else {
            "validations"
        };
        let mut label = if self.resolved {
            format!("{} {noun} rejected -> corrected", self.rejection_count)
        } else {
            format!("{} {noun} rejected", self.rejection_count)
        };
        if let Some(err) = self.last_error.as_deref().filter(|e| !e.is_empty()) {
            label.push_str(&format!(" ({err})"));
        }
        Some(label)
    }
}

/// Direct link from a step card to its session transcript slice, stdout/stderr artifacts, and live trace filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepTraceLink {
    pub run_id: String,
    pub node_id: String,
    pub attempt_id: String,
    pub session_events_path: Option<PathBuf>,
    pub event_range: Option<(u64, u64)>,
    pub stdout_path: Option<PathBuf>,
    pub stderr_path: Option<PathBuf>,
    pub output_artifact_path: Option<PathBuf>,
}

impl StepTraceLink {
    /// True when at least one transcript range or artifact path is available to open.
    pub fn is_openable(&self) -> bool {
        self.event_range.is_some()
            || self.session_events_path.is_some()
            || self.stdout_path.is_some()
            || self.stderr_path.is_some()
            || self.output_artifact_path.is_some()
    }

    /// Compact badge showing what can be opened, e.g. `"trace: events 12..48"` or `"trace: stderr"`, or `None` when `!is_openable()`.
    pub fn badge_label(&self) -> Option<String> {
        if !self.is_openable() {
            return None;
        }
        if let Some((start, end)) = self.event_range {
            Some(format!("trace: events {start}..{end}"))
        } else if self.stderr_path.is_some() {
            Some("trace: stderr".to_string())
        } else if self.stdout_path.is_some() {
            Some("trace: stdout".to_string())
        } else if self.output_artifact_path.is_some() {
            Some("trace: output".to_string())
        } else {
            Some("trace: session".to_string())
        }
    }
}

pub fn step_trace_link(run: &Run, step: &Step) -> StepTraceLink {
    let (session_events_path, event_range) = if let Some(conv) = &step.conversation {
        (
            run.session_events_path(&conv.session_id)
                .map(Path::to_path_buf),
            Some((conv.event_start_seq, conv.event_end_seq)),
        )
    } else if step.kind == NodeKind::Acp {
        (
            run.sessions.first().map(|s| s.events_path.clone()),
            None,
        )
    } else {
        (None, None)
    };

    let resolve_artifact = |art: &Option<crate::acpx::bundle::ArtifactRef>| -> Option<PathBuf> {
        art.as_ref()
            .filter(|a| !a.path.is_empty())
            .map(|a| run.summary.dir.join(&a.path))
    };

    StepTraceLink {
        run_id: run.summary.run_id.clone(),
        node_id: step.node_id.clone(),
        attempt_id: step.attempt_id.clone(),
        session_events_path,
        event_range,
        stdout_path: resolve_artifact(&step.stdout),
        stderr_path: resolve_artifact(&step.stderr),
        output_artifact_path: resolve_artifact(&step.output_artifact),
    }
}

pub fn read_attempt_tool_calls(run: &Run, step: &Step) -> Vec<MergedToolCall> {
    let link = step_trace_link(run, step);
    let Some(events_path) = link.session_events_path else {
        return Vec::new();
    };
    if link.event_range.is_none() && run.steps.len() > 1 {
        return Vec::new();
    }
    let Ok(raw) = fs::read_to_string(&events_path) else {
        return Vec::new();
    };

    let mut calls: Vec<MergedToolCall> = Vec::new();
    let mut index_by_id: HashMap<String, usize> = HashMap::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(val) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if let Some((start, end)) = link.event_range {
            let Some(seq) = val.get("seq").and_then(Value::as_u64) else {
                continue;
            };
            if seq < start || seq > end {
                continue;
            }
        }
        if val.get("direction").and_then(Value::as_str) != Some("inbound") {
            continue;
        }
        if val.pointer("/message/method").and_then(Value::as_str) != Some("session/update") {
            continue;
        }
        let Some(update) = val.pointer("/message/params/update") else {
            continue;
        };
        let session_update = update
            .get("sessionUpdate")
            .and_then(Value::as_str)
            .unwrap_or("");
        if session_update != "tool_call" && session_update != "tool_call_update" {
            continue;
        }

        let tool_call_id = update
            .get("toolCallId")
            .or_else(|| update.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("");

        let entry_idx = if !tool_call_id.is_empty() {
            if let Some(&idx) = index_by_id.get(tool_call_id) {
                idx
            } else {
                let idx = calls.len();
                calls.push(MergedToolCall {
                    tool_call_id: tool_call_id.to_string(),
                    title: String::new(),
                    kind: String::new(),
                    status: "pending".to_string(),
                    raw_input: Value::Null,
                    raw_output: None,
                    content: None,
                    truncated: false,
                });
                index_by_id.insert(tool_call_id.to_string(), idx);
                idx
            }
        } else {
            let idx = calls.len();
            calls.push(MergedToolCall {
                tool_call_id: String::new(),
                title: String::new(),
                kind: String::new(),
                status: "pending".to_string(),
                raw_input: Value::Null,
                raw_output: None,
                content: None,
                truncated: false,
            });
            idx
        };

        let Some(entry) = calls.get_mut(entry_idx) else {
            continue;
        };

        if let Some(title) = update
            .get("title")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                update
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
            })
        {
            entry.title = title.to_string();
        }
        if let Some(kind) = update
            .get("kind")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            entry.kind = kind.to_string();
        }
        if let Some(status) = update
            .get("status")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            entry.status = status.to_string();
        }
        if let Some(raw_input) = update
            .get("rawInput")
            .or_else(|| update.get("input"))
            .or_else(|| update.get("args"))
        {
            if is_non_empty_json(raw_input) || entry.raw_input.is_null() {
                entry.raw_input = raw_input.clone();
            }
        }
        if let Some(raw_output) = update
            .get("rawOutput")
            .or_else(|| update.get("output"))
            .or_else(|| update.get("result"))
            .filter(|v| !v.is_null())
        {
            entry.raw_output = Some(raw_output.clone());
        }
        if let Some(content) = update.get("content").filter(|v| !v.is_null()) {
            entry.content = Some(content.clone());
        }
        entry.truncated |= update
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    }

    calls
}

pub fn digest_step_files(run: &Run, step: &Step, observed_tools: &[&str]) -> StepFileDigest {
    let expected_files = extract_expected_files(step.prompt_text.as_deref().unwrap_or(""));
    let mut touched_files: Vec<String> = Vec::new();

    for call in read_attempt_tool_calls(run, step) {
        if is_read_only_tool(&call) {
            continue;
        }
        if let Some(path) = extract_edit_or_write_path(&call) {
            let normalized = normalize_repo_path(&path);
            if !normalized.is_empty() {
                push_unique(&mut touched_files, normalized);
            }
        }
        if let Some(command) = extract_execute_command(&call) {
            for path in extract_git_command_paths(command) {
                let normalized = normalize_repo_path(&path);
                if !normalized.is_empty() {
                    push_unique(&mut touched_files, normalized);
                }
            }
        }
    }

    for tool_str in observed_tools {
        for path in extract_observed_tool_paths(tool_str) {
            let normalized = normalize_repo_path(&path);
            if !normalized.is_empty() {
                push_unique(&mut touched_files, normalized);
            }
        }
    }

    let out_of_scope_files = if expected_files.is_empty() {
        Vec::new()
    } else {
        touched_files
            .iter()
            .filter(|path| !expected_files.contains(path))
            .cloned()
            .collect()
    };
    let current_file = touched_files.last().cloned();

    StepFileDigest {
        expected_files,
        touched_files,
        out_of_scope_files,
        current_file,
    }
}

pub fn extract_step_validations_with_outcome(
    run: &Run,
    step: &Step,
    observed_tools: &[&str],
    effective_outcome: Outcome,
) -> ValidationDigest {
    let calls = read_attempt_tool_calls(run, step);
    let mut submit_calls: Vec<(&MergedToolCall, String)> = Vec::new();

    for call in &calls {
        let output_text = extract_tool_output_text(call);
        if !is_workflow_submit_call(call, &output_text) {
            continue;
        }
        if output_text.contains(PERMISSION_DENIED_MARKER)
            || output_text.contains(CHANNEL_UNAVAILABLE_MARKER)
        {
            continue;
        }
        submit_calls.push((call, output_text));
    }

    let total_submit_calls = submit_calls.len();
    let mut rejection_count: usize = 0;
    let mut last_error: Option<String> = None;
    let mut last_rejected_output: Option<String> = None;
    let mut first_rejection_idx: Option<usize> = None;
    let mut has_accepted_submit_after_last_rejection = false;

    for (idx, (call, output_text)) in submit_calls.iter().enumerate() {
        let inner_input = call.raw_input.get("tool_input").unwrap_or(&call.raw_input);
        let output_preview = inner_input
            .get("output")
            .and_then(|v| serde_json::to_string(v).ok())
            .filter(|s| !s.is_empty())
            .map(|s| truncate_chars(&s, 120));

        let is_rejected = output_text.contains(REJECTION_MARKER)
            || call.status == "failed"
            || idx + 1 < total_submit_calls;

        if is_rejected {
            rejection_count += 1;
            if first_rejection_idx.is_none() {
                first_rejection_idx = Some(idx);
            }
            has_accepted_submit_after_last_rejection = false;
            if let Some(reason) = clean_rejection_reason(output_text) {
                last_error = Some(reason);
            }
            if let Some(preview) = output_preview {
                last_rejected_output = Some(preview);
            }
        } else {
            let is_accepted = call.status == "completed"
                || output_text.contains("Submission accepted")
                || output_text.contains("Workflow step output accepted")
                || (idx + 1 == total_submit_calls
                    && effective_outcome == Outcome::Ok
                    && step.finished_ms > step.started_ms);
            if is_accepted && rejection_count > 0 {
                has_accepted_submit_after_last_rejection = true;
            }
        }
    }

    for tool_str in observed_tools {
        if tool_str.contains(REJECTION_MARKER) {
            rejection_count += 1;
            has_accepted_submit_after_last_rejection = false;
            if let Some(reason) = clean_rejection_reason(tool_str) {
                last_error = Some(reason);
            }
        }
    }

    if rejection_count == 0
        && run.summary.waiting_on.as_deref() == Some(step.node_id.as_str())
        && step.kind != NodeKind::Checkpoint
    {
        if let Some(detail) = run.summary.status_detail.as_deref() {
            if detail.contains(REJECTION_MARKER) {
                rejection_count += 1;
                if let Some(reason) = clean_rejection_reason(detail) {
                    last_error = Some(reason);
                }
            }
        }
    }

    let total_submit_calls_after_first_rejection = first_rejection_idx
        .map(|first| total_submit_calls.saturating_sub(first + 1))
        .unwrap_or(0);

    let resolved = rejection_count > 0
        && (has_accepted_submit_after_last_rejection
            || (effective_outcome == Outcome::Ok
                && step.finished_ms > step.started_ms
                && run.summary.current_node.as_deref() != Some(step.node_id.as_str())
                && run.summary.waiting_on.as_deref() != Some(step.node_id.as_str())));

    let correction_attempts = if resolved {
        rejection_count
    } else {
        total_submit_calls_after_first_rejection
    };

    ValidationDigest {
        rejection_count,
        correction_attempts,
        resolved,
        last_error,
        last_rejected_output,
    }
}

fn is_non_empty_json(val: &Value) -> bool {
    match val {
        Value::Null => false,
        Value::Object(map) => !map.is_empty(),
        Value::Array(arr) => !arr.is_empty(),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn push_unique(vec: &mut Vec<String>, item: String) {
    if !vec.contains(&item) {
        vec.push(item);
    }
}

fn normalize_repo_path(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches(|c| matches!(c, '`' | '\'' | '"'));
    let without_scheme = trimmed.strip_prefix("file://").unwrap_or(trimmed);
    let without_dot = without_scheme
        .strip_prefix("./")
        .unwrap_or(without_scheme);

    if REPO_ROOT_PREFIXES
        .iter()
        .any(|prefix| without_dot.starts_with(prefix))
    {
        return without_dot.to_string();
    }

    let mut earliest: Option<usize> = None;
    for prefix in REPO_ROOT_PREFIXES {
        let needle = format!("/{prefix}");
        if let Some(pos) = without_dot.find(&needle) {
            let start = pos + 1;
            earliest = Some(match earliest {
                Some(prev) => prev.min(start),
                None => start,
            });
        }
    }

    if let Some(start) = earliest {
        if let Some(sliced) = without_dot.get(start..) {
            return sliced.to_string();
        }
    }

    without_dot.to_string()
}

fn extract_expected_files(prompt: &str) -> Vec<String> {
    let mut files = Vec::new();
    for raw_token in prompt.split_whitespace() {
        let candidate = if let Some(link_start) = raw_token.find("](") {
            raw_token.get(link_start + 2..).unwrap_or(raw_token)
        } else {
            raw_token
        };

        let cleaned = candidate.trim_matches(|c: char| {
            matches!(
                c,
                '`' | '\''
                    | '"'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '<'
                    | '>'
                    | ','
                    | ';'
                    | ':'
                    | '!'
                    | '?'
                    | '*'
            )
        });
        let cleaned = cleaned.trim_end_matches('.');
        if cleaned.is_empty()
            || cleaned.starts_with("http://")
            || cleaned.starts_with("https://")
            || cleaned.starts_with("ftp://")
        {
            continue;
        }

        let without_fragment = cleaned.split('#').next().unwrap_or(cleaned);
        let without_line = strip_line_col_suffix(without_fragment);
        if !without_line.contains('/') || without_line.starts_with("//") {
            continue;
        }
        if !has_known_source_extension(without_line) {
            continue;
        }

        let normalized = normalize_repo_path(without_line);
        if !normalized.is_empty() {
            push_unique(&mut files, normalized);
        }
    }
    files
}

fn strip_line_col_suffix(path: &str) -> &str {
    let mut current = path;
    while let Some((head, tail)) = current.rsplit_once(':') {
        if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) {
            current = head;
        } else {
            break;
        }
    }
    current
}

fn has_known_source_extension(path: &str) -> bool {
    let basename = path.rsplit('/').next().unwrap_or(path);
    if KNOWN_SPECIAL_BASENAMES.contains(&basename) {
        return true;
    }
    let lower = basename.to_ascii_lowercase();
    KNOWN_SOURCE_EXTENSIONS
        .iter()
        .any(|ext| lower.ends_with(ext) && lower.len() > ext.len())
}

fn is_read_only_tool(call: &MergedToolCall) -> bool {
    let title = call.title.trim().to_ascii_lowercase();
    let kind = call.kind.trim().to_ascii_lowercase();
    let tool_name = call
        .raw_input
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    let is_ro_name = |name: &str| {
        matches!(
            name,
            "read"
                | "grep"
                | "find"
                | "ls"
                | "search_tool"
                | "toolsearch"
                | "workflow"
                | "workflow__workflow"
                | "mcp__workflow__workflow"
        )
    };

    is_ro_name(&title) || is_ro_name(&kind) || is_ro_name(&tool_name)
}

fn extract_edit_or_write_path(call: &MergedToolCall) -> Option<String> {
    let inner = call.raw_input.get("tool_input").unwrap_or(&call.raw_input);
    let title_lower = call.title.to_ascii_lowercase();
    let kind_lower = call.kind.to_ascii_lowercase();

    let is_edit_name = kind_lower.contains("edit")
        || kind_lower.contains("write")
        || kind_lower.contains("multiedit")
        || kind_lower.contains("searchreplace")
        || title_lower.contains("edit")
        || title_lower.contains("write")
        || title_lower.contains("multiedit")
        || title_lower.contains("searchreplace");

    let is_edit_variant = inner
        .get("variant")
        .or_else(|| call.raw_input.get("variant"))
        .and_then(Value::as_str)
        .is_some_and(|v| v == "SearchReplace" || v == "Write");

    let path_str = inner
        .get("file_path")
        .or_else(|| inner.get("path"))
        .or_else(|| call.raw_input.get("file_path"))
        .or_else(|| call.raw_input.get("path"))
        .and_then(Value::as_str)?;

    let has_edit_payload = inner.get("new_string").is_some()
        || inner.get("oldText").is_some()
        || inner.get("edits").is_some()
        || inner.get("content").is_some()
        || call.raw_input.get("new_string").is_some()
        || call.raw_input.get("oldText").is_some()
        || call.raw_input.get("edits").is_some()
        || call.raw_input.get("content").is_some();

    if is_edit_name || is_edit_variant || has_edit_payload {
        let trimmed = path_str.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    None
}

fn extract_execute_command(call: &MergedToolCall) -> Option<&str> {
    let inner = call.raw_input.get("tool_input").unwrap_or(&call.raw_input);
    let title_lower = call.title.to_ascii_lowercase();
    let kind_lower = call.kind.to_ascii_lowercase();
    let command = inner
        .get("command")
        .or_else(|| call.raw_input.get("command"))
        .and_then(Value::as_str);

    if command.is_some()
        || kind_lower.contains("execute")
        || kind_lower.contains("bash")
        || title_lower.contains("execute")
        || title_lower.contains("bash")
    {
        command.filter(|s| !s.trim().is_empty())
    } else {
        None
    }
}

fn extract_git_command_paths(command: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for segment in split_shell_segments(command) {
        let tokens = tokenize_shell_segment(&segment);
        let Some(git_idx) = tokens.iter().position(|t| t == "git") else {
            continue;
        };

        let mut idx = git_idx + 1;
        while let Some(tok) = tokens.get(idx) {
            if matches!(
                tok.as_str(),
                "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" | "--super-prefix"
            ) {
                idx += 2;
                continue;
            }
            if tok.starts_with('-') {
                idx += 1;
                continue;
            }
            break;
        }

        let Some(subcmd) = tokens.get(idx).map(String::as_str) else {
            continue;
        };
        if subcmd != "add" && subcmd != "rm" {
            continue;
        }
        idx += 1;

        let mut after_double_dash = false;
        while let Some(tok) = tokens.get(idx) {
            idx += 1;
            if !after_double_dash && tok == "--" {
                after_double_dash = true;
                continue;
            }
            if !after_double_dash && tok.starts_with('-') {
                continue;
            }
            if tok.starts_with('>')
                || tok.starts_with('<')
                || tok.starts_with("1>")
                || tok.starts_with("2>")
            {
                break;
            }
            if tok.is_empty() || matches!(tok.as_str(), "." | ".." | "*") {
                continue;
            }
            push_unique(&mut paths, tok.clone());
        }
    }
    paths
}

fn split_shell_segments(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let mut chars = command.chars().peekable();

    while let Some(ch) = chars.next() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' && !in_single {
            escaped = true;
            current.push(ch);
            continue;
        }
        if ch == '\'' && !in_double {
            in_single = !in_single;
            current.push(ch);
            continue;
        }
        if ch == '"' && !in_single {
            in_double = !in_double;
            current.push(ch);
            continue;
        }
        if !in_single && !in_double {
            if matches!(ch, ';' | '\n') {
                if !current.trim().is_empty() {
                    segments.push(std::mem::take(&mut current));
                }
                continue;
            }
            if ch == '&' && chars.peek() == Some(&'&') {
                chars.next();
                if !current.trim().is_empty() {
                    segments.push(std::mem::take(&mut current));
                }
                continue;
            }
            if ch == '|' {
                if chars.peek() == Some(&'|') {
                    chars.next();
                }
                if !current.trim().is_empty() {
                    segments.push(std::mem::take(&mut current));
                }
                continue;
            }
        }
        current.push(ch);
    }

    if !current.trim().is_empty() {
        segments.push(current);
    }
    segments
}

fn tokenize_shell_segment(segment: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for ch in segment.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' && !in_single {
            escaped = true;
            continue;
        }
        if ch == '\'' && !in_double {
            in_single = !in_single;
            continue;
        }
        if ch == '"' && !in_single {
            in_double = !in_double;
            continue;
        }
        if !in_single && !in_double && ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(ch);
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn extract_observed_tool_paths(entry: &str) -> Vec<String> {
    let trimmed = entry.trim();
    let mut paths = Vec::new();
    let lower = trimmed.to_ascii_lowercase();

    for prefix in ["edit ", "write ", "multiedit ", "edit:", "write:"] {
        if lower.starts_with(prefix) {
            if let Some(rest) = trimmed.get(prefix.len()..) {
                let first = rest.split_whitespace().next().unwrap_or("");
                let cleaned = first.trim_matches(|c| matches!(c, '`' | '\'' | '"' | ',' | ';'));
                if !cleaned.is_empty() {
                    push_unique(&mut paths, cleaned.to_string());
                }
            }
        }
    }

    if lower.contains("git add") || lower.contains("git rm") {
        for path in extract_git_command_paths(trimmed) {
            push_unique(&mut paths, path);
        }
    }

    paths
}

fn is_workflow_tool_name(name: &str) -> bool {
    matches!(
        name,
        "workflow" | "workflow__workflow" | "mcp__workflow__workflow"
    )
}

fn is_workflow_submit_call(call: &MergedToolCall, output_text: &str) -> bool {
    let inner = call.raw_input.get("tool_input").unwrap_or(&call.raw_input);

    let has_submit_with_request_id = (inner.get("action").and_then(Value::as_str) == Some("submit")
        && inner.get("requestId").is_some())
        || (call.raw_input.get("action").and_then(Value::as_str) == Some("submit")
            && call.raw_input.get("requestId").is_some());

    let titled_submit = call.title == "submit_workflow_output"
        || call.title.contains("submit_workflow")
        || call.kind == "submit_workflow_output";

    let is_workflow_tool = is_workflow_tool_name(call.title.as_str())
        || is_workflow_tool_name(call.kind.as_str())
        || call
            .raw_input
            .get("tool_name")
            .and_then(Value::as_str)
            .is_some_and(is_workflow_tool_name)
        || has_submit_with_request_id
        || titled_submit;

    if !is_workflow_tool {
        return false;
    }
    if titled_submit {
        return true;
    }

    match inner.get("action").and_then(Value::as_str) {
        Some("submit") => true,
        Some(_) => false,
        None => call.status == "failed" && output_text.contains(REJECTION_MARKER),
    }
}

fn extract_tool_output_text(call: &MergedToolCall) -> String {
    if let Some(raw_output) = &call.raw_output {
        let text = value_to_output_text(raw_output);
        if !text.is_empty() {
            return text;
        }
    }
    if let Some(content) = &call.content {
        return value_to_output_text(content);
    }
    String::new()
}

fn value_to_output_text(val: &Value) -> String {
    match val {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Object(_) => {
            if let Some(text) = val.pointer("/content/0/text").and_then(Value::as_str) {
                return text.to_string();
            }
            if let Some(text) = val
                .pointer("/result/content/0/text")
                .and_then(Value::as_str)
            {
                return text.to_string();
            }
            if let Some(text) = val.get("error").and_then(Value::as_str) {
                return text.to_string();
            }
            if let Some(text) = val.get("text").and_then(Value::as_str) {
                return text.to_string();
            }
            serde_json::to_string(val).unwrap_or_default()
        }
        Value::Array(arr) => {
            if let Some(text) = arr
                .first()
                .and_then(|item| item.get("text"))
                .and_then(Value::as_str)
            {
                return text.to_string();
            }
            serde_json::to_string(val).unwrap_or_default()
        }
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn clean_rejection_reason(text: &str) -> Option<String> {
    let mut slice = text;
    if let Some(marker_pos) = slice.find(REJECTION_MARKER) {
        if let Some(after_marker) = slice.get(marker_pos..) {
            if let Some(colon_nl) = after_marker.find(":\n") {
                slice = after_marker.get(colon_nl + 2..).unwrap_or(after_marker);
            } else if let Some(colon) = after_marker.find(':') {
                slice = after_marker.get(colon + 1..).unwrap_or(after_marker);
            } else {
                slice = after_marker;
            }
        }
    }

    if let Some(suffix_pos) = slice.find(REJECTION_RETRY_SUFFIX) {
        if let Some(before_suffix) = slice.get(..suffix_pos) {
            slice = before_suffix;
        }
    }

    let cleaned = slice.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        s.chars().take(max_chars).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use crate::acpx::bundle::{
        ArtifactRef, Conversation, FlowDef, RunStatus, RunSummary, SessionRef,
    };

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let seq = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "xai-grok-pager-step-evidence-{}-{}-{}-{}",
                std::process::id(),
                nanos,
                seq,
                label
            ));
            fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }

        fn write_events(&self, lines: &[Value]) -> PathBuf {
            let events_dir = self.path.join("sessions/main");
            fs::create_dir_all(&events_dir).expect("create sessions/main");
            let events_path = events_dir.join("events.ndjson");
            let content = lines
                .iter()
                .map(|v| serde_json::to_string(v).expect("serialize ndjson line"))
                .collect::<Vec<_>>()
                .join("\n");
            fs::write(&events_path, content).expect("write events.ndjson");
            events_path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn make_session_update_event(seq: u64, update: Value) -> Value {
        json!({
            "seq": seq,
            "direction": "inbound",
            "message": {
                "method": "session/update",
                "params": {
                    "update": update
                }
            }
        })
    }

    fn make_step(
        node_id: &str,
        attempt_id: &str,
        outcome: Outcome,
        event_range: Option<(u64, u64)>,
    ) -> Step {
        Step {
            attempt_id: attempt_id.to_string(),
            node_id: node_id.to_string(),
            kind: NodeKind::Acp,
            outcome,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: None,
            raw_text: None,
            output: Value::Null,
            error: None,
            agent_name: Some("agent".to_string()),
            action: None,
            conversation: event_range.map(|(start, end)| Conversation {
                session_id: "main".to_string(),
                event_start_seq: start,
                event_end_seq: end,
            }),
            stdout: None,
            stderr: None,
            output_artifact: None,
        }
    }

    fn make_run(dir: PathBuf, events_path: PathBuf, steps: Vec<Step>) -> Run {
        Run {
            summary: RunSummary {
                run_id: "run-123".to_string(),
                dir,
                flow_name: "beta8-merge".to_string(),
                title: "Beta 8 merge".to_string(),
                status: RunStatus::Completed,
                started_ms: 1_000,
                updated_ms: 2_000,
                finished_ms: Some(2_000),
                current_node: None,
                waiting_on: None,
                status_detail: None,
                error: None,
            },
            flow: FlowDef::default(),
            steps,
            events: Vec::new(),
            input: Value::Null,
            sessions: vec![SessionRef {
                id: "main".to_string(),
                handle: "main".to_string(),
                events_path,
                agent_name: Some("agent".to_string()),
            }],
        }
    }

    #[test]
    fn step_trace_link_resolves_session_range_and_artifacts() {
        let tmp = TempDir::new("trace-link");
        let events_path = tmp.path.join("sessions/main/events.ndjson");
        let mut step = make_step("resolve_group2", "att-1", Outcome::Ok, Some((10, 25)));
        step.stdout = Some(ArtifactRef {
            path: "artifacts/stdout.txt".to_string(),
            media_type: "text/plain".to_string(),
            bytes: 12,
        });
        step.stderr = Some(ArtifactRef {
            path: "artifacts/stderr.txt".to_string(),
            media_type: "text/plain".to_string(),
            bytes: 4,
        });
        step.output_artifact = Some(ArtifactRef {
            path: "artifacts/output.json".to_string(),
            media_type: "application/json".to_string(),
            bytes: 32,
        });

        let run = make_run(tmp.path.clone(), events_path.clone(), vec![step.clone()]);
        let link = step_trace_link(&run, &step);

        assert!(link.is_openable());
        assert_eq!(link.run_id, "run-123");
        assert_eq!(link.node_id, "resolve_group2");
        assert_eq!(link.attempt_id, "att-1");
        assert_eq!(link.session_events_path, Some(events_path));
        assert_eq!(link.event_range, Some((10, 25)));
        assert_eq!(link.stdout_path, Some(tmp.path.join("artifacts/stdout.txt")));
        assert_eq!(link.stderr_path, Some(tmp.path.join("artifacts/stderr.txt")));
        assert_eq!(
            link.output_artifact_path,
            Some(tmp.path.join("artifacts/output.json"))
        );
        assert_eq!(link.badge_label(), Some("trace: events 10..25".to_string()));
    }

    #[test]
    fn read_attempt_tool_calls_merges_updates_and_respects_range() {
        let tmp = TempDir::new("merge-calls");
        let events_path = tmp.write_events(&[
            make_session_update_event(
                9,
                json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "tc-before",
                    "title": "edit",
                    "kind": "edit",
                    "status": "completed",
                    "rawInput": {"file_path": "crates/outside/before.rs"}
                }),
            ),
            make_session_update_event(
                10,
                json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "tc-1",
                    "title": "edit",
                    "kind": "edit",
                    "status": "pending",
                    "rawInput": {}
                }),
            ),
            make_session_update_event(
                12,
                json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "tc-1",
                    "status": "completed",
                    "rawInput": {"file_path": "crates/codegen/xai-grok-shell/src/lib.rs", "new_string": "ok"},
                    "rawOutput": {"ok": true},
                    "truncated": true
                }),
            ),
            make_session_update_event(
                26,
                json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "tc-after",
                    "title": "edit",
                    "kind": "edit",
                    "status": "completed",
                    "rawInput": {"file_path": "crates/outside/after.rs"}
                }),
            ),
        ]);

        let step = make_step("resolve_group2", "att-1", Outcome::Ok, Some((10, 25)));
        let run = make_run(tmp.path.clone(), events_path, vec![step.clone()]);
        let calls = read_attempt_tool_calls(&run, &step);

        assert_eq!(calls.len(), 1);
        let call = &calls[0];
        assert_eq!(call.tool_call_id, "tc-1");
        assert_eq!(call.title, "edit");
        assert_eq!(call.kind, "edit");
        assert_eq!(call.status, "completed");
        assert_eq!(
            call.raw_input,
            json!({"file_path": "crates/codegen/xai-grok-shell/src/lib.rs", "new_string": "ok"})
        );
        assert_eq!(call.raw_output, Some(json!({"ok": true})));
        assert!(call.truncated);
    }

    #[test]
    fn digest_step_files_tracks_in_scope_and_out_of_scope_edits() {
        let tmp = TempDir::new("file-digest");
        let events_path = tmp.write_events(&[
            make_session_update_event(
                100,
                json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "tc-read",
                    "title": "read",
                    "kind": "read",
                    "status": "completed",
                    "rawInput": {"file_path": "crates/codegen/xai-grok-shell/Cargo.toml"}
                }),
            ),
            make_session_update_event(
                101,
                json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "tc-edit-1",
                    "title": "edit",
                    "kind": "edit",
                    "status": "completed",
                    "rawInput": {
                        "file_path": "/Users/sab-mini/worktree/crates/codegen/xai-grok-shell/src/lib.rs",
                        "new_string": "pub fn ok() {}"
                    }
                }),
            ),
            make_session_update_event(
                1343,
                json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "tc-edit-2",
                    "title": "edit",
                    "kind": "edit",
                    "status": "completed",
                    "rawInput": {
                        "file_path": "/Users/sab-mini/repos/pi-workflows/_tmp_git_add_g2.sh",
                        "content": "#!/bin/sh\ngit add .\n"
                    }
                }),
            ),
        ]);

        let mut step = make_step(
            "resolve_group2_shell",
            "att-1",
            Outcome::Ok,
            Some((100, 1400)),
        );
        step.prompt_text = Some(
            "Resolve merge conflicts in `crates/codegen/xai-grok-shell/src/lib.rs` only."
                .to_string(),
        );
        let run = make_run(tmp.path.clone(), events_path, vec![step.clone()]);
        let digest = digest_step_files(&run, &step, &[]);

        assert_eq!(
            digest.expected_files,
            vec!["crates/codegen/xai-grok-shell/src/lib.rs".to_string()]
        );
        assert_eq!(
            digest.touched_files,
            vec![
                "crates/codegen/xai-grok-shell/src/lib.rs".to_string(),
                "/Users/sab-mini/repos/pi-workflows/_tmp_git_add_g2.sh".to_string(),
            ]
        );
        assert_eq!(
            digest.out_of_scope_files,
            vec!["/Users/sab-mini/repos/pi-workflows/_tmp_git_add_g2.sh".to_string()]
        );
        assert_eq!(
            digest.current_file,
            Some("/Users/sab-mini/repos/pi-workflows/_tmp_git_add_g2.sh".to_string())
        );
        assert!(digest.summary_line().contains("(!1 out-of-scope)"));
    }

    #[test]
    fn extract_step_validations_handles_resolved_and_unresolved_rejections() {
        let tmp = TempDir::new("validations-resolved");
        let events_path = tmp.write_events(&[
            make_session_update_event(
                9,
                json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "tc-bash",
                    "title": "bash",
                    "kind": "execute",
                    "status": "failed",
                    "rawInput": {"command": "cargo test"},
                    "rawOutput": "exit 1"
                }),
            ),
            make_session_update_event(
                10,
                json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "tc-wf-1",
                    "title": "workflow",
                    "kind": "workflow",
                    "status": "pending",
                    "rawInput": {}
                }),
            ),
            make_session_update_event(
                11,
                json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "tc-wf-1",
                    "status": "failed",
                    "rawInput": {
                        "action": "submit",
                        "requestId": "req-1",
                        "output": {"resolvedCount": 12}
                    },
                    "rawOutput": "The workflow rejected your submission for step \"resolve_group2_shell\":\nresolvedCount must equal 35\n\nCall the `workflow` tool again with a corrected output."
                }),
            ),
            make_session_update_event(
                11,
                json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "tc-wf-denied",
                    "title": "workflow",
                    "kind": "workflow",
                    "status": "failed",
                    "rawInput": {
                        "action": "submit",
                        "requestId": "req-1",
                        "output": {"resolvedCount": 12}
                    },
                    "rawOutput": "Tool workflow was not executed: Denied by permission policy"
                }),
            ),
            make_session_update_event(
                12,
                json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "tc-wf-2",
                    "title": "workflow",
                    "kind": "workflow",
                    "status": "pending",
                    "rawInput": {
                        "action": "submit",
                        "requestId": "req-1",
                        "output": {"resolvedCount": 35}
                    }
                }),
            ),
            make_session_update_event(
                13,
                json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "tc-wf-2",
                    "status": "completed",
                    "rawOutput": "Submission accepted. This step is complete; do not call the workflow tool again."
                }),
            ),
        ]);

        let step = make_step(
            "resolve_group2_shell",
            "att-1",
            Outcome::Ok,
            Some((9, 13)),
        );
        let run = make_run(tmp.path.clone(), events_path, vec![step.clone()]);
        let digest = extract_step_validations_with_outcome(&run, &step, &[], Outcome::Ok);

        assert_eq!(digest.rejection_count, 1);
        assert_eq!(digest.correction_attempts, 1);
        assert!(digest.resolved);
        assert_eq!(
            digest.last_error,
            Some("resolvedCount must equal 35".to_string())
        );
        assert_eq!(
            digest.last_rejected_output,
            Some("{\"resolvedCount\":12}".to_string())
        );
        assert_eq!(
            digest.badge_label(),
            Some("1 validation rejected -> corrected (resolvedCount must equal 35)".to_string())
        );

        let unresolved_step = make_step(
            "resolve_group2_shell",
            "att-1",
            Outcome::Failed,
            Some((9, 11)),
        );
        let unresolved_run = make_run(
            tmp.path.clone(),
            tmp.path.join("sessions/main/events.ndjson"),
            vec![unresolved_step.clone()],
        );
        let unresolved = extract_step_validations_with_outcome(
            &unresolved_run,
            &unresolved_step,
            &[],
            Outcome::Failed,
        );

        assert_eq!(unresolved.rejection_count, 1);
        assert_eq!(unresolved.correction_attempts, 0);
        assert!(!unresolved.resolved);
        assert_eq!(
            unresolved.last_error,
            Some("resolvedCount must equal 35".to_string())
        );
        assert_eq!(
            unresolved.last_rejected_output,
            Some("{\"resolvedCount\":12}".to_string())
        );
        assert_eq!(
            unresolved.badge_label(),
            Some("1 validation rejected (resolvedCount must equal 35)".to_string())
        );
    }
}
