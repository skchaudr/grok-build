## Edit 1: Import step_digest helpers and FlowEdge in detail.rs
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs
OLD:
~~~rust
use crate::acpx::bundle::{
    self, AgentSettings, NodeKind, NodeState, Outcome, Run, RunStatus, Step, fmt_duration,
};
use crate::render::line_utils::truncate_str;
use crate::theme::Theme;
use crate::views::traces_modal::fmt_age;
~~~
NEW:
~~~rust
use crate::acpx::bundle::{
    self, AgentSettings, FlowEdge, NodeKind, NodeState, Outcome, Run, RunStatus, Step, fmt_duration,
};
use crate::acpx::step_digest::{
    self, StepVerdict, digest_step_files, effective_outcome, extract_step_validations,
    extract_step_verdict, non_zero_exit_code, step_attempt_budget, step_failure_reason,
    step_time_budget,
};
use crate::render::line_utils::truncate_str;
use crate::theme::Theme;
use crate::views::traces_modal::fmt_age;
~~~

## Edit 2: Pass render time into step detail builder from build_lines
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs
OLD:
~~~rust
fn build_lines(
    run: &Run,
    target: &DetailTarget,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
    now_ms: i64,
) -> Vec<Line<'static>> {
    if width < 10 {
        return Vec::new();
    }
    match target {
        DetailTarget::Run => build_run_lines(run, answered, width, theme, now_ms),
        DetailTarget::Step(i) => {
            if let Some(step) = run.steps.get(*i) {
                build_step_lines(run, *i, step, answered, width, theme)
            } else {
                vec![dim_line("step not found", theme)]
            }
        }
        DetailTarget::Node(id) => build_node_lines(run, id, answered, width, theme),
    }
}
~~~
NEW:
~~~rust
fn build_lines(
    run: &Run,
    target: &DetailTarget,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
    now_ms: i64,
) -> Vec<Line<'static>> {
    if width < 10 {
        return Vec::new();
    }
    match target {
        DetailTarget::Run => build_run_lines(run, answered, width, theme, now_ms),
        DetailTarget::Step(i) => {
            if let Some(step) = run.steps.get(*i) {
                build_step_lines_at(run, *i, step, answered, width, theme, now_ms)
            } else {
                vec![dim_line("step not found", theme)]
            }
        }
        DetailTarget::Node(id) => build_node_lines(run, id, answered, width, theme),
    }
}
~~~

