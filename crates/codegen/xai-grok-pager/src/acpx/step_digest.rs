use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use serde_json::Value;

use crate::acpx::bundle::{
    FlowEdge, FlowNode, NodeKind, Outcome, Run, RunStatus, Step, fmt_duration,
};

pub use crate::acpx::step_evidence::{
    MergedToolCall, StepFileDigest, StepTraceLink, ValidationDigest, digest_step_files,
    extract_step_validations_with_outcome, read_attempt_tool_calls, step_trace_link,
};

pub const DEFAULT_FAILURE_MAX_LINES: usize = 6;
pub const DEFAULT_FAILURE_MAX_CHARS: usize = 480;

const VERDICT_REASON_MAX_CHARS: usize = 160;
const VERDICT_REASON_KEYS: &[&str] = &[
    "reason",
    "rationale",
    "summary",
    "explanation",
    "detail",
    "notes",
];

/// (a) Loop attempt count for one step execution (counted from `run.steps`; the engine has no per-node attempt limit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeAttemptBudget {
    pub attempt: usize,
    pub observed_attempts: usize,
}

impl NodeAttemptBudget {
    pub fn badge_label(&self) -> Option<String> {
        if self.attempt > 1 || self.observed_attempts > 1 {
            Some(format!("attempt {}", self.attempt))
        } else {
            None
        }
    }
}

/// (c) Reviewer or router verdict extracted from a step's output and outgoing switch edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepVerdict {
    pub route_key: String,
    pub target_node: Option<String>,
    pub reason: Option<String>,
}

impl StepVerdict {
    pub fn summary_line(&self) -> String {
        let mut line = match &self.target_node {
            Some(target) => format!("verdict: {} -> {target}", self.route_key),
            None => format!("verdict: {}", self.route_key),
        };
        if let Some(reason) = &self.reason {
            line.push_str(&format!(" ({reason})"));
        }
        line
    }
}

/// (d) Time budget (`elapsed_ms` vs `timeout_ms`) and timeout state for a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepTimeBudget {
    pub elapsed_ms: u64,
    pub timeout_ms: Option<u64>,
    pub timed_out: bool,
    pub near_expiry: bool,
}

impl StepTimeBudget {
    pub fn badge_label(&self) -> String {
        let elapsed = fmt_duration(self.elapsed_ms);
        match (self.timed_out, self.timeout_ms, self.near_expiry) {
            (true, Some(t), _) => format!("TIMED OUT {elapsed}/{}", fmt_duration(t)),
            (true, None, _) => format!("TIMED OUT {elapsed}"),
            (false, Some(t), true) => format!(
                "⚠ {elapsed}/{} ({}%)",
                fmt_duration(t),
                (self.elapsed_ms.saturating_mul(100) / t.max(1)).min(100)
            ),
            (false, Some(t), false) => format!("{elapsed}/{}", fmt_duration(t)),
            (false, None, _) => elapsed,
        }
    }
}

/// (e) Executable (non-transition) steps used and remaining against the root workflow's `maxSteps`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepBudgetSummary {
    pub used_steps: usize,
    pub max_steps: Option<usize>,
    pub remaining_steps: Option<usize>,
}

impl StepBudgetSummary {
    /// - When `max_steps == Some(max)`: `Some(format!("steps {}/{max} ({} left)", self.used_steps, self.remaining_steps.unwrap_or(0)))`.
    /// - When `max_steps == None` and `self.used_steps > 0`: `Some(format!("steps {}/?", self.used_steps))`.
    /// - When `max_steps == None` and `self.used_steps == 0`: `None`.
    pub fn badge_label(&self) -> Option<String> {
        match self.max_steps {
            Some(max) => Some(format!(
                "steps {}/{max} ({} left)",
                self.used_steps,
                self.remaining_steps.unwrap_or(0)
            )),
            None if self.used_steps > 0 => Some(format!("steps {}/?", self.used_steps)),
            None => None,
        }
    }
}

/// (f) Summary of an included child workflow, grouped by `FlowNode::mount_path` / `FlowMount::mount_path` (joined with "/").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncludeScopeSummary {
    pub mount_path: String,
    pub invocations: usize,
    pub executed_child_nodes: usize,
    pub total_child_nodes: usize,
    pub used_steps: usize,
    pub max_steps: Option<usize>,
    pub remaining_steps: Option<usize>,
    pub active: bool,
    pub named_exit: Option<String>,
}

