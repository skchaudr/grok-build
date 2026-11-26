use crate::acpx::bundle::{Outcome, Run, RunStatus, Step, fmt_duration, gate_choices};
use crate::acpx::step_digest::{
    digest_step_files, effective_outcome, extract_step_validations, extract_step_verdict,
    latest_run_failure, non_zero_exit_code, parse_scoped_node_id, run_step_budget,
    step_attempt_budget, step_time_budget, step_trace_link, summarize_included_workflows,
};

/// Formats a single-line live status bar for an ACPX workflow run.
pub fn format_run_status_line(run: &Run, answered: Option<&str>, now_ms: i64) -> String {
    let title = run_title(run);
    let (done, total) = run.progress();
    let dur = fmt_duration(run.duration_ms(now_ms));
    let step_budget = run_step_budget(run).badge_label();

    if run.summary.status == RunStatus::Waiting && answered.is_none() {
        let gate_node = waiting_gate_node(run);
        let choices = format_gate_choices_inline(run);

        let mut line = format!("◐ {title} · {done}/{total} steps");
        if let Some(budget) = step_budget {
            line.push_str(&format!(" ({budget})"));
        }
        line.push_str(&format!(" · {dur} · WAITING FOR YOUR ANSWER at {gate_node}"));
        if !choices.is_empty() {
            line.push_str(&format!(" [{choices}]"));
        }
        if let Some((fail_node, code_opt, _)) = latest_run_failure(run) {
            match code_opt {
                Some(code) => line.push_str(&format!(" · last failed: {fail_node} (exit {code})")),
                None => line.push_str(&format!(" · last failed: {fail_node}")),
            }
        }
        return line;
    }

    if run.summary.status == RunStatus::Running {
        let curr = run.summary.current_node.as_deref().unwrap_or("…");
        let mut line = format!("● {title} · {done}/{total} steps");
        if let Some(budget) = step_budget {
            line.push_str(&format!(" ({budget})"));
        }
        line.push_str(&format!(" · {dur} · running {curr}"));

        if let Some(step) = run.steps.iter().rev().find(|s| s.node_id == curr) {
            if let Some(agent) = step.agent_name.as_deref().filter(|a| !a.is_empty()) {
                line.push_str(&format!(" ({agent})"));
            }
            if let Some(badge) = step_attempt_budget(run, step).badge_label() {
                line.push_str(&format!(" · {badge}"));
            }
            let tb = step_time_budget(run, step, now_ms);
            if tb.timeout_ms.is_some() || tb.timed_out {
                line.push_str(&format!(" · {}", tb.badge_label()));
            }
            if let Some(vbadge) = extract_step_validations(run, step, &[]).badge_label() {
                line.push_str(&format!(" · {vbadge}"));
            }
            let fline = digest_step_files(run, step, &[]).summary_line();
            if !fline.is_empty() {
                line.push_str(&format!(" · {fline}"));
            }
        }
        return line;
    }

    let eff_status = if run.summary.status == RunStatus::Waiting && answered.is_some() {
        RunStatus::Completed
    } else {
        run.summary.status
    };
    let status_label = if eff_status == RunStatus::TimedOut {
        "TIMED OUT"
    } else {
        eff_status.label()
    };

    let mut line = format!("{} {title} · {done}/{total} steps", eff_status.glyph());
    if let Some(budget) = step_budget {
        line.push_str(&format!(" ({budget})"));
    }
    line.push_str(&format!(" · {dur} · {status_label}"));

    if matches!(eff_status, RunStatus::Failed | RunStatus::TimedOut)
        && let Some((fail_node, code_opt, _)) = latest_run_failure(run)
    {
        match code_opt {
            Some(code) => line.push_str(&format!(" at {fail_node} (exit {code})")),
            None => line.push_str(&format!(" at {fail_node}")),
        }
    }

    line
}