## Edit 3: Render compact attempt digest first in build_step_lines
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs
OLD:
~~~rust
fn build_step_lines(
    run: &Run,
    step_idx: usize,
    step: &Step,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();

    let is_last_of_node = run
        .steps
        .iter()
        .rposition(|s| s.node_id == step.node_id)
        == Some(step_idx);
    let is_gate = run.summary.status == RunStatus::Waiting
        && run.summary.waiting_on.as_deref() == Some(step.node_id.as_str())
        && is_last_of_node;

    let (glyph, glyph_color) = if is_gate && answered.is_none() {
        (NodeState::Waiting.glyph(), theme.warning)
    } else if is_gate {
        // The gate has been answered: done, like the graph and the run header.
        let state = NodeState::Done(Outcome::Ok);
        (state.glyph(), node_state_color(state, theme))
    } else {
        let state = NodeState::Done(step.outcome);
        (state.glyph(), node_state_color(state, theme))
    };

    let dur = fmt_duration(step.duration_ms());
    out.push(title_row_right(
        glyph,
        glyph_color,
        &step.node_id,
        &dur,
        width,
        theme,
    ));

    // Facts row
    let settings_map = bundle::step_settings(&run.summary.dir);
    let agent_lbl = settings_map
        .get(&step.attempt_id)
        .map(AgentSettings::label)
        .filter(|s| !s.is_empty())
        .or_else(|| step.agent_name.clone());

    let mut facts = Vec::new();
    if let Some(lbl) = agent_lbl {
        if !lbl.is_empty() {
            facts.push(lbl);
        }
    }
    facts.push(step.kind.label().to_string());
    facts.push(step.attempt_id.clone());
    if let Some(act) = &step.action {
        if let Some(code) = act.exit_code {
            facts.push(format!("exit {code}"));
        }
    }
    let facts_str = truncate_str(&facts.join(" · "), width);
    out.push(Line::from(Span::styled(
        facts_str,
        Style::default().fg(theme.gray),
    )));

    if let Some(err) = &step.error {
        if !err.is_empty() {
            push_wrapped(
                &mut out,
                err,
                width,
                Style::default().fg(theme.accent_error),
            );
        }
    }

    if is_gate {
        if let Some(detail) = &run.summary.status_detail {
            if !detail.is_empty() {
                let color = if answered.is_some() { theme.gray } else { theme.warning };
                push_wrapped(&mut out, detail, width, Style::default().fg(color));
            }
        }
        append_gate_section(&mut out, run, answered, width, theme);
    }

    // Prompt
    if let Some(prompt) = &step.prompt_text {
        if !prompt.trim().is_empty() {
            push_section_rule(&mut out, "Prompt", width, theme);
            push_wrapped_capped(
                &mut out,
                prompt,
                width,
                14,
                Style::default().fg(theme.text_secondary),
                theme,
            );
        }
    }

    // Command (for actions)
    if let Some(action) = &step.action {
        let mut cmd_parts = Vec::new();
        if let Some(cmd) = &action.command {
            if !cmd.is_empty() {
                cmd_parts.push(cmd.clone());
            }
        }
        for arg in &action.args {
            cmd_parts.push(arg.clone());
        }
        let has_cmd = !cmd_parts.is_empty();
        let has_cwd = action.cwd.as_deref().is_some_and(|c| !c.is_empty());
        if has_cmd || has_cwd {
            push_section_rule(&mut out, "Command", width, theme);
            if has_cmd {
                let cmd_line = cmd_parts.join(" ");
                push_wrapped(
                    &mut out,
                    &cmd_line,
                    width,
                    Style::default().fg(theme.command),
                );
            }
            if let Some(cwd) = &action.cwd {
                if !cwd.is_empty() {
                    push_wrapped(&mut out, cwd, width, Style::default().fg(theme.path));
                }
            }
        }
    }

    // Conversation
    if let Some(conv) = &step.conversation {
        push_section_rule(&mut out, "Conversation", width, theme);
        match run.session_events_path(&conv.session_id) {
            Some(path) => {
                append_conversation(
                    &mut out,
                    path,
                    Some((conv.event_start_seq, conv.event_end_seq)),
                    None,
                    width,
                    theme,
                );
            }
            None => {
                out.push(dim_line("session events file not found", theme));
            }
        }
    }

    // Result
    let result_lines = format_value_lines(&step.output, width, theme);
    if !result_lines.is_empty() {
        push_section_rule(&mut out, "Result", width, theme);
        out.extend(result_lines);
    }

    // stdout / stderr for actions
    if let Some(art) = &step.stdout {
        if art.bytes > 0 {
            append_artifact_section(&mut out, "stdout", &run.summary.dir, art, width, theme);
        }
    }
    if let Some(art) = &step.stderr {
        if art.bytes > 0 {
            append_artifact_section(&mut out, "stderr", &run.summary.dir, art, width, theme);
        }
    }

    // Events
    let evs: Vec<_> = run
        .step_events(&step.attempt_id)
        .into_iter()
        .filter(|e| e.kind != "node_heartbeat")
        .collect();
    if !evs.is_empty() {
        push_section_rule(&mut out, "Events", width, theme);
        let base_ms = if step.started_ms > 0 {
            step.started_ms
        } else {
            evs.first().map(|e| e.at_ms).unwrap_or(0)
        };
        for ev in evs {
            out.push(format_trace_event(ev, base_ms, width, theme));
        }
    }

    out
}
~~~
NEW:
~~~rust
fn build_step_lines(
    run: &Run,
    step_idx: usize,
    step: &Step,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    build_step_lines_at(
        run,
        step_idx,
        step,
        answered,
        width,
        theme,
        run.summary.updated_ms,
    )
}