impl IncludeScopeSummary {
    pub fn summary_line(&self) -> String {
        let mut line = format!(
            "{}: {}/{} nodes",
            self.mount_path, self.executed_child_nodes, self.total_child_nodes
        );
        if let Some(max) = self.max_steps {
            line.push_str(&format!(
                " · {}/{max} steps ({} left)",
                self.used_steps,
                self.remaining_steps.unwrap_or(0)
            ));
        } else {
            line.push_str(&format!(" · {} steps", self.used_steps));
        }
        if self.invocations > 1 {
            line.push_str(&format!(" · {}x", self.invocations));
        }
        if self.active {
            line.push_str(" · active");
        } else if let Some(exit) = &self.named_exit {
            line.push_str(&format!(" · exit:{exit}"));
        }
        line
    }
}

pub fn non_zero_exit_code(step: &Step) -> Option<i64> {
    step.action
        .as_ref()
        .and_then(|action| action.exit_code)
        .filter(|&code| code != 0)
        .or_else(|| {
            step.output
                .get("exitCode")
                .and_then(Value::as_i64)
                .filter(|&code| code != 0)
        })
}

pub fn effective_outcome(step: &Step) -> Outcome {
    if step.outcome != Outcome::Ok {
        return step.outcome;
    }
    if non_zero_exit_code(step).is_some()
        || step.output.get("ok").and_then(Value::as_bool) == Some(false)
    {
        Outcome::Failed
    } else {
        Outcome::Ok
    }
}

pub fn trim_failure_tail(
    error: Option<&str>,
    stderr: Option<&str>,
    output_tail: Option<&str>,
    stdout: Option<&str>,
    exit_code: Option<i64>,
    max_lines: usize,
    max_chars: usize,
) -> Option<String> {
    if max_lines == 0 || max_chars == 0 {
        return None;
    }

    let source = [error, stderr, output_tail, stdout]
        .into_iter()
        .flatten()
        .find(|text| !text.trim().is_empty());

    if let Some(text) = source {
        let non_empty_lines: Vec<&str> = text
            .lines()
            .map(|line| line.trim_end_matches('\r').trim_end())
            .filter(|line| !line.trim().is_empty())
            .collect();

        if !non_empty_lines.is_empty() {
            let start = non_empty_lines.len().saturating_sub(max_lines);
            let joined = non_empty_lines.get(start..).unwrap_or_default().join("\n");
            return Some(truncate_chars(&joined, max_chars));
        }
    }

    exit_code
        .filter(|&code| code != 0)
        .map(|code| truncate_chars(&format!("exit code {code}"), max_chars))
}

pub fn step_failure_reason(step: &Step) -> Option<String> {
    let outcome = effective_outcome(step);
    if outcome == Outcome::Ok {
        return None;
    }

    let error = step
        .error
        .as_deref()
        .or_else(|| step.output.get("error").and_then(Value::as_str));
    let stderr = step.output.get("stderr").and_then(Value::as_str);
    let output_tail = step
        .output
        .get("outputTail")
        .or_else(|| step.output.get("output_tail"))
        .or_else(|| step.output.get("tail"))
        .or_else(|| step.output.get("message"))
        .and_then(Value::as_str)
        .or(step.raw_text.as_deref());
    let stdout = step.output.get("stdout").and_then(Value::as_str);
    let exit_code = non_zero_exit_code(step);

    if let Some(trimmed) = trim_failure_tail(
        error,
        stderr,
        output_tail,
        stdout,
        exit_code,
        DEFAULT_FAILURE_MAX_LINES,
        DEFAULT_FAILURE_MAX_CHARS,
    ) {
        return Some(trimmed);
    }

    match outcome {
        Outcome::TimedOut => Some(format!("timed out after {}", fmt_duration(step.duration_ms()))),
        Outcome::Cancelled => Some("cancelled".to_string()),
        _ => None,
    }
}

pub fn latest_run_failure(run: &Run) -> Option<(String, Option<i64>, String)> {
    for step in run.steps.iter().rev() {
        let outcome = effective_outcome(step);
        if matches!(outcome, Outcome::Failed | Outcome::TimedOut) {
            let exit_code = non_zero_exit_code(step);
            let reason = step_failure_reason(step).unwrap_or_else(|| match outcome {
                Outcome::TimedOut => {
                    format!("timed out after {}", fmt_duration(step.duration_ms()))
                }
                _ => "step failed".to_string(),
            });
            return Some((step.node_id.clone(), exit_code, reason));
        }
    }

    if let Some(err) = run.summary.error.as_deref()
        && let Some(trimmed) = trim_failure_tail(
            Some(err),
            None,
            None,
            None,
            None,
            DEFAULT_FAILURE_MAX_LINES,
            DEFAULT_FAILURE_MAX_CHARS,
        )
    {
        let node_id = run
            .summary
            .current_node
            .clone()
            .unwrap_or_else(|| run.flow.name.clone());
        return Some((node_id, None, trimmed));
    }

    if run.summary.status == RunStatus::TimedOut {
        let node_id = run
            .summary
            .current_node
            .clone()
            .unwrap_or_else(|| run.flow.name.clone());
        return Some((node_id, None, "run timed out".to_string()));
    }

    None
}