/// Formats a structured multi-line `/status` report for an ACPX workflow run.
pub fn format_run_status_report(run: &Run, answered: Option<&str>, now_ms: i64) -> String {
    let mut out = format!(
        "**{}** (`{}`)",
        format_run_status_line(run, answered, now_ms),
        run.summary.run_id
    );

    let included = summarize_included_workflows(run);
    if !included.is_empty() {
        out.push_str("\n**Included workflows:**");
        for inc in &included {
            out.push_str(&format!("\n- {}", inc.summary_line()));
        }
    }

    if !run.steps.is_empty() {
        out.push_str("\n**Steps:**");
        for (idx, step) in run.steps.iter().enumerate() {
            out.push('\n');
            out.push_str(&format_step_bullet(run, idx, step, now_ms));
        }
    }

    if run.summary.status == RunStatus::Waiting && answered.is_none() {
        append_waiting_gate_section(&mut out, run);
    }

    out
}

fn run_title(run: &Run) -> &str {
    if !run.summary.title.is_empty() {
        &run.summary.title
    } else if !run.summary.flow_name.is_empty() {
        &run.summary.flow_name
    } else {
        &run.summary.run_id
    }
}

fn waiting_gate_node(run: &Run) -> &str {
    run.summary
        .waiting_on
        .as_deref()
        .or(run.summary.current_node.as_deref())
        .unwrap_or("gate")
}

fn format_gate_choices_inline(run: &Run) -> String {
    gate_choices(run)
        .into_iter()
        .enumerate()
        .map(|(idx, (label, _))| format!("{}: {label}", idx + 1))
        .collect::<Vec<_>>()
        .join(", ")
}

fn outcome_glyph(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Ok => "✓",
        Outcome::TimedOut => "⏱",
        Outcome::Failed => "✗",
        Outcome::Cancelled => "⊘",
        Outcome::Unknown => "?",
    }
}

fn display_node_name(run: &Run, node_id: &str) -> String {
    if let (Some(mount), _, Some(exit_name)) = parse_scoped_node_id(node_id) {
        return format!("{mount} ↳ exit:{exit_name}");
    }
    if let Some(node) = run.flow.nodes.iter().find(|n| n.id == node_id)
        && node.include_transition.as_deref() == Some("exit")
        && !node.mount_path.is_empty()
        && let Some(exit_name) = node.local_node_id.as_deref().filter(|s| !s.is_empty())
    {
        return format!("{} ↳ exit:{exit_name}", node.mount_path.join("/"));
    }
    node_id.to_string()
}

struct StepBulletParts<'a> {
    glyph: &'static str,
    display_name: String,
    attempt_badge: Option<String>,
    agent_name: Option<&'a str>,
    time_badge: String,
    exit_code: Option<i64>,
    validation_badge: Option<String>,
    verdict_line: Option<String>,
    file_line: String,
    trace_badge: Option<String>,
}

fn render_step_bullet(parts: &StepBulletParts<'_>) -> String {
    let mut line = format!("- {} {}", parts.glyph, parts.display_name);

    if let Some(badge) = parts.attempt_badge.as_deref().filter(|s| !s.is_empty()) {
        line.push_str(&format!(" ({badge})"));
    }
    if let Some(agent) = parts.agent_name.filter(|s| !s.is_empty()) {
        line.push_str(&format!(" [{agent}]"));
    }
    if !parts.time_badge.is_empty() {
        line.push_str(&format!(" · {}", parts.time_badge));
    }
    if let Some(code) = parts.exit_code {
        line.push_str(&format!(" (exit {code})"));
    }
    if let Some(vbadge) = parts.validation_badge.as_deref().filter(|s| !s.is_empty()) {
        line.push_str(&format!(" · {vbadge}"));
    }
    if let Some(vline) = parts.verdict_line.as_deref().filter(|s| !s.is_empty()) {
        line.push_str(&format!(" · {vline}"));
    }
    if !parts.file_line.is_empty() {
        line.push_str(&format!(" · {}", parts.file_line));
    }
    if let Some(tbadge) = parts.trace_badge.as_deref().filter(|s| !s.is_empty()) {
        line.push_str(&format!(" · {tbadge}"));
    }

    line
}

fn format_step_bullet(run: &Run, idx: usize, step: &Step, now_ms: i64) -> String {
    let glyph = outcome_glyph(effective_outcome(step));
    let display_name = display_node_name(run, &step.node_id);
    let attempt_badge = step_attempt_budget(run, step).badge_label();
    let agent_name = step.agent_name.as_deref();
    let time_badge = step_time_budget(run, step, now_ms).badge_label();
    let exit_code = non_zero_exit_code(step);
    let validation_badge = extract_step_validations(run, step, &[]).badge_label();
    let verdict_line = extract_step_verdict(run, idx).map(|v| v.summary_line());
    let file_line = digest_step_files(run, step, &[]).summary_line();
    let trace_badge = step_trace_link(run, step).badge_label();

    render_step_bullet(&StepBulletParts {
        glyph,
        display_name,
        attempt_badge,
        agent_name,
        time_badge,
        exit_code,
        validation_badge,
        verdict_line,
        file_line,
        trace_badge,
    })
}