fn build_step_lines_at(
    run: &Run,
    step_idx: usize,
    step: &Step,
    answered: Option<&str>,
    width: usize,
    theme: &Theme,
    now_ms: i64,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();

    let is_last_of_node = run
        .steps
        .iter()
        .rposition(|s| s.node_id == step.node_id)
        == Some(step_idx);
    let is_gate = run.summary.status == RunStatus::Waiting
        && run.summary.waiting_on.as_deref() == Some(step.node_id.as_str())
        && is_last_of_node;

    let outcome = effective_outcome(step);
    let (glyph, glyph_color) = if is_gate && answered.is_none() {
        (NodeState::Waiting.glyph(), theme.warning)
    } else if is_gate {
        // The gate has been answered: done, like the graph and the run header.
        let state = NodeState::Done(Outcome::Ok);
        (state.glyph(), node_state_color(state, theme))
    } else {
        let state = NodeState::Done(outcome);
        (state.glyph(), node_state_color(state, theme))
    };

    let time_budget = step_time_budget(run, step, now_ms);
    let timing = time_budget.badge_label();
    out.push(title_row_right(
        glyph,
        glyph_color,
        &step.node_id,
        &timing,
        width,
        theme,
    ));

    // Facts row
    let settings_map = bundle::step_settings(&run.summary.dir);
    let agent_lbl = settings_map
        .get(&step.attempt_id)
        .map(AgentSettings::label)
        .filter(|s| !s.is_empty())
        .or_else(|| step.agent_name.clone());

    let attempt_budget = step_attempt_budget(run, step);
    let recorded_exit_code = step
        .action
        .as_ref()
        .and_then(|act| act.exit_code)
        .or_else(|| step.output.get("exitCode").and_then(Value::as_i64));

    let mut facts = Vec::new();
    if let Some(lbl) = agent_lbl {
        if !lbl.is_empty() {
            facts.push(lbl);
        }
    }
    facts.push(step.kind.label().to_string());
    if let Some(act) = &step.action {
        if !act.action_type.is_empty() && act.action_type != step.kind.label() {
            facts.push(act.action_type.clone());
        }
    }
    facts.push(format!("attempt {}", attempt_budget.attempt));
    facts.push(step.attempt_id.clone());
    if let Some(code) = recorded_exit_code {
        facts.push(format!("exit {code}"));
    }
    let facts_str = truncate_str(&facts.join(" · "), width);
    out.push(Line::from(Span::styled(
        facts_str,
        Style::default().fg(theme.gray),
    )));

    // Bounded failure tail comes before command/cwd for failed attempts.
    if let Some(failure_tail) = failure_tail_for_step(run, step) {
        push_section_rule(&mut out, "Failure", width, theme);
        push_wrapped(
            &mut out,
            &failure_tail,
            width,
            Style::default().fg(theme.accent_error),
        );
    } else if let Some(err) = &step.error {
        if !err.is_empty() {
            push_wrapped(
                &mut out,
                err,
                width,
                Style::default().fg(theme.accent_error),
            );
        }
    }

    if is_gate {
        if let Some(detail) = &run.summary.status_detail {
            if !detail.is_empty() {
                let color = if answered.is_some() {
                    theme.gray
                } else {
                    theme.warning
                };
                push_wrapped(&mut out, detail, width, Style::default().fg(color));
            }
        }
        append_gate_section(&mut out, run, answered, width, theme);
    }

    // Compact attempt digest (verdict, validation, file scope, short note, affordances)
    let verdict = extract_step_verdict(run, step_idx);
    let (validation, file_digest) = if step.kind == NodeKind::Acp {
        (
            extract_step_validations(run, step, &[]),
            digest_step_files(run, step, &[]),
        )
    } else {
        (Default::default(), Default::default())
    };
    let validation_badge = validation.badge_label();
    let file_summary = file_digest.summary_line();
    let note = if step.kind == NodeKind::Acp && !is_gate {
        extract_step_note(step, verdict.as_ref())
    } else {
        None
    };

    let has_brief_digest = verdict.is_some()
        || validation_badge.is_some()
        || !file_summary.is_empty()
        || note.is_some();

    if has_brief_digest {
        push_section_rule(&mut out, "Digest", width, theme);
        if let Some(v) = &verdict {
            push_wrapped(
                &mut out,
                &v.summary_line(),
                width,
                Style::default().fg(theme.text_primary),
            );
        }
        if let Some(badge) = &validation_badge {
            let color = if validation.resolved {
                theme.gray
            } else {
                theme.warning
            };
            push_wrapped(&mut out, badge, width, Style::default().fg(color));
        }
        if !file_summary.is_empty() {
            let color = if file_digest.out_of_scope_files.is_empty() {
                theme.text_secondary
            } else {
                theme.warning
            };
            push_wrapped(&mut out, &file_summary, width, Style::default().fg(color));
        }
        if let Some((note_text, _)) = &note {
            push_wrapped(
                &mut out,
                note_text,
                width,
                Style::default().fg(theme.text_secondary),
            );
        }
    }

    let affordance = if step.conversation.is_some() {
        "Enter inspect · r replay"
    } else {
        "Enter inspect"
    };
    push_wrapped(&mut out, affordance, width, Style::default().fg(theme.gray_dim));

    // Command (for actions, after any failure tail)
    if let Some(action) = &step.action {
        let mut cmd_parts = Vec::new();
        if let Some(cmd) = &action.command {
            if !cmd.is_empty() {
                cmd_parts.push(cmd.clone());
            }
        }
        for arg in &action.args {
            cmd_parts.push(arg.clone());
        }
        let has_cmd = !cmd_parts.is_empty();
        let has_cwd = action.cwd.as_deref().is_some_and(|c| !c.is_empty());
        if has_cmd || has_cwd {
            push_section_rule(&mut out, "Command", width, theme);
            if has_cmd {
                let cmd_line = cmd_parts.join(" ");
                push_wrapped(
                    &mut out,
                    &cmd_line,
                    width,
                    Style::default().fg(theme.command),
                );
            }
            if let Some(cwd) = &action.cwd {
                if !cwd.is_empty() {
                    push_wrapped(&mut out, cwd, width, Style::default().fg(theme.path));
                }
            }
        }
    }

    // When a finished agent step already has brief digest evidence, keep its
    // first view free of raw prompt dumps, redundant wrapper fields, and
    // repeated transcript; full evidence stays reachable via Enter / r.
    let suppress_agent_dumps = step.kind == NodeKind::Acp && has_brief_digest;

    // Prompt
    if !suppress_agent_dumps {
        if let Some(prompt) = &step.prompt_text {
            if !prompt.trim().is_empty() {
                push_section_rule(&mut out, "Prompt", width, theme);
                push_wrapped_capped(
                    &mut out,
                    prompt,
                    width,
                    14,
                    Style::default().fg(theme.text_secondary),
                    theme,
                );
            }
        }
    }

    // Conversation
    if !suppress_agent_dumps {
        if let Some(conv) = &step.conversation {
            push_section_rule(&mut out, "Conversation", width, theme);
            match run.session_events_path(&conv.session_id) {
                Some(path) => {
                    append_conversation(
                        &mut out,
                        path,
                        Some((conv.event_start_seq, conv.event_end_seq)),
                        None,
                        width,
                        theme,
                    );
                }
                None => {
                    out.push(dim_line("session events file not found", theme));
                }
            }
        }
    }

    // Result
    let show_result = !suppress_agent_dumps
        || has_extra_result_fields(run, step, note.as_ref().is_some_and(|(_, full)| *full));
    if show_result {
        let result_lines = format_value_lines(&step.output, width, theme);
        if !result_lines.is_empty() {
            push_section_rule(&mut out, "Result", width, theme);
            out.extend(result_lines);
        }
    }

    // stdout / stderr for actions
    if let Some(art) = &step.stdout {
        if art.bytes > 0 {
            append_artifact_section(&mut out, "stdout", &run.summary.dir, art, width, theme);
        }
    }
    if let Some(art) = &step.stderr {
        if art.bytes > 0 {
            append_artifact_section(&mut out, "stderr", &run.summary.dir, art, width, theme);
        }
    }

    // Events
    let evs: Vec<_> = run
        .step_events(&step.attempt_id)
        .into_iter()
        .filter(|e| e.kind != "node_heartbeat")
        .collect();
    if !evs.is_empty() {
        push_section_rule(&mut out, "Events", width, theme);
        let base_ms = if step.started_ms > 0 {
            step.started_ms
        } else {
            evs.first().map(|e| e.at_ms).unwrap_or(0)
        };
        for ev in evs {
            out.push(format_trace_event(ev, base_ms, width, theme));
        }
    }

    out
}