pub fn active_branch_nodes(
    start_at: &str,
    node_ids: &[String],
    edges: &[(String, String)],
    visited: &HashSet<String>,
) -> HashSet<String> {
    let mut reachable = HashSet::new();
    if start_at.is_empty() {
        return reachable;
    }

    let known: HashSet<&str> = node_ids.iter().map(String::as_str).collect();
    if !known.is_empty() && !known.contains(start_at) {
        return reachable;
    }

    let mut outgoing: HashMap<&str, Vec<&str>> = HashMap::new();
    for (from, to) in edges {
        if known.is_empty() || known.contains(to.as_str()) {
            outgoing.entry(from.as_str()).or_default().push(to.as_str());
        }
    }

    let mut queue = VecDeque::new();
    reachable.insert(start_at.to_string());
    queue.push_back(start_at);

    while let Some(current) = queue.pop_front() {
        let Some(targets) = outgoing.get(current) else {
            continue;
        };

        let any_target_visited = targets.len() > 1 && targets.iter().any(|t| visited.contains(*t));
        for &target in targets {
            if any_target_visited && !visited.contains(target) {
                continue;
            }
            if reachable.insert(target.to_string()) {
                queue.push_back(target);
            }
        }
    }

    reachable
}

pub fn extract_step_validations(
    run: &Run,
    step: &Step,
    observed_tools: &[&str],
) -> ValidationDigest {
    extract_step_validations_with_outcome(run, step, observed_tools, effective_outcome(step))
}

pub fn step_attempt_budget(run: &Run, step: &Step) -> NodeAttemptBudget {
    let mut observed_attempts = 0usize;
    let mut attempt = None;

    for candidate in &run.steps {
        if candidate.node_id == step.node_id {
            observed_attempts += 1;
            if attempt.is_none() && candidate.attempt_id == step.attempt_id {
                attempt = Some(observed_attempts);
            }
        }
    }

    NodeAttemptBudget {
        attempt: attempt.unwrap_or(observed_attempts + 1),
        observed_attempts,
    }
}

pub fn extract_step_verdict(run: &Run, step_index: usize) -> Option<StepVerdict> {
    let step = run.steps.get(step_index)?;
    let reason = extract_verdict_reason(step);
    let next_node_id = run.steps.get(step_index + 1).map(|s| s.node_id.as_str());

    let switch_edge = run.flow.edges.iter().find_map(|edge| match edge {
        FlowEdge::Switch { from, on, cases } if from == &step.node_id => Some((on, cases)),
        _ => None,
    });

    if let Some((on, cases)) = switch_edge {
        if let Some(route_key) = resolve_output_key(&step.output, on) {
            let target_node = match_switch_target(cases, &route_key, next_node_id);
            return Some(StepVerdict {
                route_key,
                target_node,
                reason,
            });
        }

        if let Some(next_id) = next_node_id
            && let Some((case_key, target)) = cases.iter().find(|(_, target)| target == next_id)
        {
            return Some(StepVerdict {
                route_key: case_key.clone(),
                target_node: Some(target.clone()),
                reason,
            });
        }
    }

    let explicit_key = ["verdict", "decision"].iter().find_map(|key| {
        step.output
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    })?;

    let target_node = switch_edge
        .and_then(|(_, cases)| match_switch_target(cases, &explicit_key, next_node_id));

    Some(StepVerdict {
        route_key: explicit_key,
        target_node,
        reason,
    })
}

pub fn step_time_budget(run: &Run, step: &Step, now_ms: i64) -> StepTimeBudget {
    let is_live_step = run.summary.status == RunStatus::Running
        && run.summary.current_node.as_deref() == Some(step.node_id.as_str())
        && step.finished_ms <= step.started_ms
        && now_ms > step.started_ms;

    let elapsed_ms = if is_live_step {
        (now_ms - step.started_ms) as u64
    } else {
        step.duration_ms()
    };

    let timeout_ms = run
        .flow
        .nodes
        .iter()
        .find(|node| node.id == step.node_id)
        .and_then(|node| node.timeout_ms);

    let timed_out = effective_outcome(step) == Outcome::TimedOut;
    let near_expiry = !timed_out
        && timeout_ms.is_some_and(|t| t > 0 && elapsed_ms.saturating_mul(100) / t >= 75);

    StepTimeBudget {
        elapsed_ms,
        timeout_ms,
        timed_out,
        near_expiry,
    }
}