fn append_waiting_gate_section(out: &mut String, run: &Run) {
    let gate_node = waiting_gate_node(run);
    out.push_str(&format!("\n**Waiting gate:** `{gate_node}`"));

    if let Some(detail) = run
        .summary
        .status_detail
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.push_str(&format!("\n- Detail: {detail}"));
    }

    let choices = gate_choices(run);
    if !choices.is_empty() {
        for (idx, (label, flow_file)) in choices.iter().enumerate() {
            if flow_file.is_empty() {
                out.push_str(&format!("\n- {}: {label}", idx + 1));
            } else {
                out.push_str(&format!("\n- {}: {label} (`{flow_file}`)", idx + 1));
            }
        }
    }

    if let Some((fail_node, code_opt, reason)) = latest_run_failure(run) {
        let first_line = reason
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        match (code_opt, first_line.is_empty()) {
            (Some(code), false) => {
                out.push_str(&format!(
                    "\n- Last failure: {fail_node} (exit {code}): {first_line}"
                ));
            }
            (Some(code), true) => {
                out.push_str(&format!("\n- Last failure: {fail_node} (exit {code})"));
            }
            (None, false) => {
                out.push_str(&format!("\n- Last failure: {fail_node}: {first_line}"));
            }
            (None, true) => {
                out.push_str(&format!("\n- Last failure: {fail_node}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use serde_json::{Value, json};

    use crate::acpx::bundle::{
        ActionReceipt, ArtifactRef, Conversation, FlowDef, FlowEdge, FlowMount, FlowNode, NodeKind,
        RunSummary, SessionRef,
    };
    use crate::acpx::step_digest::{StepFileDigest, ValidationDigest};

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

    struct TempEventsFile {
        path: PathBuf,
    }

    impl TempEventsFile {
        fn new(name: &str, lines: &[Value]) -> Self {
            let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "run_status_bar_{name}_{}_{}.ndjson",
                std::process::id(),
                id
            ));
            let content = lines
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            fs::write(&path, content).expect("write temp events.ndjson");
            Self { path }
        }
    }

    impl Drop for TempEventsFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    fn base_node(id: &str, kind: NodeKind) -> FlowNode {
        FlowNode {
            id: id.to_string(),
            kind,
            summary: None,
            is_shell: matches!(kind, NodeKind::Action),
            timeout_ms: None,
            mount_path: Vec::new(),
            local_node_id: None,
            include_transition: None,
        }
    }

    fn base_step(attempt_id: &str, node_id: &str, kind: NodeKind, outcome: Outcome) -> Step {
        Step {
            attempt_id: attempt_id.to_string(),
            node_id: node_id.to_string(),
            kind,
            outcome,
            started_ms: 1_000,
            finished_ms: 2_000,
            prompt_text: None,
            raw_text: None,
            output: Value::Null,
            error: None,
            agent_name: None,
            action: None,
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        }
    }

    fn base_run(run_id: &str, status: RunStatus) -> Run {
        Run {
            summary: RunSummary {
                run_id: run_id.to_string(),
                dir: PathBuf::from(format!("/tmp/{run_id}")),
                flow_name: "demo-flow".to_string(),
                title: "Demo Run".to_string(),
                status,
                started_ms: 1_000,
                updated_ms: 5_000,
                finished_ms: None,
                current_node: None,
                waiting_on: None,
                status_detail: None,
                error: None,
            },
            flow: FlowDef {
                name: "demo-flow".to_string(),
                start_at: "start".to_string(),
                nodes: Vec::new(),
                edges: Vec::new(),
                max_steps: None,
                mounts: Vec::new(),
            },
            steps: Vec::new(),
            events: Vec::new(),
            input: Value::Null,
            sessions: Vec::new(),
        }
    }

    fn tool_call_event(
        seq: u64,
        tool_call_id: &str,
        title: &str,
        kind: &str,
        status: &str,
        raw_input: Value,
        raw_output: Value,
    ) -> Value {
        json!({
            "seq": seq,
            "direction": "inbound",
            "message": {
                "method": "session/update",
                "params": {
                    "update": {
                        "sessionUpdate": "tool_call",
                        "toolCallId": tool_call_id,
                        "title": title,
                        "kind": kind,
                        "status": status,
                        "rawInput": raw_input,
                        "rawOutput": raw_output,
                        "truncated": false
                    }
                }
            }
        })
    }

    #[test]
    fn running_status_line_shows_step_budget_attempt_timeout_and_file_progress() {
        let events = TempEventsFile::new(
            "running_loop",
            &[tool_call_event(
                1,
                "tc-edit-1",
                "edit",
                "edit",
                "completed",
                json!({ "path": "crates/app/src/lib.rs" }),
                json!({ "ok": true }),
            )],
        );

        let mut run = base_run("run-running-1", RunStatus::Running);
        run.summary.title = "Loop Fix".to_string();
        run.summary.current_node = Some("repair".to_string());
        run.summary.started_ms = 1_000;
        run.summary.updated_ms = 100_000;

        let mut repair_node = base_node("repair", NodeKind::Acp);
        repair_node.timeout_ms = Some(120_000);
        run.flow.nodes = vec![repair_node];
        run.flow.max_steps = Some(20);

        let mut attempt_1 = base_step("att-1", "repair", NodeKind::Acp, Outcome::Failed);
        attempt_1.started_ms = 1_000;
        attempt_1.finished_ms = 10_000;

        let mut attempt_2 = base_step("att-2", "repair", NodeKind::Acp, Outcome::Unknown);
        attempt_2.started_ms = 10_000;
        attempt_2.finished_ms = 100_000;
        attempt_2.agent_name = Some("coder".to_string());
        attempt_2.prompt_text =
            Some("Update crates/app/src/lib.rs and crates/app/src/main.rs".to_string());
        attempt_2.conversation = Some(Conversation {
            session_id: "main".to_string(),
            event_start_seq: 1,
            event_end_seq: 5,
        });

        run.steps = vec![attempt_1, attempt_2];
        run.sessions = vec![SessionRef {
            id: "main".to_string(),
            handle: "main".to_string(),
            events_path: events.path.clone(),
            agent_name: Some("coder".to_string()),
        }];

        let line = format_run_status_line(&run, None, 100_000);
        assert!(
            line.contains("steps 2/20 (18 left)"),
            "missing step budget in: {line}"
        );
        assert!(line.contains("attempt 2"), "missing attempt badge in: {line}");
        assert!(
            line.contains("⚠ 1m30s/2m00s (75%)"),
            "missing time budget badge in: {line}"
        );
        assert!(line.contains("(coder)"), "missing agent name in: {line}");

        let fline = digest_step_files(&run, &run.steps[1], &[]).summary_line();
        assert!(!fline.is_empty(), "expected non-empty file progress summary");
        assert!(
            line.contains(&fline),
            "missing file progress `{fline}` in: {line}"
        );
    }

    #[test]
    fn waiting_status_line_shows_gate_choices_and_last_failure_exit_code() {
        let mut run = base_run("run-waiting-1", RunStatus::Waiting);
        run.summary.title = "Release Gate".to_string();
        run.summary.waiting_on = Some("final_review_gate".to_string());
        run.summary.started_ms = 1_000;
        run.summary.updated_ms = 15_000;

        run.flow.nodes = vec![
            base_node("cargo_check", NodeKind::Action),
            base_node("final_review_gate", NodeKind::Checkpoint),
        ];

        let mut check_step = base_step("att-1", "cargo_check", NodeKind::Action, Outcome::Ok);
        check_step.action = Some(ActionReceipt {
            action_type: "shell".to_string(),
            command: Some("cargo".to_string()),
            args: vec!["check".to_string()],
            cwd: None,
            exit_code: Some(101),
            duration_ms: Some(2_300),
        });
        check_step.output = json!({ "exitCode": 101 });
        check_step.error = Some("error[E0308]: mismatched types".to_string());

        let mut gate_step = base_step(
            "att-2",
            "final_review_gate",
            NodeKind::Checkpoint,
            Outcome::Ok,
        );
        gate_step.output = json!({
            "next": {
                "repair": "repair.json",
                "accept": "accept.json"
            }
        });

        run.steps = vec![check_step, gate_step];

        let line = format_run_status_line(&run, None, 20_000);
        assert!(
            line.contains(
                "WAITING FOR YOUR ANSWER at final_review_gate [1: accept, 2: repair] · last failed: cargo_check (exit 101)"
            ),
            "unexpected waiting status line: {line}"
        );

        let answered_line = format_run_status_line(&run, Some("accept"), 20_000);
        assert!(
            answered_line.starts_with("✓ Release Gate"),
            "answered waiting run should render as completed: {answered_line}"
        );
        assert!(
            !answered_line.contains("WAITING FOR YOUR ANSWER"),
            "answered waiting run must not show waiting prompt: {answered_line}"
        );
    }

    #[test]
    fn status_report_includes_mounts_attempts_budgets_validations_verdicts_links_glyphs_and_gate() {
        let events = TempEventsFile::new(
            "report_full",
            &[
                tool_call_event(
                    1,
                    "tc-val-1",
                    "validate_output",
                    "validate",
                    "failed",
                    json!({ "verdict": "maybe" }),
                    json!({
                        "valid": false,
                        "error": "schema mismatch: expected approve or reject"
                    }),
                ),
                tool_call_event(
                    2,
                    "tc-edit-1",
                    "edit",
                    "edit",
                    "completed",
                    json!({ "path": "src/lib.rs" }),
                    json!({ "ok": true }),
                ),
                tool_call_event(
                    3,
                    "tc-val-2",
                    "validate_output",
                    "validate",
                    "completed",
                    json!({ "verdict": "approve" }),
                    json!({ "valid": true }),
                ),
            ],
        );

        let mut run = base_run("run-report-1", RunStatus::Waiting);
        run.summary.title = "Full Pipeline".to_string();
        run.summary.waiting_on = Some("final_review_gate".to_string());
        run.summary.status_detail = Some("Review diff before merging".to_string());
        run.summary.started_ms = 0;
        run.summary.updated_ms = 60_000;

        let mut plan_node = base_node("plan", NodeKind::Acp);
        plan_node.timeout_ms = Some(30_000);

        let mut review_entry = base_node("review", NodeKind::Compute);
        review_entry.include_transition = Some("entry".to_string());

        let mut review_children = Vec::new();
        for i in 1..=5 {
            let mut child = base_node(&format!("review/s{i}"), NodeKind::Acp);
            child.mount_path = vec!["review".to_string()];
            child.local_node_id = Some(format!("s{i}"));
            review_children.push(child);
        }

        let mut review_exit = base_node("review/__piw_exit_passed", NodeKind::Compute);
        review_exit.mount_path = vec!["review".to_string()];
        review_exit.local_node_id = Some("passed".to_string());
        review_exit.include_transition = Some("exit".to_string());

        run.flow.max_steps = Some(50);
        run.flow.mounts = vec![FlowMount {
            mount_path: vec!["review".to_string()],
            workflow_name: "code-review".to_string(),
            max_steps: Some(30),
        }];
        run.flow.nodes = vec![
            plan_node,
            base_node("cargo_check", NodeKind::Action),
            review_entry,
        ];
        run.flow.nodes.extend(review_children);
        run.flow.nodes.push(review_exit);
        run.flow.nodes.push(base_node("review_router", NodeKind::Acp));
        run.flow
            .nodes
            .push(base_node("final_review_gate", NodeKind::Checkpoint));
        run.flow.edges = vec![FlowEdge::Switch {
            from: "review_router".to_string(),
            on: "verdict".to_string(),
            cases: vec![
                ("approve".to_string(), "final_review_gate".to_string()),
                ("repair".to_string(), "plan".to_string()),
            ],
        }];

        let mut plan_1 = base_step("a-plan-1", "plan", NodeKind::Acp, Outcome::TimedOut);
        plan_1.started_ms = 0;
        plan_1.finished_ms = 30_000;
        plan_1.agent_name = Some("planner".to_string());

        let mut plan_2 = base_step("a-plan-2", "plan", NodeKind::Acp, Outcome::Ok);
        plan_2.started_ms = 30_000;
        plan_2.finished_ms = 40_000;
        plan_2.agent_name = Some("planner".to_string());
        plan_2.prompt_text = Some("Edit src/lib.rs".to_string());
        plan_2.conversation = Some(Conversation {
            session_id: "main".to_string(),
            event_start_seq: 1,
            event_end_seq: 3,
        });

        let mut check = base_step("a-check", "cargo_check", NodeKind::Action, Outcome::Ok);
        check.action = Some(ActionReceipt {
            action_type: "shell".to_string(),
            command: Some("cargo".to_string()),
            args: vec!["check".to_string()],
            cwd: None,
            exit_code: Some(101),
            duration_ms: Some(1_500),
        });
        check.output = json!({ "exitCode": 101 });
        check.stderr = Some(ArtifactRef {
            path: "artifacts/cargo.stderr".to_string(),
            media_type: "text/plain".to_string(),
            bytes: 128,
        });

        let entry_step = base_step("a-rev-in", "review", NodeKind::Compute, Outcome::Ok);
        let child_steps: Vec<Step> = (1..=5)
            .map(|i| {
                base_step(
                    &format!("a-rev-{i}"),
                    &format!("review/s{i}"),
                    NodeKind::Acp,
                    Outcome::Ok,
                )
            })
            .collect();
        let exit_step = base_step(
            "a-rev-out",
            "review/__piw_exit_passed",
            NodeKind::Compute,
            Outcome::Ok,
        );

        let mut router = base_step("a-router", "review_router", NodeKind::Acp, Outcome::Ok);
        router.output = json!({
            "verdict": "approve",
            "reason": "all review checks passed"
        });

        let mut gate = base_step(
            "a-gate",
            "final_review_gate",
            NodeKind::Checkpoint,
            Outcome::Ok,
        );
        gate.output = json!({
            "next": {
                "accept": "accept.json",
                "repair": "repair.json"
            }
        });

        run.steps = vec![plan_1, plan_2, check, entry_step];
        run.steps.extend(child_steps);
        run.steps.push(exit_step);
        run.steps.push(router);
        run.steps.push(gate);

        run.sessions = vec![SessionRef {
            id: "main".to_string(),
            handle: "main".to_string(),
            events_path: events.path.clone(),
            agent_name: Some("planner".to_string()),
        }];

        let report = format_run_status_report(&run, None, 60_000);

        assert!(
            report.starts_with("**◐ Full Pipeline"),
            "missing bold header line:\n{report}"
        );
        assert!(
            report.contains("(`run-report-1`)"),
            "missing run_id in header:\n{report}"
        );
        assert!(
            report.contains("**Included workflows:**"),
            "missing Included workflows section:\n{report}"
        );
        assert!(
            report.contains("5/30 steps (25 left)"),
            "missing included mount step cap:\n{report}"
        );
        assert!(
            report.contains("- ⏱ plan"),
            "missing TimedOut glyph for plan attempt 1:\n{report}"
        );
        assert!(
            report.contains("attempt 2"),
            "missing attempt count badge:\n{report}"
        );
        assert!(
            report.contains("- ✗ cargo_check") && report.contains("(exit 101)"),
            "missing Failed glyph and exit code for cargo_check:\n{report}"
        );
        assert!(
            report.contains("review ↳ exit:passed"),
            "missing formatted included exit node:\n{report}"
        );
        assert!(
            report.contains("trace: events 1..3"),
            "missing session transcript trace badge:\n{report}"
        );
        assert!(
            report.contains("trace: stderr"),
            "missing stderr artifact trace badge:\n{report}"
        );

        let router_idx = run
            .steps
            .iter()
            .position(|s| s.node_id == "review_router")
            .expect("router step exists");
        let verdict = extract_step_verdict(&run, router_idx).expect("expected router verdict");
        assert!(
            report.contains(&verdict.summary_line()),
            "missing reviewer verdict `{}`:\n{report}",
            verdict.summary_line()
        );

        if let Some(vbadge) = extract_step_validations(&run, &run.steps[1], &[]).badge_label() {
            assert!(
                report.contains(&vbadge),
                "missing validator rejection badge `{vbadge}`:\n{report}"
            );
        }

        assert!(
            report.contains("**Waiting gate:** `final_review_gate`"),
            "missing waiting gate section:\n{report}"
        );
        assert!(
            report.contains("Review diff before merging"),
            "missing waiting gate detail:\n{report}"
        );
        assert!(
            report.contains("1: accept (`accept.json`)")
                && report.contains("2: repair (`repair.json`)"),
            "missing waiting gate choices:\n{report}"
        );
    }

    #[test]
    fn render_step_bullet_includes_validation_and_file_badges() {
        let validation = ValidationDigest {
            rejection_count: 1,
            correction_attempts: 1,
            resolved: true,
            last_error: Some("missing verdict".to_string()),
            last_rejected_output: None,
        };
        let files = StepFileDigest {
            expected_files: vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
            touched_files: vec!["src/a.rs".to_string()],
            out_of_scope_files: Vec::new(),
            current_file: Some("src/a.rs".to_string()),
        };

        let bullet = render_step_bullet(&StepBulletParts {
            glyph: "✓",
            display_name: "review/check".to_string(),
            attempt_badge: Some("attempt 2".to_string()),
            agent_name: Some("reviewer"),
            time_badge: "12.0s/1m00s (20%)".to_string(),
            exit_code: None,
            validation_badge: validation.badge_label(),
            verdict_line: Some("verdict: approve -> done".to_string()),
            file_line: files.summary_line(),
            trace_badge: Some("trace: events 10..20".to_string()),
        });

        assert!(bullet.contains("- ✓ review/check (attempt 2) [reviewer]"));
        assert!(bullet.contains("12.0s/1m00s (20%)"));
        assert!(bullet.contains("1 validation rejected -> corrected (missing verdict)"));
        assert!(bullet.contains("verdict: approve -> done"));
        assert!(bullet.contains("trace: events 10..20"));
    }

    #[test]
    fn title_fallbacks_and_terminal_statuses_format_cleanly() {
        let mut run = base_run("run-timeout-1", RunStatus::TimedOut);
        run.summary.title.clear();
        run.summary.flow_name.clear();
        run.summary.finished_ms = Some(61_000);

        let mut timed_out_step = base_step("a1", "slow_step", NodeKind::Acp, Outcome::TimedOut);
        timed_out_step.error = Some("step exceeded timeout".to_string());
        run.steps = vec![timed_out_step];
        run.flow.nodes = vec![base_node("slow_step", NodeKind::Acp)];

        let line = format_run_status_line(&run, None, 70_000);
        assert!(
            line.starts_with("✗ run-timeout-1 · 1/1 steps"),
            "expected run_id fallback and timed out glyph: {line}"
        );
        assert!(line.contains("TIMED OUT"), "missing TIMED OUT label: {line}");
        assert!(
            line.contains("at slow_step"),
            "missing failure node on timed out run: {line}"
        );
    }

    #[test]
    #[ignore]
    fn live_beta8_merge_v1_acceptance_check() {
        use std::fs;
        use std::path::Path;
        use std::sync::Arc;
        use std::thread;
        use std::time::Duration;
        use crate::acpx::bundle;
        use crate::acpx::step_digest::{
            digest_step_files, effective_outcome, run_step_budget, summarize_included_workflows,
        };
        use crate::acpx::workflow_preflight::{
            audit_workflow_preflight, format_preflight_markdown, PreflightOverrides,
        };
        use crate::piw::client::PiwClient;
        use crate::piw::mirror::PiwMirror;
        use crate::views::acpx_modal::modal_view::{
            build_waiting_gate_banner, format_step_structure_annotation, prune_run_for_graph,
        };
        use crate::theme::Theme;

        let mirror_root = Path::new("/Users/sab-mini/.grok/cache/pi-workflows-mirror");
        let beta8_id = "20261124T063602457Z-beta8-merge-8814563d";
        let monitor_id = "20261124T064326952Z-monitor-cd63bbf0";
        let beta8_dir = mirror_root.join(format!("piw-{beta8_id}"));
        let monitor_dir = mirror_root.join(format!("piw-{monitor_id}"));

        if let Some(sock) = PiwClient::default_socket_path().filter(|p| p.exists()) {
            for dir in [&beta8_dir, &monitor_dir] {
                let _ = fs::remove_file(dir.join(".mirror_meta.json"));
                let _ = fs::remove_file(dir.join("flow.json"));
                let _ = fs::remove_file(dir.join("trace.ndjson"));
                let _ = fs::remove_file(dir.join("projections/steps.json"));
                let _ = fs::remove_file(dir.join("sessions/main/events.ndjson"));
            }
            let client = Arc::new(PiwClient::connect(&sock).expect("connect PiwClient"));
            let mirror = PiwMirror::start(client, mirror_root.to_path_buf()).expect("start PiwMirror");
            mirror.want_run(beta8_id);
            mirror.want_run(monitor_id);
            for _ in 0..100 {
                thread::sleep(Duration::from_millis(150));
                if beta8_dir.join("flow.json").is_file()
                    && beta8_dir.join("sessions/main/events.ndjson").is_file()
                    && monitor_dir.join("flow.json").is_file()
                {
                    thread::sleep(Duration::from_millis(300));
                    break;
                }
            }
            mirror.stop();
        }

        let beta8_run = bundle::load_run(&beta8_dir).expect("load re-mirrored beta8-merge run");
        let monitor_run = bundle::load_run(&monitor_dir).expect("load re-mirrored monitor run");

        println!("=== 1. PREFLIGHT AUDIT ON beta8-merge ===");
        let preflight = audit_workflow_preflight(
            &beta8_run.flow,
            None,
            std::slice::from_ref(&beta8_run),
            &PreflightOverrides::default(),
        );
        println!("{}", format_preflight_markdown(&preflight));
        assert!(!preflight.ready_to_launch(), "beta8-merge without overrides must be blocked by preflight");

        println!("=== 2. PRUNED GRAPH NODES ON beta8-merge ===");
        let pruned = prune_run_for_graph(&beta8_run);
        let pruned_ids: Vec<&str> = pruned.flow.nodes.iter().map(|n| n.id.as_str()).collect();
        println!("Original nodes ({}): {:?}", beta8_run.flow.nodes.len(), beta8_run.flow.nodes.iter().map(|n| &n.id).collect::<Vec<_>>());
        println!("Pruned nodes   ({}): {:?}", pruned_ids.len(), pruned_ids);
        assert!(!pruned_ids.contains(&"resolve_group1_air"), "untaken air branch must be pruned");
        assert!(pruned_ids.contains(&"resolve_group2_shell"), "taken mini branch must remain");

        println!("=== 3. STEP OUTCOMES & ANNOTATIONS ON beta8-merge ===");
        let now_ms = beta8_run.summary.updated_ms;
        for (idx, step) in beta8_run.steps.iter().enumerate() {
            let eff = effective_outcome(step);
            let ann = format_step_structure_annotation(&beta8_run, idx, now_ms);
            println!("  step[{idx}] {} -> outcome={eff:?} | {ann}", step.node_id);
        }
        let cargo_step = beta8_run.steps.iter().find(|s| s.node_id == "cargo_check").expect("cargo_check step");
        assert_eq!(effective_outcome(cargo_step), Outcome::Failed, "cargo_check exit 101 must be Outcome::Failed");
        let g2_step = beta8_run.steps.iter().find(|s| s.node_id == "resolve_group2_shell").expect("resolve_group2_shell step");
        let g2_files = digest_step_files(&beta8_run, g2_step, &[]);
        println!("  resolve_group2_shell files: expected={}, touched={}, out_of_scope={:?}", g2_files.expected_files.len(), g2_files.touched_files.len(), g2_files.out_of_scope_files);

        println!("=== 4. STATUS REPORT ON beta8-merge ===");
        println!("{}", format_run_status_report(&beta8_run, None, now_ms));

        println!("=== 5. WAITING GATE BANNER ON beta8-merge ===");
        let theme = Theme::default();
        let banner = build_waiting_gate_banner(
            &beta8_run,
            "final_review_gate",
            Some("cargo_check failed with exit 101; choose accept or repair"),
            2,
            120,
            &theme,
        );
        for line in &banner {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            println!("  BANNER: {text}");
        }

        println!("=== 6. MONITOR RUN BUDGETS & INCLUDED WORKFLOWS ===");
        println!("  monitor root step budget: {:?}", run_step_budget(&monitor_run));
        for inc in summarize_included_workflows(&monitor_run) {
            println!("  monitor include: {}", inc.summary_line());
        }
    }
}