fn failure_tail_for_step(run: &Run, step: &Step) -> Option<String> {
    if effective_outcome(step) == Outcome::Ok {
        return None;
    }

    const TAIL_ARTIFACT_MAX_BYTES: usize = 64 * 1024;

    let error = step
        .error
        .as_deref()
        .or_else(|| step.output.get("error").and_then(Value::as_str))
        .filter(|s| !s.trim().is_empty());

    let inline_stderr = step
        .output
        .get("stderr")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty());

    let artifact_stderr = if error.is_none() && inline_stderr.is_none() {
        step.stderr
            .as_ref()
            .filter(|a| a.bytes > 0)
            .and_then(|art| {
                bundle::read_artifact(&run.summary.dir, art, TAIL_ARTIFACT_MAX_BYTES).ok()
            })
            .filter(|s| !s.trim().is_empty())
    } else {
        None
    };

    let output_tail = step
        .output
        .get("outputTail")
        .or_else(|| step.output.get("output_tail"))
        .or_else(|| step.output.get("tail"))
        .or_else(|| step.output.get("message"))
        .and_then(Value::as_str)
        .or(step.raw_text.as_deref())
        .filter(|s| !s.trim().is_empty());

    let inline_stdout = step
        .output
        .get("stdout")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty());

    let artifact_stdout = if error.is_none()
        && inline_stderr.is_none()
        && artifact_stderr.is_none()
        && output_tail.is_none()
        && inline_stdout.is_none()
    {
        step.stdout
            .as_ref()
            .filter(|a| a.bytes > 0)
            .and_then(|art| {
                bundle::read_artifact(&run.summary.dir, art, TAIL_ARTIFACT_MAX_BYTES).ok()
            })
            .filter(|s| !s.trim().is_empty())
    } else {
        None
    };

    if let Some(trimmed) = step_digest::trim_failure_tail(
        error,
        inline_stderr.or(artifact_stderr.as_deref()),
        output_tail,
        inline_stdout.or(artifact_stdout.as_deref()),
        non_zero_exit_code(step),
        step_digest::DEFAULT_FAILURE_MAX_LINES,
        step_digest::DEFAULT_FAILURE_MAX_CHARS,
    ) {
        return Some(trimmed);
    }

    step_failure_reason(step)
}