pub fn run_step_budget(run: &Run) -> StepBudgetSummary {
    let used_steps = run
        .steps
        .iter()
        .filter(|step| !is_include_transition_step(run, &step.node_id))
        .count();
    let max_steps = run.flow.max_steps;
    let remaining_steps = max_steps.map(|max| max.saturating_sub(used_steps));

    StepBudgetSummary {
        used_steps,
        max_steps,
        remaining_steps,
    }
}

pub fn parse_scoped_node_id(node_id: &str) -> (Option<&str>, &str, Option<&str>) {
    let (mount, local) = match node_id.rsplit_once('/') {
        Some((m, l)) if !m.is_empty() => (Some(m), l),
        Some((_, l)) => (None, l),
        None => (None, node_id),
    };
    let exit = local
        .strip_prefix("__piw_exit_")
        .filter(|name| !name.is_empty());
    (mount, local, exit)
}

pub fn summarize_included_workflows(run: &Run) -> Vec<IncludeScopeSummary> {
    let mut grouped: BTreeMap<String, Vec<&FlowNode>> = BTreeMap::new();
    for node in &run.flow.nodes {
        if !node.mount_path.is_empty() {
            grouped
                .entry(node.mount_path.join("/"))
                .or_default()
                .push(node);
        }
    }

    let mut summaries = Vec::with_capacity(grouped.len());
    for (mount_key, nodes) in grouped {
        let max_steps = run
            .flow
            .mounts
            .iter()
            .find(|mount| mount.mount_path.join("/") == mount_key)
            .and_then(|mount| mount.max_steps);

        let group_node_ids: HashSet<&str> = nodes.iter().map(|node| node.id.as_str()).collect();
        let entry_node_ids: HashSet<&str> = nodes
            .iter()
            .filter(|node| node.include_transition.as_deref() == Some("entry"))
            .map(|node| node.id.as_str())
            .collect();
        let exit_nodes_by_id: HashMap<&str, &FlowNode> = nodes
            .iter()
            .filter(|node| node.include_transition.as_deref() == Some("exit"))
            .map(|node| (node.id.as_str(), *node))
            .collect();
        let transition_node_ids: HashSet<&str> = nodes
            .iter()
            .filter(|node| is_group_transition_node(node, &mount_key))
            .map(|node| node.id.as_str())
            .collect();

        let total_child_nodes = nodes.len();
        let executed_distinct: HashSet<&str> = run
            .steps
            .iter()
            .map(|step| step.node_id.as_str())
            .filter(|id| group_node_ids.contains(id))
            .collect();
        let executed_child_nodes = executed_distinct.len();

        let entry_step_count = run
            .steps
            .iter()
            .filter(|step| entry_node_ids.contains(step.node_id.as_str()))
            .count();
        let invocations = if !entry_node_ids.is_empty() {
            entry_step_count
        } else if executed_child_nodes > 0 {
            1
        } else {
            0
        };

        let last_entry_index = run.steps.iter().rposition(|step| {
            entry_node_ids.contains(step.node_id.as_str()) || step.node_id == mount_key
        });
        let scope_steps = match last_entry_index {
            Some(idx) => run.steps.get(idx + 1..).unwrap_or_default(),
            None => run.steps.as_slice(),
        };

        let mount_prefix = format!("{mount_key}/");
        let used_steps = scope_steps
            .iter()
            .filter(|step| {
                let in_scope = step.node_id.starts_with(&mount_prefix)
                    || group_node_ids.contains(step.node_id.as_str());
                in_scope
                    && !transition_node_ids.contains(step.node_id.as_str())
                    && !is_include_transition_step(run, &step.node_id)
            })
            .count();
        let remaining_steps = max_steps.map(|max| max.saturating_sub(used_steps));

        let active = run
            .summary
            .current_node
            .as_deref()
            .is_some_and(|current| group_node_ids.contains(current));

        let named_exit = run.steps.iter().rev().find_map(|step| {
            let node = exit_nodes_by_id.get(step.node_id.as_str())?;
            node.local_node_id
                .clone()
                .filter(|name| !name.is_empty())
                .or_else(|| parse_scoped_node_id(&node.id).2.map(str::to_string))
        });

        summaries.push(IncludeScopeSummary {
            mount_path: mount_key,
            invocations,
            executed_child_nodes,
            total_child_nodes,
            used_steps,
            max_steps,
            remaining_steps,
            active,
            named_exit,
        });
    }

    summaries
}

