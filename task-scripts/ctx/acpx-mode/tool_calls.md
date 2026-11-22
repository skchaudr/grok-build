# Tool calls: ToolCall / ToolCallUpdate for an execute (shell) call with streaming output

Notes: simplest live output for a shell card = ToolCall{kind: Execute, status InProgress, raw_input:{command, description}} then ToolCallUpdate with status InProgress and _meta.terminal_output.data = NEW text delta (the pager accumulates deltas, tracker.rs accumulate_terminal_output) and a final ToolCallUpdate status Completed or Failed. Pi instead sends raw_output = ToolOutput::Bash JSON with output_delta; both are read. Generic (non-Execute) cards read ToolCallContent::Content Text only and render it as plain muted text (no markdown).

- tool start: ToolCall construction and send
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/tools.rs:218-240
        let content = edit_diff_content(name, args.as_ref(), None).unwrap_or_default();
        let usage = self.state.borrow_mut().tool_usage.remove(id);
        let ask_user_args = (name == "ask_user_question")
            .then(|| args.clone())
            .flatten();
        // When the tool is Q&A we still spawn even if args is None so the control
        // file gets an error payload instead of leaving the extension polling forever.
        let open_ask_user = name == "ask_user_question";
        let mut tool_call =
            acp::ToolCall::new(acp::ToolCallId::new(id.to_string()), name.to_string())
                .kind(tool_kind(name))
                .status(acp::ToolCallStatus::InProgress)
                .content(content)
                .locations(Vec::new())
                .raw_input(args);
        if let Some(usage) = usage {
            let mut meta = acp::Meta::new();
            meta.insert("piToolUsage".into(), usage);
            tool_call = tool_call.meta(Some(meta));
        }
        self.send_update(acp::SessionUpdate::ToolCall(tool_call))
            .await;
        if name == "exit_plan_mode" {
```

- handle_tool_update: streaming update with raw_output Bash JSON and output_delta
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/tools.rs:455-543
    pub(super) async fn handle_tool_update(&self, event: &Value) {
        let id = string(event, &["toolCallId", "id"]).unwrap_or("pi-tool");
        if self.suppressed_codemode_nested_call(event) {
            return;
        }
        let output = event
            .get("partialResult")
            .or_else(|| event.get("result"))
            .cloned()
            .unwrap_or(Value::Null);
        let name = string(event, &["toolName", "name"]).unwrap_or_default();
        if eval_top_level_hidden(self.eval_v2_only, name) {
            return;
        }
        let args = normalize_tool_raw_input(
            name,
            event
                .get("args")
                .or_else(|| event.get("input"))
                .cloned()
                .or_else(|| self.state.borrow().tool_args.get(id).cloned()),
        );
        if let Some(args) = args.clone() {
            self.state
                .borrow_mut()
                .tool_args
                .insert(id.to_string(), args);
        }
        // Execute streaming: stock Grok shell sends BashOutput with
        // `output_delta` so the pager appends and keeps is_running. Pi only
        // gives growing full text in partialResult; convert the growth into
        // output_delta so Run/bash cards actually breathe mid-command.
        let raw_output = if tool_kind(name) == acp::ToolKind::Execute {
            let command = args
                .as_ref()
                .and_then(|value| string(value, &["command", "cmd"]))
                .unwrap_or_default()
                .to_string();
            let description = args.as_ref().and_then(|value| {
                string(value, &["description", "task_name"])
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToOwned::to_owned)
            });
            let full_text = pi_result_text(&output);
            let full_bytes = full_text.as_bytes().to_vec();
            let prev = self
                .state
                .borrow_mut()
                .bash_stream_output
                .insert(id.to_string(), full_bytes.clone())
                .unwrap_or_default();
            let delta = if full_bytes.starts_with(&prev) {
                full_bytes[prev.len()..].to_vec()
            } else {
                // Truncation / reset: send full buffer as delta after empty
                // (tracker append path only; set_execute_output on full replace).
                full_bytes.clone()
            };
            json!({
                "type": "Bash",
                "output": full_bytes,
                "output_for_prompt": full_text,
                "exit_code": 0,
                "command": command,
                "truncated": false,
                "signal": null,
                "timed_out": false,
                "description": description,
                "current_dir": "",
                "output_file": "",
                "total_bytes": full_bytes.len(),
                "was_bare_echo": false,
                "output_delta": delta,
            })
        } else {
            normalize_tool_raw_output(name, args.as_ref(), &output, false)
        };
        let mut fields = acp::ToolCallUpdateFields::new()
            .status(Some(acp::ToolCallStatus::InProgress))
            .raw_output(Some(raw_output));
        if tool_kind(name) != acp::ToolKind::Edit {
            fields = fields.content(Some(tool_content(&output)));
        }
        self.send_update(acp::SessionUpdate::ToolCallUpdate(
            acp::ToolCallUpdate::new(acp::ToolCallId::new(id.to_string()), fields),
        ))
        .await;
    }
```

- handle_tool_end: final status, raw_output, content
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/tools.rs:545-591
    pub(super) async fn handle_tool_end(&self, event: &Value) {
        let id = string(event, &["toolCallId", "id"]).unwrap_or("pi-tool");
        if self.suppressed_codemode_nested_call(event) {
            return;
        }
        let output = event.get("result").cloned().unwrap_or(Value::Null);
        let is_error = event.get("isError").and_then(Value::as_bool) == Some(true);
        let status = if is_error {
            acp::ToolCallStatus::Failed
        } else {
            acp::ToolCallStatus::Completed
        };
        let name = string(event, &["toolName", "name"]).unwrap_or_default();
        if eval_top_level_hidden(self.eval_v2_only, name) {
            if !eval_result_without_tool_calls(&output) {
                self.state.borrow_mut().tool_args.remove(id);
                return;
            }
            // Eval-v2-only withheld the start card, but this cell never called
            // a host tool: emit the shell now so the cell renders itself
            // instead of disappearing behind its absent effects. The completed
            // update below then fills in output and status.
            let args = self.state.borrow_mut().tool_args.remove(id);
            let tool_call = acp::ToolCall::new(
                acp::ToolCallId::new(id.to_string()),
                name.to_string(),
            )
            .kind(tool_kind(name))
            .status(acp::ToolCallStatus::InProgress)
            .content(Vec::new())
            .locations(Vec::new())
            .raw_input(args);
            self.send_update(acp::SessionUpdate::ToolCall(tool_call))
                .await;
        }
        let args = normalize_tool_raw_input(
            name,
            event
                .get("args")
                .or_else(|| event.get("input"))
                .cloned()
                .or_else(|| self.state.borrow_mut().tool_args.remove(id)),
        );
        self.state.borrow_mut().bash_stream_output.remove(id);
        let raw_output = normalize_tool_raw_output(name, args.as_ref(), &output, is_error);
        let mut fields = acp::ToolCallUpdateFields::new()
            .status(Some(status))
```

- tool_kind mapping
```rust
// from crates/codegen/pi-grok-adapter/src/tool_projection.rs:316-331
pub(crate) fn tool_kind(name: &str) -> acp::ToolKind {
    let name = name.trim().to_ascii_lowercase();
    match name.as_str() {
        // Pi builtin read/bash/edit-family tools plus their common aliases.
        "read" | "read_file" | "view" => acp::ToolKind::Read,
        "bash" | "execute" | "run_terminal_command" => acp::ToolKind::Execute,
        "eval" => acp::ToolKind::Other,
        "edit" | "write" | "create_file" => acp::ToolKind::Edit,
        "grep" | "find" | "glob" => acp::ToolKind::Search,
        // ListDir is detected in the pager via `raw_input.target_directory`
        // (there is no ACP ListDir kind). Keep Other so that branch can match.
        "ls" | "list_dir" | "listdir" | "list_directory" => acp::ToolKind::Other,
        "web_fetch" | "fetch" => acp::ToolKind::Fetch,
        _ => acp::ToolKind::Other,
    }
}
```

- tool_content: text to ToolCallContent
```rust
// from crates/codegen/pi-grok-adapter/src/tool_projection.rs:20-60
pub(crate) fn tool_content(value: &Value) -> Vec<acp::ToolCallContent> {
    let source = value.get("content").unwrap_or(value);
    let mut output = Vec::new();
    match source {
        Value::Array(items) => {
            for item in items {
                let kind = string(item, &["type", "kind"])
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if kind == "image"
                    && let (Some(data), Some(mime_type)) = (
                        string(item, &["data"]),
                        string(item, &["mimeType", "mime_type"]),
                    )
                {
                    output.push(acp::ToolCallContent::from(acp::ContentBlock::Image(
                        acp::ImageContent::new(data, mime_type),
                    )));
                } else {
                    let text = string(item, &["text", "content", "message", "output"])
                        .map(ToOwned::to_owned)
                        .unwrap_or_else(|| json_text(item));
                    if !text.is_empty() {
                        output.push(acp::ToolCallContent::from(acp::ContentBlock::Text(
                            acp::TextContent::new(text),
                        )));
                    }
                }
            }
        }
        _ => {
            let text = json_text(source);
            if !text.is_empty() {
                output.push(acp::ToolCallContent::from(acp::ContentBlock::Text(
                    acp::TextContent::new(text),
                )));
            }
        }
    }
    output
}
```

- bash_tool_output: the Bash raw_output JSON
```rust
// from crates/codegen/pi-grok-adapter/src/tool_projection.rs:859-944
pub(crate) fn bash_tool_output(
    command: &str,
    description: Option<&str>,
    result: &Value,
    is_error: bool,
) -> Value {
    let text = if result.get("output").and_then(Value::as_str).is_some()
        && result.get("content").is_none()
    {
        // Direct Pi `bash` RPC response: { output, exitCode, ... }.
        result
            .get("output")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    } else {
        pi_result_text(result)
    };

    let exit_code = result
        .get("exitCode")
        .and_then(Value::as_i64)
        .or_else(|| {
            result
                .get("details")
                .and_then(|details| details.get("exitCode"))
                .and_then(Value::as_i64)
        })
        .unwrap_or(if is_error { 1 } else { 0 });

    let truncated = result
        .get("truncated")
        .and_then(Value::as_bool)
        .or_else(|| {
            result
                .pointer("/details/truncation/truncated")
                .and_then(Value::as_bool)
        })
        .unwrap_or(false);

    let output_file = result
        .get("fullOutputPath")
        .and_then(Value::as_str)
        .or_else(|| {
            result
                .pointer("/details/fullOutputPath")
                .and_then(Value::as_str)
        })
        .unwrap_or("")
        .to_string();

    // Prefer arg label; fall back to tool-result details (background bridge).
    let description = description
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            result
                .pointer("/details/description")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
        });

    let bytes = text.as_bytes().to_vec();
    let total_bytes = bytes.len();
    json!({
        "type": "Bash",
        "output": bytes,
        "output_for_prompt": text,
        "exit_code": exit_code,
        "command": command,
        "truncated": truncated,
        "signal": null,
        "timed_out": false,
        "description": description,
        "current_dir": "",
        "output_file": output_file,
        "total_bytes": total_bytes,
        "was_bare_echo": false,
    })
}