fn extract_step_note(step: &Step, verdict: Option<&StepVerdict>) -> Option<(String, bool)> {
    const NOTE_KEYS: &[&str] = &[
        "summary",
        "note",
        "notes",
        "message",
        "answer",
        "reason",
        "rationale",
        "explanation",
        "detail",
    ];

    let raw = match &step.output {
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() || trimmed.starts_with('{') || trimmed.starts_with('[') {
                return None;
            }
            trimmed
        }
        Value::Object(map) => NOTE_KEYS.iter().find_map(|key| {
            map.get(*key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
        })?,
        _ => return None,
    };

    if let Some(v) = verdict {
        if let Some(reason) = v.reason.as_deref() {
            let reason_prefix = reason.trim_end_matches('…');
            if !reason_prefix.is_empty()
                && (raw == reason || raw.starts_with(reason_prefix) || reason.starts_with(raw))
            {
                return None;
            }
        }
    }

    Some(truncate_note_text(raw))
}

fn truncate_note_text(text: &str) -> (String, bool) {
    const MAX_LINES: usize = 3;
    const MAX_CHARS: usize = 240;

    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return (String::new(), true);
    }

    let mut truncated = lines.len() > MAX_LINES;
    let joined = lines
        .iter()
        .take(MAX_LINES)
        .copied()
        .collect::<Vec<_>>()
        .join("\n");

    let char_count = joined.chars().count();
    let mut out = if char_count > MAX_CHARS {
        truncated = true;
        joined.chars().take(MAX_CHARS.saturating_sub(1)).collect()
    } else {
        joined
    };

    if truncated {
        out = out.trim_end().to_string();
        out.push('…');
    }

    (out, !truncated)
}

fn has_extra_result_fields(run: &Run, step: &Step, note_covered_entire_string: bool) -> bool {
    match &step.output {
        Value::Null => false,
        Value::String(s) => !s.trim().is_empty() && !note_covered_entire_string,
        Value::Array(arr) => !arr.is_empty(),
        Value::Bool(_) | Value::Number(_) => true,
        Value::Object(map) => {
            if map.is_empty() {
                return false;
            }
            let switch_on_key = run.flow.edges.iter().find_map(|edge| match edge {
                FlowEdge::Switch { from, on, .. } if from == &step.node_id => on.split('.').next(),
                _ => None,
            });
            map.keys()
                .any(|key| !is_digest_covered_key(key, switch_on_key))
        }
    }
}