fn is_group_transition_node(node: &FlowNode, mount_key: &str) -> bool {
    matches!(
        node.include_transition.as_deref(),
        Some("entry") | Some("exit")
    ) || node.id == mount_key
        || parse_scoped_node_id(&node.id).2.is_some()
}

fn is_include_transition_step(run: &Run, node_id: &str) -> bool {
    if let Some(node) = run.flow.nodes.iter().find(|node| node.id == node_id)
        && matches!(
            node.include_transition.as_deref(),
            Some("entry") | Some("exit")
        )
    {
        return true;
    }
    if parse_scoped_node_id(node_id).2.is_some() {
        return true;
    }
    run.flow
        .mounts
        .iter()
        .any(|mount| !mount.mount_path.is_empty() && mount.mount_path.join("/") == node_id)
}

fn resolve_output_key(output: &Value, expr: &str) -> Option<String> {
    let path = expr
        .trim()
        .strip_prefix("$.")
        .or_else(|| expr.trim().strip_prefix('$'))
        .unwrap_or(expr.trim())
        .trim_start_matches('.');
    if path.is_empty() {
        return None;
    }

    let mut current = output;
    for segment in path.split('.') {
        if segment.is_empty() {
            return None;
        }
        current = current.get(segment)?;
    }

    match current {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Number(num) => Some(num.to_string()),
        _ => None,
    }
}

fn match_switch_target(
    cases: &[(String, String)],
    route_key: &str,
    next_node_id: Option<&str>,
) -> Option<String> {
    cases
        .iter()
        .find(|(case_key, _)| case_key == route_key)
        .or_else(|| {
            cases
                .iter()
                .find(|(case_key, _)| case_key == "default" || case_key == "*")
        })
        .map(|(_, target)| target.clone())
        .or_else(|| {
            let next_id = next_node_id?;
            cases
                .iter()
                .find(|(_, target)| target == next_id)
                .map(|(_, target)| target.clone())
        })
}

fn extract_verdict_reason(step: &Step) -> Option<String> {
    for &key in VERDICT_REASON_KEYS {
        if step.kind == NodeKind::Checkpoint && key == "summary" {
            continue;
        }
        let Some(raw) = step.output.get(key).and_then(Value::as_str) else {
            continue;
        };
        let collapsed = collapse_single_line(raw);
        if !collapsed.is_empty() {
            return Some(truncate_chars(&collapsed, VERDICT_REASON_MAX_CHARS));
        }
    }
    None
}