#[cfg(test)]
mod tests;
```

- BashOutput wire type (xai-grok-tools)
```rust
// from crates/codegen/xai-grok-tools/src/types/output.rs:383-414
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BashOutput {
    pub output: Vec<u8>,
    /// ANSI-stripped and soft-wrapped output for model prompt.
    /// Pre-baked at construction time so `to_prompt_format` is a simple read.
    #[serde(default)]
    pub output_for_prompt: String,
    pub exit_code: i32,
    pub command: String,
    pub truncated: bool,
    pub signal: Option<String>,
    pub timed_out: bool,
    /// describes the intent of this bash command
    pub description: Option<String>,
    /// the current working directory after the command completes
    pub current_dir: String,
    /// Path to the output file where full output is stored.
    /// Use read_file tool to retrieve full output when truncated.
    pub output_file: String,
    /// Total bytes of output (before truncation).
    pub total_bytes: usize,
    /// Incremental output delta (new bytes since last notification). When present, consumers should append to their
    /// accumulated buffer instead of replacing with `output`. When `Some(vec![])`, consumers should clear their accumulated
    /// buffer (reset signal). When `None`, the consumer should use `output` as the full buffer.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub output_delta: Option<Vec<u8>>,
    /// Set by the grok_build `run_terminal_cmd` implementation when the command was detected as a bare `echo "<msg>"` (or close variant: echo -n,
    /// echo -e, simple printf for literal output, etc.). Telemetry / statistics on this pattern for the grok_build backend. Potential doom-loop /
    /// stagnation signals (repeated trivial echoes are a common "no progress" signal). Model hints (see BareEchoHintState in the bash tool).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub was_bare_echo: bool,
}
```

- pager: generic tool card from ToolCall (Other card): title, plain text output
```rust
// from crates/codegen/xai-grok-pager/src/acp/tracker.rs:2807-2863
        _ => {
            if is_execute_tool_function_name(&tc.title) {
                let command = execute_command_from_tool_call(tc);
                let header_display = peeled_if_changed(&command, session_cwd);
                let mut block = ExecuteToolCallBlock::new(command);
                block.header_display = header_display;
                if let Some(desc) = extract_raw_field(tc, "description") {
                    block = block.with_description(desc);
                }
                return finish_execute_block(block, tc, success);
            }
            let name = tool_call_title(tc);
            let summary = if tc.title.is_empty() {
                extract_raw_field(tc, "path")
                    .or_else(|| extract_raw_field(tc, "url"))
                    .or_else(|| extract_raw_field(tc, "query"))
                    .unwrap_or_default()
            } else if tc.kind == acp::ToolKind::Other {
                String::new()
            } else {
                format!("{:?}", tc.kind).to_lowercase()
            };
            let (label, ctor): (String, fn(OtherToolCallBlock) -> ToolCallBlock) = if name
                .eq_ignore_ascii_case("skill")
                || name.to_ascii_lowercase().starts_with("skill:")
            {
                let label = name
                    .find(':')
                    .and_then(|i| name.get(i..))
                    .map(|rest| format!("Skill{rest}"))
                    .unwrap_or_else(|| "Skill".into());
                (label, ToolCallBlock::Skill)
            } else {
                (name.into_owned(), ToolCallBlock::Other)
            };
            let mut block = OtherToolCallBlock::new(label, summary);
            if let Some(input_json) = format_other_tool_input(tc) {
                block = block.with_input_json(input_json);
            }
            let mut ct = content_text(tc);
            if ct.is_empty()
                && let Some(extracted) = extract_use_tool_output(&tc.raw_output)
            {
                ct = extracted;
            }
            // Every rebuild, including an in-flight `tool_call_update`, copies the
            // current line onto the header. The first value must not stick.
            if let Some(line) = model_calls_line(&ct) {
                block.summary = line;
            }
            if !success {
                block.error = Some(if ct.is_empty() { "Failed".into() } else { ct });
            } else if !ct.is_empty() {
                block.set_output_text(ct);
            }
            RenderBlock::ToolCall(ctor(block))
        }