fn is_digest_covered_key(key: &str, switch_on_key: Option<&str>) -> bool {
    if switch_on_key == Some(key) {
        return true;
    }
    matches!(
        key,
        "verdict"
            | "decision"
            | "route"
            | "reason"
            | "rationale"
            | "summary"
            | "explanation"
            | "detail"
            | "notes"
            | "note"
            | "message"
            | "answer"
            | "ok"
            | "status"
    )
}
~~~

## Edit 4: Adapt test_step_sections_order_prompt_result_events for digest-first order
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs
OLD:
~~~rust
    #[test]
    fn test_step_sections_order_prompt_result_events() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_500,
            prompt_text: Some("Inspect the repository".to_string()),
            raw_text: None,
            output: json!({"answer": "all good"}),
            error: None,
            agent_name: Some("claude".to_string()),
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });
        run.events.push(TraceEvent {
            seq: 1,
            at_ms: 1_100,
            scope: "node".to_string(),
            kind: "node_started".to_string(),
            node_id: Some("work".to_string()),
            attempt_id: Some("work#1".to_string()),
            detail: "Running acp node".to_string(),
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 40));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 80, 40),
            &run,
            &DetailTarget::Step(0),
            &Theme::default(),
            5_000,
        );
        let text = buffer_lines(&buf).join("\n");
        let p_idx = text.find("── Prompt").expect("Prompt section");
        let r_idx = text.find("── Result").expect("Result section");
        let e_idx = text.find("── Events").expect("Events section");
        assert!(p_idx < r_idx && r_idx < e_idx);
    }
~~~
NEW:
~~~rust
    #[test]
    fn test_step_sections_order_prompt_result_events() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_500,
            prompt_text: Some("Inspect the repository".to_string()),
            raw_text: None,
            output: json!({"answer": "all good"}),
            error: None,
            agent_name: Some("claude".to_string()),
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });
        run.events.push(TraceEvent {
            seq: 1,
            at_ms: 1_100,
            scope: "node".to_string(),
            kind: "node_started".to_string(),
            node_id: Some("work".to_string()),
            attempt_id: Some("work#1".to_string()),
            detail: "Running acp node".to_string(),
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 40));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 80, 40),
            &run,
            &DetailTarget::Step(0),
            &Theme::default(),
            5_000,
        );
        let text = buffer_lines(&buf).join("\n");
        let d_idx = text.find("── Digest").expect("Digest section");
        let e_idx = text.find("── Events").expect("Events section");
        assert!(d_idx < e_idx, "{text}");
        assert!(text.contains("all good"), "{text}");
        assert!(text.contains("Enter inspect"), "{text}");
        assert!(!text.contains("r replay"), "{text}");
        assert!(!text.contains("── Prompt"), "{text}");
    }
~~~

## Edit 5: Add attempt digest tests for failed shell, repeated attempts, agent evidence, and pending gate
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs
OLD:
~~~rust
    #[test]
    fn test_target_change_resets_scroll() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: Some("a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm".to_string()),
            raw_text: None,
            output: json!({"res": "1\n2\n3\n4\n5\n6\n7\n8"}),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 50, 6));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 50, 6),
            &run,
            &DetailTarget::Step(0),
            &Theme::default(),
            5_000,
        );
        assert!(panel.scroll(5));
        assert!(panel.scroll > 0);

        panel.render(
            &mut buf,
            Rect::new(0, 0, 50, 6),
            &run,
            &DetailTarget::Run,
            &Theme::default(),
            5_000,
        );
        assert_eq!(panel.scroll, 0);
    }