fn collapse_single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut chars = text.chars();
    let prefix: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_none() {
        return prefix;
    }
    if max_chars == 1 {
        return "…".to_string();
    }
    let mut shortened: String = prefix.chars().take(max_chars - 1).collect();
    shortened.push('…');
    shortened
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;
    use crate::acpx::bundle::{ActionReceipt, FlowDef, FlowMount, RunSummary};

    fn make_node(id: &str, kind: NodeKind) -> FlowNode {
        FlowNode {
            id: id.to_string(),
            kind,
            summary: None,
            is_shell: false,
            timeout_ms: None,
            mount_path: Vec::new(),
            local_node_id: None,
            include_transition: None,
        }
    }

    fn make_step(attempt_id: &str, node_id: &str, kind: NodeKind, outcome: Outcome) -> Step {
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

    fn make_run(nodes: Vec<FlowNode>, edges: Vec<FlowEdge>, steps: Vec<Step>) -> Run {
        Run {
            summary: RunSummary {
                run_id: "run-1".to_string(),
                dir: PathBuf::from("/tmp/run-1"),
                flow_name: "demo".to_string(),
                title: "Demo".to_string(),
                status: RunStatus::Completed,
                started_ms: 0,
                updated_ms: 10_000,
                finished_ms: Some(10_000),
                current_node: None,
                waiting_on: None,
                status_detail: None,
                error: None,
            },
            flow: FlowDef {
                name: "demo".to_string(),
                start_at: nodes.first().map(|n| n.id.clone()).unwrap_or_default(),
                nodes,
                edges,
                max_steps: None,
                mounts: Vec::new(),
            },
            steps,
            events: Vec::new(),
            input: Value::Null,
            sessions: Vec::new(),
        }
    }

    #[test]
    fn effective_outcome_catches_non_zero_exit_and_preserves_timed_out() {
        let mut failed_shell = make_step("u1", "cargo_check", NodeKind::Action, Outcome::Ok);
        failed_shell.action = Some(ActionReceipt {
            action_type: "shell".to_string(),
            command: Some("cargo".to_string()),
            args: vec!["check".to_string()],
            cwd: None,
            exit_code: Some(101),
            duration_ms: Some(500),
        });
        assert_eq!(non_zero_exit_code(&failed_shell), Some(101));
        assert_eq!(effective_outcome(&failed_shell), Outcome::Failed);

        let mut output_not_ok = make_step("u2", "check", NodeKind::Compute, Outcome::Ok);
        output_not_ok.output = json!({ "ok": false });
        assert_eq!(effective_outcome(&output_not_ok), Outcome::Failed);

        let mut timed_out = make_step("u3", "slow_step", NodeKind::Acp, Outcome::TimedOut);
        timed_out.started_ms = 0;
        timed_out.finished_ms = 5_000;
        assert_eq!(effective_outcome(&timed_out), Outcome::TimedOut);
        assert_eq!(
            step_failure_reason(&timed_out),
            Some("timed out after 5.0s".to_string())
        );

        let ok_step = make_step("u4", "good", NodeKind::Acp, Outcome::Ok);
        assert_eq!(effective_outcome(&ok_step), Outcome::Ok);
        assert_eq!(step_failure_reason(&ok_step), None);
    }

    #[test]
    fn trim_failure_tail_prefers_error_then_stderr_and_respects_limits() {
        let trimmed = trim_failure_tail(
            None,
            Some("line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\n"),
            Some("ignored"),
            None,
            Some(1),
            3,
            100,
        );
        assert_eq!(trimmed, Some("line 5\nline 6\nline 7".to_string()));

        let fallback = trim_failure_tail(Some("   "), None, None, None, Some(101), 6, 480);
        assert_eq!(fallback, Some("exit code 101".to_string()));

        let char_capped = trim_failure_tail(Some("abcdefghij"), None, None, None, None, 6, 5);
        assert_eq!(char_capped, Some("abcd…".to_string()));
    }

    #[test]
    fn active_branch_nodes_drops_untaken_branch_after_decision() {
        let node_ids = vec![
            "route_host".to_string(),
            "resolve_group1_air".to_string(),
            "resolve_group2_shell".to_string(),
        ];
        let edges = vec![
            ("route_host".to_string(), "resolve_group1_air".to_string()),
            ("route_host".to_string(), "resolve_group2_shell".to_string()),
        ];

        let before_decision: HashSet<String> = ["route_host".to_string()].into_iter().collect();
        let kept_before = active_branch_nodes("route_host", &node_ids, &edges, &before_decision);
        assert!(kept_before.contains("route_host"));
        assert!(kept_before.contains("resolve_group1_air"));
        assert!(kept_before.contains("resolve_group2_shell"));

        let after_decision: HashSet<String> = [
            "route_host".to_string(),
            "resolve_group2_shell".to_string(),
        ]
        .into_iter()
        .collect();
        let kept_after = active_branch_nodes("route_host", &node_ids, &edges, &after_decision);
        assert!(kept_after.contains("route_host"));
        assert!(kept_after.contains("resolve_group2_shell"));
        assert!(!kept_after.contains("resolve_group1_air"));
    }

    #[test]
    fn step_attempt_budget_counts_uuid_attempts() {
        let s1 = make_step(
            "550e8400-e29b-41d4-a716-446655440000",
            "cargo_check",
            NodeKind::Action,
            Outcome::Failed,
        );
        let s2 = make_step(
            "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
            "cargo_check",
            NodeKind::Action,
            Outcome::Ok,
        );
        let run = make_run(
            vec![make_node("cargo_check", NodeKind::Action)],
            Vec::new(),
            vec![s1.clone(), s2.clone()],
        );

        let first = step_attempt_budget(&run, &s1);
        assert_eq!(
            first,
            NodeAttemptBudget {
                attempt: 1,
                observed_attempts: 2
            }
        );
        assert_eq!(first.badge_label(), Some("attempt 1".to_string()));

        let second = step_attempt_budget(&run, &s2);
        assert_eq!(
            second,
            NodeAttemptBudget {
                attempt: 2,
                observed_attempts: 2
            }
        );
        assert_eq!(second.badge_label(), Some("attempt 2".to_string()));

        let solo_step = make_step("solo-uuid", "solo", NodeKind::Acp, Outcome::Ok);
        let solo_run = make_run(
            vec![make_node("solo", NodeKind::Acp)],
            Vec::new(),
            vec![solo_step.clone()],
        );
        assert_eq!(step_attempt_budget(&solo_run, &solo_step).badge_label(), None);
    }

    #[test]
    fn extract_step_verdict_reads_switch_output_and_reason() {
        let mut review = make_step("a1", "review", NodeKind::Acp, Outcome::Ok);
        review.output = json!({
            "verdict": "needs_work",
            "reason": "Missing test coverage\nfor timeout handling"
        });
        let repair = make_step("a2", "repair", NodeKind::Acp, Outcome::Ok);

        let run = make_run(
            vec![
                make_node("review", NodeKind::Acp),
                make_node("repair", NodeKind::Acp),
                make_node("done", NodeKind::Compute),
            ],
            vec![FlowEdge::Switch {
                from: "review".to_string(),
                on: "$.verdict".to_string(),
                cases: vec![
                    ("approved".to_string(), "done".to_string()),
                    ("needs_work".to_string(), "repair".to_string()),
                ],
            }],
            vec![review, repair],
        );

        let verdict = extract_step_verdict(&run, 0).expect("verdict should be extracted");
        assert_eq!(verdict.route_key, "needs_work");
        assert_eq!(verdict.target_node.as_deref(), Some("repair"));
        assert_eq!(
            verdict.reason.as_deref(),
            Some("Missing test coverage for timeout handling")
        );
        assert_eq!(
            verdict.summary_line(),
            "verdict: needs_work -> repair (Missing test coverage for timeout handling)"
        );

        // Checkpoint nodes skip "summary" when extracting reason.
        let mut gate = make_step("g1", "approve", NodeKind::Checkpoint, Outcome::Ok);
        gate.output = json!({
            "decision": "proceed",
            "summary": "gate prompt summary",
            "rationale": "looks good"
        });
        let gate_run = make_run(
            vec![make_node("approve", NodeKind::Checkpoint)],
            Vec::new(),
            vec![gate],
        );
        let gate_verdict = extract_step_verdict(&gate_run, 0).expect("checkpoint verdict");
        assert_eq!(gate_verdict.route_key, "proceed");
        assert_eq!(gate_verdict.reason.as_deref(), Some("looks good"));
    }

    #[test]
    fn step_time_budget_handles_near_expiry_timeout_and_no_timeout() {
        let mut timed_node = make_node("build", NodeKind::Action);
        timed_node.timeout_ms = Some(120_000);
        let untimed_node = make_node("notes", NodeKind::Acp);

        let mut near_step = make_step("s1", "build", NodeKind::Action, Outcome::Ok);
        near_step.started_ms = 0;
        near_step.finished_ms = 90_000;

        let mut timed_out_step = make_step("s2", "build", NodeKind::Action, Outcome::TimedOut);
        timed_out_step.started_ms = 0;
        timed_out_step.finished_ms = 120_000;

        let mut untimed_step = make_step("s3", "notes", NodeKind::Acp, Outcome::Ok);
        untimed_step.started_ms = 0;
        untimed_step.finished_ms = 4_500;

        let run = make_run(
            vec![timed_node, untimed_node],
            Vec::new(),
            vec![
                near_step.clone(),
                timed_out_step.clone(),
                untimed_step.clone(),
            ],
        );

        let near = step_time_budget(&run, &near_step, 100_000);
        assert_eq!(near.elapsed_ms, 90_000);
        assert_eq!(near.timeout_ms, Some(120_000));
        assert!(!near.timed_out);
        assert!(near.near_expiry);
        assert_eq!(near.badge_label(), "⚠ 1m30s/2m00s (75%)");

        let expired = step_time_budget(&run, &timed_out_step, 130_000);
        assert!(expired.timed_out);
        assert!(!expired.near_expiry);
        assert_eq!(expired.badge_label(), "TIMED OUT 2m00s/2m00s");

        let untimed = step_time_budget(&run, &untimed_step, 10_000);
        assert_eq!(untimed.timeout_ms, None);
        assert!(!untimed.timed_out);
        assert!(!untimed.near_expiry);
        assert_eq!(untimed.badge_label(), "4.5s");
    }

    #[test]
    fn parse_scoped_node_id_extracts_mount_local_and_exit() {
        assert_eq!(
            parse_scoped_node_id("verify/__piw_exit_passed"),
            (Some("verify"), "__piw_exit_passed", Some("passed"))
        );
        assert_eq!(
            parse_scoped_node_id("outer/inner/run_tests"),
            (Some("outer/inner"), "run_tests", None)
        );
        assert_eq!(parse_scoped_node_id("plan"), (None, "plan", None));
    }

    #[test]
    fn run_step_budget_and_included_workflows_respect_caps_and_transitions() {
        let plan = make_node("plan", NodeKind::Acp);
        let implement = make_node("implement", NodeKind::Acp);

        let mut verify_entry = make_node("verify", NodeKind::Compute);
        verify_entry.mount_path = vec!["verify".to_string()];
        verify_entry.local_node_id = Some("verify".to_string());
        verify_entry.include_transition = Some("entry".to_string());

        let mut verify_lint = make_node("verify/lint", NodeKind::Action);
        verify_lint.mount_path = vec!["verify".to_string()];
        verify_lint.local_node_id = Some("lint".to_string());

        let mut verify_test = make_node("verify/test", NodeKind::Action);
        verify_test.mount_path = vec!["verify".to_string()];
        verify_test.local_node_id = Some("test".to_string());

        let mut verify_exit = make_node("verify/__piw_exit_passed", NodeKind::Compute);
        verify_exit.mount_path = vec!["verify".to_string()];
        verify_exit.local_node_id = Some("passed".to_string());
        verify_exit.include_transition = Some("exit".to_string());

        let mut approval_entry = make_node("approval", NodeKind::Compute);
        approval_entry.mount_path = vec!["approval".to_string()];
        approval_entry.local_node_id = Some("approval".to_string());
        approval_entry.include_transition = Some("entry".to_string());

        let mut approval_gate = make_node("approval/gate", NodeKind::Checkpoint);
        approval_gate.mount_path = vec!["approval".to_string()];
        approval_gate.local_node_id = Some("gate".to_string());

        let mut approval_exit = make_node("approval/__piw_exit_approved", NodeKind::Compute);
        approval_exit.mount_path = vec!["approval".to_string()];
        approval_exit.local_node_id = Some("approved".to_string());
        approval_exit.include_transition = Some("exit".to_string());

        let steps = vec![
            make_step("s1", "plan", NodeKind::Acp, Outcome::Ok),
            make_step("s2", "approval", NodeKind::Compute, Outcome::Ok),
            make_step("s3", "approval/gate", NodeKind::Checkpoint, Outcome::Ok),
            make_step(
                "s4",
                "approval/__piw_exit_approved",
                NodeKind::Compute,
                Outcome::Ok,
            ),
            make_step("s5", "implement", NodeKind::Acp, Outcome::Ok),
            make_step("s6", "verify", NodeKind::Compute, Outcome::Ok),
            make_step("s7", "verify/lint", NodeKind::Action, Outcome::Ok),
            make_step("s8", "verify/test", NodeKind::Action, Outcome::Ok),
            make_step(
                "s9",
                "verify/__piw_exit_passed",
                NodeKind::Compute,
                Outcome::Ok,
            ),
        ];

        let mut run = make_run(
            vec![
                plan,
                approval_entry,
                approval_gate,
                approval_exit,
                implement,
                verify_entry,
                verify_lint,
                verify_test,
                verify_exit,
            ],
            Vec::new(),
            steps,
        );
        run.flow.max_steps = Some(320);
        run.flow.mounts = vec![
            FlowMount {
                mount_path: vec!["verify".to_string()],
                workflow_name: "change-verification".to_string(),
                max_steps: Some(30),
            },
            FlowMount {
                mount_path: vec!["approval".to_string()],
                workflow_name: "plan-approval".to_string(),
                max_steps: None,
            },
        ];

        // 1. Root budget excludes the 4 entry/exit transition steps, leaving 5 executable steps.
        let root_budget = run_step_budget(&run);
        assert_eq!(
            root_budget,
            StepBudgetSummary {
                used_steps: 5,
                max_steps: Some(320),
                remaining_steps: Some(315),
            }
        );
        assert_eq!(
            root_budget.badge_label(),
            Some("steps 5/320 (315 left)".to_string())
        );

        let includes = summarize_included_workflows(&run);
        assert_eq!(includes.len(), 2);

        // 3. Uncapped mount ("approval" comes before "verify" in BTreeMap order).
        let approval = &includes[0];
        assert_eq!(approval.mount_path, "approval");
        assert_eq!(approval.used_steps, 1);
        assert_eq!(approval.max_steps, None);
        assert_eq!(approval.remaining_steps, None);
        assert_eq!(approval.named_exit.as_deref(), Some("approved"));
        assert_eq!(
            approval.summary_line(),
            "approval: 3/3 nodes · 1 steps · exit:approved"
        );
        assert!(!approval.summary_line().contains("left"));

        // 2. Capped mount ("verify").
        let verify = &includes[1];
        assert_eq!(verify.mount_path, "verify");
        assert_eq!(verify.used_steps, 2);
        assert_eq!(verify.max_steps, Some(30));
        assert_eq!(verify.remaining_steps, Some(28));
        assert_eq!(verify.named_exit.as_deref(), Some("passed"));
        assert_eq!(
            verify.summary_line(),
            "verify: 4/4 nodes · 2/30 steps (28 left) · exit:passed"
        );
    }
}