```

- pager: Execute card from ToolKind::Execute
```rust
// from crates/codegen/xai-grok-pager/src/acp/tracker.rs:2355-2371
        acp::ToolKind::Execute => {
            let command = execute_command_from_tool_call(tc);
            let header_display = peeled_if_changed(&command, session_cwd);
            let description = extract_raw_field(tc, "description");
            let is_bash_mode = tc
                .meta
                .as_ref()
                .and_then(|m| m.get("bash_mode"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let mut block = ExecuteToolCallBlock::new(command);
            block.bash_mode = is_bash_mode;
            block.header_display = header_display;
            if let Some(desc) = description {
                block = block.with_description(desc);
            }
            finish_execute_block(block, tc, success)
```

- pager: execute command resolution and function-name placeholders
```rust
// from crates/codegen/xai-grok-pager/src/acp/tracker.rs:2119-2161
/// True when `s` is an ACP/function tool id rather than a shell command.
/// Eager ToolCall messages often set `title` to the function name (`run_terminal_command`) before `raw_input.command` arrives.
/// Using that as the execute header flashes the internal tool name in the TUI.
fn is_execute_tool_function_name(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "run_terminal_command"
            | "run_terminal_cmd"
            | "bash"
            | "shell"
            | "execute"
            | "run_command"
            | "terminal"
    )
}
/// Eager execute-related placeholder that should not be shown to the user. Only empty execute commands count as
/// placeholders. A real shell invocation of `bash` / `shell` / etc. must not be dropped on late `is_background`
/// (that would lose demotion and stdout).
fn entry_is_execute_placeholder(entry: &crate::scrollback::entry::ScrollbackEntry) -> bool {
    match &entry.block {
        RenderBlock::ToolCall(ToolCallBlock::Execute(ex)) => ex.command.trim().is_empty(),
        RenderBlock::ToolCall(ToolCallBlock::Other(o)) => is_execute_tool_function_name(&o.name),
        _ => false,
    }
}
/// Non-empty `raw_input.command` if present (empty / whitespace treated as missing).
fn raw_input_command(tc: &acp::ToolCall) -> Option<String> {
    extract_raw_field(tc, "command").and_then(|c| {
        let t = c.trim();
        if t.is_empty() { None } else { Some(c) }
    })
}
/// Resolve the shell command for an execute tool call.
/// Prefer `raw_input.command`.
/// Do **not** fall back to a title that is only the tool function name (that produces the "Run run_terminal_command" flash).
fn execute_command_from_tool_call(tc: &acp::ToolCall) -> String {
    if let Some(cmd) = raw_input_command(tc) {
        return cmd;
    }
    if !tc.title.is_empty() && !is_execute_tool_function_name(&tc.title) {
        return tc.title.clone();
    }
    String::new()
```

- pager: execute output sources (bash raw_output, _meta.terminal_output, content text)
```rust
// from crates/codegen/xai-grok-pager/src/acp/tracker.rs:3268-3325
/// Extract BashOutput from a serde_json::Value containing ToolOutput::Bash.
fn extract_bash_output_from_value(raw: &Option<serde_json::Value>) -> Option<BashOutput> {
    let val = raw.as_ref()?;
    match serde_json::from_value::<ToolOutput>(val.clone()) {
        Ok(ToolOutput::Bash(bash)) => Some(bash),
        _ => None,
    }
}
/// `_meta.terminal_output.data` (Zed/pi-acp dialect).
fn terminal_output_data(meta: Option<&acp::Meta>) -> Option<&str> {
    meta?.get("terminal_output")?.get("data")?.as_str()
}
/// Append an update's `_meta.terminal_output` delta to the cumulative text kept in the base
/// tool call's meta, so a rebuilt block always carries the full output.
fn accumulate_terminal_output(base: &mut acp::ToolCall, update_meta: Option<&acp::Meta>) {
    let Some(delta) = terminal_output_data(update_meta) else {
        return;
    };
    let mut acc = terminal_output_data(base.meta.as_ref())
        .unwrap_or_default()
        .to_owned();
    acc.push_str(delta);
    base.meta
        .get_or_insert_with(Default::default)
        .insert("terminal_output".into(), serde_json::json!({ "data": acc }));
}
/// Execute output from xAI `ToolOutput::Bash`, then `_meta.terminal_output`, then ACP text content.
fn extract_execute_output(tc: &acp::ToolCall) -> Option<String> {
    if let Some(bash) = extract_bash_output_from_value(&tc.raw_output) {
        return Some(String::from_utf8_lossy(&bash.output).into_owned());
    }
    if let Some(data) = terminal_output_data(tc.meta.as_ref()) {
        return Some(data.to_owned());
    }
    let text = content_text(tc);
    (!text.is_empty()).then_some(text)
}
fn finish_execute_block(
    mut block: ExecuteToolCallBlock,
    tc: &acp::ToolCall,
    success: bool,
) -> RenderBlock {
    let bash = extract_bash_output_from_value(&tc.raw_output);
    let output = extract_execute_output(tc);
    if let Some(output) = &output {
        block = block.with_output(output);
    }
    let bash_failed = bash.as_ref().is_some_and(|b| b.exit_code != 0);
    if !success || bash_failed {
        let error_msg = match &bash {
            Some(b) if b.signal.is_some() => b.signal.clone().unwrap_or_default(),
            Some(b) if b.exit_code != 0 => format!("exit code {}", b.exit_code),
            // Any text output is already shown as the block's output.
            _ => "Command failed".into(),
        };
        block = block.with_error(error_msg);
    }
    RenderBlock::ToolCall(ToolCallBlock::Execute(block))
```

- pager: update merge for in-flight calls accumulates terminal_output deltas
```rust
// from crates/codegen/xai-grok-pager/src/acp/tracker.rs:1636-1646
            acp::ToolCallStatus::Completed | acp::ToolCallStatus::Failed
        );
        let tc_id = tcu.tool_call_id.0.to_string();
        if !is_completed {
            let defer_as_bg = if let Some(pending) = self.pending_tools.get_mut(&tc_id) {
                let bash_output = extract_bash_output_from_value(&tcu.fields.raw_output);
                pending.base.update(tcu.fields);
                accumulate_terminal_output(&mut pending.base, tcu.meta.as_ref());
                if let Some(entry_id) = pending.entry_id {
                    refresh_tool_trace(scrollback, entry_id, &pending.base, meta);
                }
```