~~~
NEW:
~~~rust
    #[test]
    fn test_target_change_resets_scroll() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: Some("a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm".to_string()),
            raw_text: None,
            output: json!({"res": "1\n2\n3\n4\n5\n6\n7\n8"}),
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let mut panel = DetailPanel::new();
        let mut buf = Buffer::empty(Rect::new(0, 0, 50, 6));
        panel.render(
            &mut buf,
            Rect::new(0, 0, 50, 6),
            &run,
            &DetailTarget::Step(0),
            &Theme::default(),
            5_000,
        );
        assert!(panel.scroll(5));
        assert!(panel.scroll > 0);

        panel.render(
            &mut buf,
            Rect::new(0, 0, 50, 6),
            &run,
            &DetailTarget::Run,
            &Theme::default(),
            5_000,
        );
        assert_eq!(panel.scroll, 0);
    }

    #[test]
    fn test_failed_shell_orders_failure_tail_before_command_and_cwd_without_success() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let art_dir = tmp.path().join("artifacts");
        std::fs::create_dir_all(&art_dir).expect("mkdir");
        let stderr_rel = "artifacts/err.txt";
        let stderr_body = (0..10)
            .map(|i| format!("err line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(tmp.path().join(stderr_rel), &stderr_body).expect("write stderr");

        let mut run = make_test_run(tmp.path());
        run.steps.push(Step {
            attempt_id: "list#1".to_string(),
            node_id: "list".to_string(),
            kind: NodeKind::Action,
            // Raw outcome is Ok, but nonzero exit code must render as failed.
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 1_250,
            prompt_text: None,
            raw_text: None,
            output: Value::Null,
            error: None,
            agent_name: None,
            action: Some(ActionReceipt {
                action_type: "shell".to_string(),
                command: Some("cargo".to_string()),
                args: vec!["test".to_string(), "--lib".to_string()],
                cwd: Some("/private/tmp/repo".to_string()),
                exit_code: Some(1),
                duration_ms: Some(250),
            }),
            conversation: None,
            stdout: None,
            stderr: Some(ArtifactRef {
                path: stderr_rel.to_string(),
                media_type: "text/plain".to_string(),
                bytes: stderr_body.len() as u64,
            }),
            output_artifact: None,
        });

        let text = panel_text(&run, &DetailTarget::Step(0), None);
        assert!(text.contains("✗ list"), "{text}");
        assert!(!text.contains("✓ list"), "{text}");
        assert!(text.contains("attempt 1"), "{text}");
        assert!(text.contains("exit 1"), "{text}");
        assert!(text.contains("Enter inspect"), "{text}");
        assert!(!text.contains("r replay"), "{text}");

        let fail_idx = text.find("── Failure").expect("Failure section");
        let tail_idx = text.find("err line 9").expect("bounded tail");
        let cmd_idx = text.find("── Command").expect("Command section");
        let cmd_line_idx = text.find("cargo test --lib").expect("command line");
        let cwd_idx = text.find("/private/tmp/repo").expect("cwd");
        assert!(fail_idx < tail_idx, "{text}");
        assert!(tail_idx < cmd_idx, "{text}");
        assert!(cmd_idx < cmd_line_idx && cmd_line_idx < cwd_idx, "{text}");
        assert!(!text[fail_idx..cmd_idx].contains("err line 0"), "{text}");
    }

    #[test]
    fn test_repeated_attempt_shows_own_number_and_timing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        if let Some(node) = run.flow.nodes.iter_mut().find(|n| n.id == "work") {
            node.timeout_ms = Some(10_000);
        }
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Failed,
            started_ms: 1_000,
            finished_ms: 3_000,
            prompt_text: None,
            raw_text: None,
            output: Value::Null,
            error: Some("first try failed".to_string()),
            agent_name: Some("claude".to_string()),
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });
        run.steps.push(Step {
            attempt_id: "work#2".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 4_000,
            finished_ms: 7_000,
            prompt_text: None,
            raw_text: None,
            output: json!({"summary": "fixed on retry"}),
            error: None,
            agent_name: Some("claude".to_string()),
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let first = panel_text(&run, &DetailTarget::Step(0), None);
        assert!(first.contains("attempt 1"), "{first}");
        assert!(
            first.contains(&format!("{}/{}", fmt_duration(2_000), fmt_duration(10_000))),
            "{first}"
        );

        let second = panel_text(&run, &DetailTarget::Step(1), None);
        assert!(second.contains("attempt 2"), "{second}");
        assert!(
            second.contains(&format!("{}/{}", fmt_duration(3_000), fmt_duration(10_000))),
            "{second}"
        );
    }

    #[test]
    fn test_agent_with_brief_evidence_omits_prompt_wrappers_and_transcript_in_first_view() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let events_path = tmp.path().join("events.ndjson");
        let lines = [
            json!({
                "seq": 1,
                "direction": "inbound",
                "message": {
                    "method": "session/update",
                    "params": {
                        "update": {
                            "sessionUpdate": "tool_call",
                            "toolCallId": "tc1",
                            "title": "Edit",
                            "kind": "edit",
                            "status": "completed",
                            "rawInput": {
                                "path": "crates/app/src/lib.rs",
                                "file_path": "crates/app/src/lib.rs"
                            }
                        }
                    }
                }
            }),
            json!({
                "seq": 2,
                "direction": "inbound",
                "message": {
                    "method": "session/update",
                    "params": {
                        "update": {
                            "sessionUpdate": "agent_message_chunk",
                            "content": {"text": "TRANSCRIPT_BODY_SHOULD_NOT_DUMP"}
                        }
                    }
                }
            }),
        ];
        let body = lines
            .iter()
            .map(|v| serde_json::to_string(v).expect("json"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&events_path, body).expect("write events");

        let mut run = make_test_run(tmp.path());
        run.flow.edges = vec![
            FlowEdge::Simple {
                from: "list".to_string(),
                to: "work".to_string(),
            },
            FlowEdge::Switch {
                from: "work".to_string(),
                on: "verdict".to_string(),
                cases: vec![("approve".to_string(), "review".to_string())],
            },
        ];
        run.sessions.push(SessionRef {
            id: "s1".to_string(),
            handle: "main".to_string(),
            events_path,
            agent_name: Some("claude".to_string()),
        });
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: Some("Update crates/app/src/lib.rs\nLONG_PROMPT_MARKER".to_string()),
            raw_text: Some("{\"verdict\":\"approve\"}".to_string()),
            output: json!({"verdict": "approve", "reason": "All checks passed"}),
            error: None,
            agent_name: Some("claude".to_string()),
            action: None,
            conversation: Some(Conversation {
                session_id: "s1".to_string(),
                event_start_seq: 1,
                event_end_seq: 2,
            }),
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let text = panel_text(&run, &DetailTarget::Step(0), None);
        assert!(text.contains("── Digest"), "{text}");
        assert!(
            text.contains("verdict: approve -> review (All checks passed)"),
            "{text}"
        );
        assert!(text.contains("files "), "{text}");
        assert!(text.contains("crates/app/src/lib.rs"), "{text}");
        assert!(text.contains("Enter inspect · r replay"), "{text}");
        assert!(!text.contains("LONG_PROMPT_MARKER"), "{text}");
        assert!(!text.contains("TRANSCRIPT_BODY_SHOULD_NOT_DUMP"), "{text}");
        assert!(!text.contains("\"verdict\":"), "{text}");
        assert!(!text.contains("── Prompt"), "{text}");
        assert!(!text.contains("── Conversation"), "{text}");
    }

    #[test]
    fn test_agent_without_evidence_does_not_invent_it() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut run = make_test_run(tmp.path());
        run.steps.push(Step {
            attempt_id: "work#1".to_string(),
            node_id: "work".to_string(),
            kind: NodeKind::Acp,
            outcome: Outcome::Ok,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: None,
            raw_text: None,
            output: Value::Null,
            error: None,
            agent_name: Some("claude".to_string()),
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        });

        let text = panel_text(&run, &DetailTarget::Step(0), None);
        assert!(text.contains("attempt 1"), "{text}");
        assert!(text.contains("Enter inspect"), "{text}");
        assert!(!text.contains("r replay"), "{text}");
        assert!(!text.contains("── Digest"), "{text}");
        assert!(!text.contains("verdict:"), "{text}");
        assert!(!text.contains("validation"), "{text}");
        assert!(!text.contains("files "), "{text}");
        assert!(!text.contains("exit "), "{text}");
        let header = text.lines().next().unwrap_or("");
        assert!(!header.contains('/'), "{header}");
    }

    #[test]
    fn test_pending_gate_keeps_question_and_choices_conspicuous() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let run = waiting_gate_run(tmp.path());

        let text = panel_text(&run, &DetailTarget::Step(1), None);
        assert!(text.contains("◐ review"), "{text}");
        let question_idx = text
            .find("land it, or send it back?")
            .expect("gate question");
        let gate_idx = text.find("── Gate").expect("Gate section");
        let choice_idx = text
            .find("[1] abort → rollback.flow.ts")
            .expect("first choice");
        assert!(question_idx < gate_idx && gate_idx < choice_idx, "{text}");
        assert!(text.contains("[2] ship → deploy.flow.ts"), "{text}");
        assert!(text.contains("Enter inspect"), "{text}");
        assert!(!text.contains("r replay"), "{text}");
    }
~~~
