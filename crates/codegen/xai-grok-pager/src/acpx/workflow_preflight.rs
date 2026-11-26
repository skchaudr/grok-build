use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::acpx::bundle::{FlowDef, FlowEdge, Outcome, Run, RunStatus, fmt_duration};
use crate::acpx::step_digest::effective_outcome;

const TIMEOUT_ROUND_STEP_MS: u64 = 30_000;
const MIN_RECOMMENDED_TIMEOUT_MS: u64 = 60_000;

/// Historical duration and failure statistics for one node across recorded runs of the same workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepHistoryStats {
    pub node_id: String,
    pub executions: usize,
    pub failures: usize,
    pub timeouts: usize,
    pub min_duration_ms: u64,
    pub median_duration_ms: u64,
    pub max_duration_ms: u64,
    /// History-backed recommended `timeoutMs`: `max_duration_ms.saturating_mul(2)` rounded up to the next 30_000ms
    /// (minimum 60_000ms), or `None` when `executions == 0`.
    pub recommended_timeout_ms: Option<u64>,
}

/// One reusable workflow template entry discovered from recorded runs and workflow files on disk
/// (the v1 entry point for creating or cloning a workflow from past work).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowTemplateEntry {
    pub flow_name: String,
    pub source_path: Option<PathBuf>,
    pub node_count: usize,
    pub has_loops: bool,
    pub recorded_runs: usize,
    pub completed_runs: usize,
    pub failed_runs: usize,
    pub last_run_id: Option<String>,
    pub median_run_duration_ms: Option<u64>,
}

/// User-supplied or soft-prompted preflight budget & loop overrides applied before launch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreflightOverrides {
    /// Per-node `timeoutMs` overrides (`node_id -> timeout_ms`).
    pub node_timeouts_ms: BTreeMap<String, u64>,
    /// Per-cycle loop attempt limits (`"<from>-><to>"` or `from` node id -> max attempts).
    pub loop_limits: BTreeMap<String, usize>,
    /// Root workflow `maxSteps` override.
    pub max_steps: Option<usize>,
    /// Free-form operator steering notes attached during preflight co-design.
    pub steering_notes: Vec<String>,
}

/// Specific preflight issue that blocks or warns before launching a workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreflightIssueKind {
    /// A back-edge `from -> to` forms a cycle with no loop attempt bound in the definition or `overrides.loop_limits`.
    UnboundedLoop {
        from: String,
        to: String,
    },
    /// A node has no `timeoutMs` in the definition or `overrides.node_timeouts_ms`.
    MissingNodeTimeout {
        node_id: String,
        suggested_ms: Option<u64>,
    },
    /// A node's configured `timeoutMs` is smaller than or within 10% of its historical `max_duration_ms` (likely a guess).
    UnderbudgetedTimeout {
        node_id: String,
        configured_ms: u64,
        historical_max_ms: u64,
        suggested_ms: u64,
    },
    /// Root workflow has no `maxSteps` in the definition or `overrides.max_steps`.
    MissingMaxSteps,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightIssue {
    /// True when this issue must be filled or explicitly accepted before launch (`UnboundedLoop`, `MissingNodeTimeout`, `MissingMaxSteps`).
    pub blocking: bool,
    pub kind: PreflightIssueKind,
    pub message: String,
}

/// Complete preflight audit for a workflow definition against past run records and current overrides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightReport {
    pub flow_name: String,
    pub node_count: usize,
    pub effective_max_steps: Option<usize>,
    /// Resolved `timeoutMs` per node (`node_id -> Option<u64>`).
    pub node_timeouts_ms: BTreeMap<String, Option<u64>>,
    /// Detected cycle back-edges `(from, to, resolved_limit)`.
    pub loop_edges: Vec<(String, String, Option<usize>)>,
    /// Per-node historical stats across `past_runs` of the same workflow.
    pub history: Vec<StepHistoryStats>,
    pub issues: Vec<PreflightIssue>,
    pub steering_notes: Vec<String>,
}

impl PreflightReport {
    /// True when no issue in `self.issues` has `blocking == true`.
    pub fn ready_to_launch(&self) -> bool {
        !self.issues.iter().any(|issue| issue.blocking)
    }
}

/// Analyze all executed steps across `past_runs` whose `run.summary.flow_name == flow_name` (or `run.flow.name == flow_name`),
/// returning `StepHistoryStats` sorted by `node_id` (or in `past_runs` first-appearance order).
/// Uses `crate::acpx::step_digest::effective_outcome(step)` so `allowFailure` steps with non-zero exit code count as failures!
pub fn analyze_workflow_history(flow_name: &str, past_runs: &[Run]) -> Vec<StepHistoryStats> {
    struct NodeAccum {
        durations: Vec<u64>,
        failures: usize,
        timeouts: usize,
    }

    let mut per_node: BTreeMap<String, NodeAccum> = BTreeMap::new();

    for run in past_runs {
        if run.summary.flow_name != flow_name && run.flow.name != flow_name {
            continue;
        }

        let transition_nodes: HashSet<&str> = run
            .flow
            .nodes
            .iter()
            .filter(|n| n.include_transition.is_some())
            .map(|n| n.id.as_str())
            .collect();

        for step in &run.steps {
            if transition_nodes.contains(step.node_id.as_str()) {
                continue;
            }

            let entry = per_node
                .entry(step.node_id.clone())
                .or_insert_with(|| NodeAccum {
                    durations: Vec::new(),
                    failures: 0,
                    timeouts: 0,
                });

            let duration_ms = step_duration_ms(step);
            entry.durations.push(duration_ms);

            match effective_outcome(step) {
                Outcome::Failed => entry.failures += 1,
                Outcome::TimedOut => entry.timeouts += 1,
                Outcome::Ok | Outcome::Cancelled | Outcome::Unknown => {}
            }
        }
    }

    per_node
        .into_iter()
        .map(|(node_id, mut accum)| {
            accum.durations.sort_unstable();
            let executions = accum.durations.len();
            let min_duration_ms = accum.durations.first().copied().unwrap_or(0);
            let max_duration_ms = accum.durations.last().copied().unwrap_or(0);
            let median_duration_ms = median_sorted_u64(&accum.durations);
            let recommended_timeout_ms = if executions == 0 {
                None
            } else {
                Some(compute_recommended_timeout_ms(max_duration_ms))
            };

            StepHistoryStats {
                node_id,
                executions,
                failures: accum.failures,
                timeouts: accum.timeouts,
                min_duration_ms,
                median_duration_ms,
                max_duration_ms,
                recommended_timeout_ms,
            }
        })
        .collect()
}

/// Build the catalog of workflow templates from `past_runs` and discovered `workflow_files` on disk.
pub fn discover_workflow_templates(
    past_runs: &[Run],
    workflow_files: &[PathBuf],
) -> Vec<WorkflowTemplateEntry> {
    struct TemplateAccum {
        source_path: Option<PathBuf>,
        node_count: usize,
        has_loops: bool,
        recorded_runs: usize,
        completed_runs: usize,
        failed_runs: usize,
        last_run_id: Option<String>,
        last_run_ts: Option<i64>,
        durations: Vec<u64>,
    }

    let mut entries: BTreeMap<String, TemplateAccum> = BTreeMap::new();

    for path in workflow_files {
        let Some(flow_name) = flow_name_from_path(path) else {
            continue;
        };
        let entry = entries.entry(flow_name).or_insert_with(|| TemplateAccum {
            source_path: None,
            node_count: 0,
            has_loops: false,
            recorded_runs: 0,
            completed_runs: 0,
            failed_runs: 0,
            last_run_id: None,
            last_run_ts: None,
            durations: Vec::new(),
        });
        if entry.source_path.is_none() {
            entry.source_path = Some(path.clone());
        }
    }

    for run in past_runs {
        let flow_name = effective_flow_name(run);
        if flow_name.is_empty() {
            continue;
        }
        let entry = entries
            .entry(flow_name.to_string())
            .or_insert_with(|| TemplateAccum {
                source_path: None,
                node_count: 0,
                has_loops: false,
                recorded_runs: 0,
                completed_runs: 0,
                failed_runs: 0,
                last_run_id: None,
                last_run_ts: None,
                durations: Vec::new(),
            });

        entry.recorded_runs += 1;
        match run.summary.status {
            RunStatus::Completed => entry.completed_runs += 1,
            RunStatus::Failed | RunStatus::TimedOut => entry.failed_runs += 1,
            RunStatus::Running | RunStatus::Waiting | RunStatus::Unknown => {}
        }

        let run_ts = run.summary.started_ms.max(run.summary.updated_ms);
        let is_newer = match entry.last_run_ts {
            None => true,
            Some(prev_ts) => run_ts >= prev_ts,
        };
        if is_newer {
            entry.last_run_ts = Some(run_ts);
            entry.last_run_id = Some(run.summary.run_id.clone());
            if !run.flow.nodes.is_empty() {
                entry.node_count = run.flow.nodes.len();
                entry.has_loops = !find_cycle_back_edges(&run.flow).is_empty();
            }
        } else if entry.node_count == 0 && !run.flow.nodes.is_empty() {
            entry.node_count = run.flow.nodes.len();
            entry.has_loops = !find_cycle_back_edges(&run.flow).is_empty();
        }

        entry
            .durations
            .push(run.duration_ms(run.summary.updated_ms));
    }

    entries
        .into_iter()
        .map(|(flow_name, mut accum)| {
            accum.durations.sort_unstable();
            let median_run_duration_ms = if accum.durations.is_empty() {
                None
            } else {
                Some(median_sorted_u64(&accum.durations))
            };
            WorkflowTemplateEntry {
                flow_name,
                source_path: accum.source_path,
                node_count: accum.node_count,
                has_loops: accum.has_loops,
                recorded_runs: accum.recorded_runs,
                completed_runs: accum.completed_runs,
                failed_runs: accum.failed_runs,
                last_run_id: accum.last_run_id,
                median_run_duration_ms,
            }
        })
        .collect()
}

/// Detect directed cycle back-edges `(from, to)` in `flow` using DFS from `flow.start_at` (plus any unvisited nodes in `flow.nodes` order).
pub fn find_cycle_back_edges(flow: &FlowDef) -> Vec<(String, String)> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum VisitState {
        Unvisited,
        Visiting,
        Visited,
    }

    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut edge_sources: Vec<&str> = Vec::new();
    let mut seen_edges: HashSet<(&str, &str)> = HashSet::new();

    for edge in &flow.edges {
        match edge {
            FlowEdge::Simple { from, to } => {
                if seen_edges.insert((from.as_str(), to.as_str())) {
                    adj.entry(from.as_str()).or_default().push(to.as_str());
                    edge_sources.push(from.as_str());
                }
            }
            FlowEdge::Switch { from, cases, .. } => {
                for (_, to) in cases {
                    if seen_edges.insert((from.as_str(), to.as_str())) {
                        adj.entry(from.as_str()).or_default().push(to.as_str());
                        edge_sources.push(from.as_str());
                    }
                }
            }
        }
    }

    let mut roots: Vec<&str> = Vec::new();
    if !flow.start_at.is_empty() {
        roots.push(flow.start_at.as_str());
    }
    for node in &flow.nodes {
        roots.push(node.id.as_str());
    }
    for src in edge_sources {
        roots.push(src);
    }

    let mut state: HashMap<&str, VisitState> = HashMap::new();
    let mut back_edges: Vec<(String, String)> = Vec::new();
    let mut seen_back_edges: HashSet<(String, String)> = HashSet::new();

    for root in roots {
        if state.get(root).copied().unwrap_or(VisitState::Unvisited) != VisitState::Unvisited {
            continue;
        }

        let mut stack: Vec<(&str, usize)> = vec![(root, 0)];
        state.insert(root, VisitState::Visiting);

        while let Some((current, next_idx)) = stack.last_mut() {
            let neighbors = adj.get(*current).map(Vec::as_slice).unwrap_or(&[]);
            if let Some(&target) = neighbors.get(*next_idx) {
                *next_idx += 1;
                match state.get(target).copied().unwrap_or(VisitState::Unvisited) {
                    VisitState::Visiting => {
                        let pair = ((*current).to_string(), target.to_string());
                        if seen_back_edges.insert(pair.clone()) {
                            back_edges.push(pair);
                        }
                    }
                    VisitState::Unvisited => {
                        state.insert(target, VisitState::Visiting);
                        stack.push((target, 0));
                    }
                    VisitState::Visited => {}
                }
            } else {
                let finished = *current;
                state.insert(finished, VisitState::Visited);
                stack.pop();
            }
        }
    }

    back_edges
}

/// Audit `flow` (reading `node.timeout_ms`, `flow.max_steps`, and optional `raw_flow_json` for `"maxSteps"`,
/// `"nodes.<id>.timeoutMs"`, `"nodes.<id>.maxAttempts"`) against `past_runs` and `overrides`.
pub fn audit_workflow_preflight(
    flow: &FlowDef,
    raw_flow_json: Option<&Value>,
    past_runs: &[Run],
    overrides: &PreflightOverrides,
) -> PreflightReport {
    let history = analyze_workflow_history(&flow.name, past_runs);
    let history_by_node: HashMap<&str, &StepHistoryStats> = history
        .iter()
        .map(|stats| (stats.node_id.as_str(), stats))
        .collect();

    let effective_max_steps = overrides
        .max_steps
        .filter(|&n| n > 0)
        .or_else(|| flow.max_steps.filter(|&n| n > 0))
        .or_else(|| raw_flow_json.and_then(json_max_steps));

    let mut issues: Vec<PreflightIssue> = Vec::new();

    if effective_max_steps.is_none() {
        issues.push(PreflightIssue {
            blocking: true,
            kind: PreflightIssueKind::MissingMaxSteps,
            message: format!(
                "Workflow `{}` has no root maxSteps limit configured.",
                flow.name
            ),
        });
    }

    let mut node_timeouts_ms: BTreeMap<String, Option<u64>> = BTreeMap::new();
    for node in &flow.nodes {
        if node.include_transition.is_some() {
            continue;
        }

        let resolved_timeout = overrides
            .node_timeouts_ms
            .get(&node.id)
            .copied()
            .filter(|&ms| ms > 0)
            .or_else(|| node.timeout_ms.filter(|&ms| ms > 0))
            .or_else(|| raw_flow_json.and_then(|v| json_node_timeout_ms(v, &node.id)));

        node_timeouts_ms.insert(node.id.clone(), resolved_timeout);

        let stats = history_by_node.get(node.id.as_str()).copied();
        match resolved_timeout {
            None => {
                let suggested_ms = stats.and_then(|s| s.recommended_timeout_ms);
                let message = match suggested_ms {
                    Some(ms) => format!(
                        "Node `{}` has no timeoutMs configured (history suggests {}).",
                        node.id,
                        fmt_duration(ms)
                    ),
                    None => format!("Node `{}` has no timeoutMs configured.", node.id),
                };
                issues.push(PreflightIssue {
                    blocking: true,
                    kind: PreflightIssueKind::MissingNodeTimeout {
                        node_id: node.id.clone(),
                        suggested_ms,
                    },
                    message,
                });
            }
            Some(configured_ms) => {
                if let Some(s) = stats
                    && s.executions > 0
                    && s.max_duration_ms > 0
                    && is_underbudgeted_timeout(configured_ms, s.max_duration_ms)
                {
                    let suggested_ms = s
                        .recommended_timeout_ms
                        .unwrap_or_else(|| compute_recommended_timeout_ms(s.max_duration_ms));
                    issues.push(PreflightIssue {
                        blocking: false,
                        kind: PreflightIssueKind::UnderbudgetedTimeout {
                            node_id: node.id.clone(),
                            configured_ms,
                            historical_max_ms: s.max_duration_ms,
                            suggested_ms,
                        },
                        message: format!(
                            "Node `{}` timeoutMs ({}) is within 10% of or below historical max ({}); consider {}.",
                            node.id,
                            fmt_duration(configured_ms),
                            fmt_duration(s.max_duration_ms),
                            fmt_duration(suggested_ms)
                        ),
                    });
                }
            }
        }
    }

    let back_edges = find_cycle_back_edges(flow);
    let mut loop_edges: Vec<(String, String, Option<usize>)> = Vec::with_capacity(back_edges.len());

    for (from, to) in back_edges {
        let edge_key = format!("{from}->{to}");
        let resolved_limit = overrides
            .loop_limits
            .get(&edge_key)
            .copied()
            .filter(|&n| n > 0)
            .or_else(|| overrides.loop_limits.get(&from).copied().filter(|&n| n > 0))
            .or_else(|| raw_flow_json.and_then(|v| json_node_max_attempts(v, &from)))
            .or_else(|| raw_flow_json.and_then(|v| json_node_max_attempts(v, &to)));

        if resolved_limit.is_none() {
            issues.push(PreflightIssue {
                blocking: true,
                kind: PreflightIssueKind::UnboundedLoop {
                    from: from.clone(),
                    to: to.clone(),
                },
                message: format!("Cycle `{from} -> {to}` has no loop attempt limit configured."),
            });
        }

        loop_edges.push((from, to, resolved_limit));
    }

    PreflightReport {
        flow_name: flow.name.clone(),
        node_count: flow.nodes.len(),
        effective_max_steps,
        node_timeouts_ms,
        loop_edges,
        history,
        issues,
        steering_notes: overrides.steering_notes.clone(),
    }
}

/// Format `report` as a concise Markdown preflight card for `grok acpx` chat (showing steps, historical vs configured
/// timeouts, loop bounds, `maxSteps`, blocking issues, and how to reply in plain text to set budgets or launch).
pub fn format_preflight_markdown(report: &PreflightReport) -> String {
    let mut out = String::new();
    let blocking_count = report.issues.iter().filter(|i| i.blocking).count();
    let warning_count = report.issues.len().saturating_sub(blocking_count);
    let status = if report.ready_to_launch() {
        "READY TO LAUNCH".to_string()
    } else {
        format!("BLOCKED ({blocking_count} blocking)")
    };

    let _ = writeln!(out, "## Workflow Preflight: `{}`", report.flow_name);
    let max_steps_str = match report.effective_max_steps {
        Some(n) => n.to_string(),
        None => "unset".to_string(),
    };
    let _ = writeln!(
        out,
        "- **Status**: {status} | **Nodes**: {} | **maxSteps**: {max_steps_str} | **Warnings**: {warning_count}",
        report.node_count
    );

    let history_by_node: HashMap<&str, &StepHistoryStats> = report
        .history
        .iter()
        .map(|s| (s.node_id.as_str(), s))
        .collect();

    if !report.node_timeouts_ms.is_empty() {
        let _ = writeln!(out, "\n### Steps & Timeouts");
        let _ = writeln!(
            out,
            "| Node | Configured | History (med / max, runs) | Suggested |"
        );
        let _ = writeln!(out, "| --- | --- | --- | --- |");

        for (node_id, configured) in &report.node_timeouts_ms {
            let configured_str = match configured {
                Some(ms) => fmt_duration(*ms),
                None => "unset".to_string(),
            };
            let (history_str, suggested_str) = match history_by_node.get(node_id.as_str()) {
                Some(stats) if stats.executions > 0 => {
                    let mut h = format!(
                        "{} / {} ({}x",
                        fmt_duration(stats.median_duration_ms),
                        fmt_duration(stats.max_duration_ms),
                        stats.executions
                    );
                    if stats.failures > 0 {
                        let _ = write!(h, ", {} fail", stats.failures);
                    }
                    if stats.timeouts > 0 {
                        let _ = write!(h, ", {} timeout", stats.timeouts);
                    }
                    h.push(')');
                    let sug = match stats.recommended_timeout_ms {
                        Some(ms) => fmt_duration(ms),
                        None => "-".to_string(),
                    };
                    (h, sug)
                }
                _ => ("no history".to_string(), "-".to_string()),
            };
            let _ = writeln!(
                out,
                "| `{node_id}` | {configured_str} | {history_str} | {suggested_str} |"
            );
        }
    }

    if !report.loop_edges.is_empty() {
        let _ = writeln!(out, "\n### Loop Bounds");
        for (from, to, limit) in &report.loop_edges {
            match limit {
                Some(n) => {
                    let _ = writeln!(out, "- `{from} -> {to}`: max {n} attempts");
                }
                None => {
                    let _ = writeln!(out, "- `{from} -> {to}`: **UNBOUNDED**");
                }
            }
        }
    }

    if !report.issues.is_empty() {
        let _ = writeln!(out, "\n### Issues");
        for issue in &report.issues {
            let tag = if issue.blocking { "BLOCKING" } else { "WARN" };
            let _ = writeln!(out, "- [{tag}] {}", issue.message);
        }
    }

    if !report.steering_notes.is_empty() {
        let _ = writeln!(out, "\n### Steering Notes");
        for note in &report.steering_notes {
            let _ = writeln!(out, "- {note}");
        }
    }

    let _ = writeln!(out, "\n### Next");
    if report.ready_to_launch() {
        let _ = writeln!(
            out,
            "Reply with `launch` to start, or reply with budget adjustments (e.g. `maxSteps 40`, `timeout <node> 180s`, `loop <from>-><to> 3`)."
        );
    } else {
        let _ = writeln!(
            out,
            "Reply in plain text to fill the blocking budgets (e.g. `maxSteps 40`, `timeout <node> 180s`, `loop <from>-><to> 3`, or `use suggested timeouts`), then `launch`."
        );
    }

    out
}

fn step_duration_ms(step: &crate::acpx::bundle::Step) -> u64 {
    let from_timestamps = step.duration_ms();
    if from_timestamps > 0 {
        from_timestamps
    } else {
        step.action
            .as_ref()
            .and_then(|a| a.duration_ms)
            .unwrap_or(0)
    }
}

fn median_sorted_u64(sorted: &[u64]) -> u64 {
    let len = sorted.len();
    if len == 0 {
        return 0;
    }
    let mid = len / 2;
    if len % 2 == 1 {
        sorted.get(mid).copied().unwrap_or(0)
    } else {
        let lo = sorted.get(mid.saturating_sub(1)).copied().unwrap_or(0);
        let hi = sorted.get(mid).copied().unwrap_or(0);
        lo.saturating_add(hi.saturating_sub(lo) / 2)
    }
}

fn compute_recommended_timeout_ms(max_duration_ms: u64) -> u64 {
    let doubled = max_duration_ms.saturating_mul(2);
    let rounded = if doubled == 0 {
        0
    } else {
        let rem = doubled % TIMEOUT_ROUND_STEP_MS;
        if rem == 0 {
            doubled
        } else {
            doubled.saturating_add(TIMEOUT_ROUND_STEP_MS - rem)
        }
    };
    rounded.max(MIN_RECOMMENDED_TIMEOUT_MS)
}

fn is_underbudgeted_timeout(configured_ms: u64, historical_max_ms: u64) -> bool {
    // True when configured_ms <= historical_max_ms * 1.10 (i.e. smaller than or within 10% of historical max).
    (configured_ms as u128).saturating_mul(10) <= (historical_max_ms as u128).saturating_mul(11)
}

fn effective_flow_name(run: &Run) -> &str {
    if !run.summary.flow_name.is_empty() {
        run.summary.flow_name.as_str()
    } else {
        run.flow.name.as_str()
    }
}

fn flow_name_from_path(path: &Path) -> Option<String> {
    let file_name = path.file_name()?.to_str()?;
    let suffixes = [
        ".workflow.ts",
        ".workflow.js",
        ".workflow.mjs",
        ".workflow.cjs",
        ".workflow.json",
        ".workflow.yaml",
        ".workflow.yml",
        ".ts",
        ".js",
        ".json",
        ".yaml",
        ".yml",
    ];
    let stripped = suffixes
        .iter()
        .find_map(|suffix| file_name.strip_suffix(suffix))
        .unwrap_or(file_name);
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_string())
    }
}

fn json_max_steps(raw: &Value) -> Option<usize> {
    raw.get("maxSteps")
        .and_then(Value::as_u64)
        .and_then(|v| usize::try_from(v).ok())
        .filter(|&n| n > 0)
}

fn json_node_entry<'a>(raw: &'a Value, node_id: &str) -> Option<&'a Value> {
    let nodes = raw.get("nodes")?;
    if let Some(map) = nodes.as_object() {
        map.get(node_id)
    } else if let Some(arr) = nodes.as_array() {
        arr.iter()
            .find(|item| item.get("id").and_then(Value::as_str) == Some(node_id))
    } else {
        None
    }
}

fn json_node_timeout_ms(raw: &Value, node_id: &str) -> Option<u64> {
    json_node_entry(raw, node_id)?
        .get("timeoutMs")
        .and_then(Value::as_u64)
        .filter(|&ms| ms > 0)
}

fn json_node_max_attempts(raw: &Value, node_id: &str) -> Option<usize> {
    json_node_entry(raw, node_id)?
        .get("maxAttempts")
        .and_then(Value::as_u64)
        .and_then(|v| usize::try_from(v).ok())
        .filter(|&n| n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acpx::bundle::{ActionReceipt, FlowNode, NodeKind, RunSummary, Step};
    use serde_json::json;

    fn make_node(id: &str, kind: NodeKind, timeout_ms: Option<u64>) -> FlowNode {
        FlowNode {
            id: id.to_string(),
            kind,
            summary: None,
            is_shell: matches!(kind, NodeKind::Action),
            timeout_ms,
            mount_path: Vec::new(),
            local_node_id: None,
            include_transition: None,
        }
    }

    fn beta8_merge_flow(
        max_steps: Option<usize>,
        timeouts: [Option<u64>; 4],
    ) -> FlowDef {
        FlowDef {
            name: "beta8-merge".to_string(),
            start_at: "merge".to_string(),
            nodes: vec![
                make_node("merge", NodeKind::Action, timeouts[0]),
                make_node("cargo_check", NodeKind::Action, timeouts[1]),
                make_node("repair_compile", NodeKind::Acp, timeouts[2]),
                make_node("finalize", NodeKind::Action, timeouts[3]),
            ],
            edges: vec![
                FlowEdge::Simple {
                    from: "merge".to_string(),
                    to: "cargo_check".to_string(),
                },
                FlowEdge::Switch {
                    from: "cargo_check".to_string(),
                    on: "status".to_string(),
                    cases: vec![
                        ("ok".to_string(), "finalize".to_string()),
                        ("err".to_string(), "repair_compile".to_string()),
                    ],
                },
                FlowEdge::Simple {
                    from: "repair_compile".to_string(),
                    to: "cargo_check".to_string(),
                },
            ],
            max_steps,
            mounts: Vec::new(),
        }
    }

    fn make_step(
        attempt_id: &str,
        node_id: &str,
        outcome: Outcome,
        duration_ms: i64,
        exit_code: Option<i64>,
    ) -> Step {
        Step {
            attempt_id: attempt_id.to_string(),
            node_id: node_id.to_string(),
            kind: NodeKind::Action,
            outcome,
            started_ms: 1_000,
            finished_ms: 1_000 + duration_ms,
            prompt_text: None,
            raw_text: None,
            output: Value::Null,
            error: None,
            agent_name: None,
            action: exit_code.map(|code| ActionReceipt {
                action_type: "shell".to_string(),
                command: Some("cargo".to_string()),
                args: vec!["check".to_string()],
                cwd: None,
                exit_code: Some(code),
                duration_ms: Some(duration_ms.max(0) as u64),
            }),
            conversation: None,
            stdout: None,
            stderr: None,
            output_artifact: None,
        }
    }

    fn make_run(
        run_id: &str,
        flow: FlowDef,
        status: RunStatus,
        started_ms: i64,
        finished_ms: Option<i64>,
        steps: Vec<Step>,
    ) -> Run {
        let updated_ms = finished_ms.unwrap_or(started_ms + 5_000);
        Run {
            summary: RunSummary {
                run_id: run_id.to_string(),
                dir: PathBuf::from(format!("/tmp/runs/{run_id}")),
                flow_name: flow.name.clone(),
                title: flow.name.clone(),
                status,
                started_ms,
                updated_ms,
                finished_ms,
                current_node: None,
                waiting_on: None,
                status_detail: None,
                error: None,
            },
            flow,
            steps,
            events: Vec::new(),
            input: Value::Null,
            sessions: Vec::new(),
        }
    }

    #[test]
    fn find_cycle_back_edges_detects_beta8_repair_cycle_and_empty_for_dag() {
        let cyclic = beta8_merge_flow(None, [None, None, None, None]);
        let back_edges = find_cycle_back_edges(&cyclic);
        assert_eq!(
            back_edges,
            vec![("repair_compile".to_string(), "cargo_check".to_string())]
        );

        let dag = FlowDef {
            name: "linear-dag".to_string(),
            start_at: "a".to_string(),
            nodes: vec![
                make_node("a", NodeKind::Action, Some(60_000)),
                make_node("b", NodeKind::Acp, Some(60_000)),
                make_node("c", NodeKind::Action, Some(60_000)),
            ],
            edges: vec![
                FlowEdge::Simple {
                    from: "a".to_string(),
                    to: "b".to_string(),
                },
                FlowEdge::Simple {
                    from: "a".to_string(),
                    to: "c".to_string(),
                },
                FlowEdge::Simple {
                    from: "b".to_string(),
                    to: "c".to_string(),
                },
            ],
            max_steps: Some(10),
            mounts: Vec::new(),
        };
        assert!(find_cycle_back_edges(&dag).is_empty());
    }

    #[test]
    fn analyze_workflow_history_aggregates_durations_and_allow_failure_exit_codes() {
        let flow = beta8_merge_flow(None, [None, None, None, None]);
        let run1 = make_run(
            "run-1",
            flow.clone(),
            RunStatus::Completed,
            10_000,
            Some(95_000),
            vec![
                // allowFailure step: Outcome::Ok but non-zero exit_code 101 => effective_outcome == Failed
                make_step("a1", "cargo_check", Outcome::Ok, 40_000, Some(101)),
                make_step("a2", "repair_compile", Outcome::Ok, 50_000, None),
                make_step("a3", "cargo_check", Outcome::Ok, 20_000, Some(0)),
            ],
        );
        let run2 = make_run(
            "run-2",
            flow,
            RunStatus::TimedOut,
            20_000,
            Some(120_000),
            vec![make_step(
                "b1",
                "cargo_check",
                Outcome::TimedOut,
                60_000,
                None,
            )],
        );
        let unrelated_flow = FlowDef {
            name: "other-flow".to_string(),
            start_at: "x".to_string(),
            nodes: vec![make_node("x", NodeKind::Action, None)],
            edges: Vec::new(),
            max_steps: None,
            mounts: Vec::new(),
        };
        let other_run = make_run(
            "run-other",
            unrelated_flow,
            RunStatus::Completed,
            30_000,
            Some(35_000),
            vec![make_step("c1", "cargo_check", Outcome::Ok, 999_000, Some(0))],
        );

        let stats = analyze_workflow_history("beta8-merge", &[run1, run2, other_run]);
        assert_eq!(stats.len(), 2);

        let check = stats.iter().find(|s| s.node_id == "cargo_check").unwrap();
        assert_eq!(check.executions, 3);
        assert_eq!(check.failures, 1);
        assert_eq!(check.timeouts, 1);
        assert_eq!(check.min_duration_ms, 20_000);
        assert_eq!(check.median_duration_ms, 40_000);
        assert_eq!(check.max_duration_ms, 60_000);
        // 60_000 * 2 = 120_000 (already multiple of 30_000)
        assert_eq!(check.recommended_timeout_ms, Some(120_000));

        let repair = stats.iter().find(|s| s.node_id == "repair_compile").unwrap();
        assert_eq!(repair.executions, 1);
        assert_eq!(repair.failures, 0);
        assert_eq!(repair.timeouts, 0);
        // 50_000 * 2 = 100_000 -> rounded up to 120_000
        assert_eq!(repair.recommended_timeout_ms, Some(120_000));
    }

    #[test]
    fn audit_workflow_preflight_flags_beta8_merge_and_clears_with_overrides() {
        let flow = beta8_merge_flow(None, [None, None, None, None]);
        let past_run = make_run(
            "run-hist-1",
            flow.clone(),
            RunStatus::Completed,
            1_000,
            Some(200_000),
            vec![
                make_step("s1", "merge", Outcome::Ok, 15_000, Some(0)),
                make_step("s2", "cargo_check", Outcome::Ok, 95_000, Some(0)),
                make_step("s3", "repair_compile", Outcome::Ok, 70_000, None),
                make_step("s4", "finalize", Outcome::Ok, 10_000, Some(0)),
            ],
        );

        let initial = audit_workflow_preflight(
            &flow,
            None,
            std::slice::from_ref(&past_run),
            &PreflightOverrides::default(),
        );
        assert!(!initial.ready_to_launch());
        assert!(initial
            .issues
            .iter()
            .any(|i| i.blocking && matches!(i.kind, PreflightIssueKind::MissingMaxSteps)));
        assert!(initial.issues.iter().any(|i| {
            i.blocking
                && matches!(
                    &i.kind,
                    PreflightIssueKind::UnboundedLoop { from, to }
                        if from == "repair_compile" && to == "cargo_check"
                )
        }));
        assert!(initial.issues.iter().any(|i| {
            i.blocking
                && matches!(
                    &i.kind,
                    PreflightIssueKind::MissingNodeTimeout { node_id, suggested_ms }
                        if node_id == "cargo_check" && *suggested_ms == Some(210_000)
                )
        }));

        // Underbudgeted timeout: cargo_check historical max is 95_000ms; 100_000ms is within 10% (<= 104_500ms).
        let underbudgeted_flow = beta8_merge_flow(
            Some(30),
            [Some(60_000), Some(100_000), Some(150_000), Some(60_000)],
        );
        let mut loop_only_override = PreflightOverrides::default();
        loop_only_override
            .loop_limits
            .insert("repair_compile->cargo_check".to_string(), 3);
        let underbudgeted_report = audit_workflow_preflight(
            &underbudgeted_flow,
            None,
            std::slice::from_ref(&past_run),
            &loop_only_override,
        );
        assert!(underbudgeted_report.ready_to_launch());
        assert!(underbudgeted_report.issues.iter().any(|i| {
            !i.blocking
                && matches!(
                    &i.kind,
                    PreflightIssueKind::UnderbudgetedTimeout {
                        node_id,
                        configured_ms: 100_000,
                        historical_max_ms: 95_000,
                        suggested_ms: 210_000,
                    } if node_id == "cargo_check"
                )
        }));

        // Full overrides resolve all blocking issues and warnings.
        let mut overrides = PreflightOverrides {
            max_steps: Some(40),
            steering_notes: vec!["Stop if clippy fails twice".to_string()],
            ..PreflightOverrides::default()
        };
        overrides.node_timeouts_ms.insert("merge".to_string(), 60_000);
        overrides
            .node_timeouts_ms
            .insert("cargo_check".to_string(), 210_000);
        overrides
            .node_timeouts_ms
            .insert("repair_compile".to_string(), 150_000);
        overrides
            .node_timeouts_ms
            .insert("finalize".to_string(), 60_000);
        overrides
            .loop_limits
            .insert("repair_compile->cargo_check".to_string(), 4);

        let ready = audit_workflow_preflight(&flow, None, &[past_run], &overrides);
        assert!(ready.ready_to_launch());
        assert!(ready.issues.is_empty());
        assert_eq!(ready.effective_max_steps, Some(40));
        assert_eq!(
            ready.loop_edges,
            vec![("repair_compile".to_string(), "cargo_check".to_string(), Some(4))]
        );
        assert_eq!(ready.steering_notes, vec!["Stop if clippy fails twice"]);
    }

    #[test]
    fn audit_workflow_preflight_reads_raw_flow_json_fallbacks() {
        let flow = beta8_merge_flow(None, [None, None, None, None]);
        let raw = json!({
            "maxSteps": 25,
            "nodes": {
                "merge": { "timeoutMs": 60000 },
                "cargo_check": { "timeoutMs": 120000 },
                "repair_compile": { "timeoutMs": 180000, "maxAttempts": 3 },
                "finalize": { "timeoutMs": 60000 }
            }
        });

        let report =
            audit_workflow_preflight(&flow, Some(&raw), &[], &PreflightOverrides::default());
        assert!(report.ready_to_launch());
        assert_eq!(report.effective_max_steps, Some(25));
        assert_eq!(
            report.loop_edges,
            vec![("repair_compile".to_string(), "cargo_check".to_string(), Some(3))]
        );
    }

    #[test]
    fn discover_workflow_templates_merges_runs_and_files() {
        let beta8 = beta8_merge_flow(Some(30), [Some(60_000); 4]);
        let run1 = make_run(
            "run-01",
            beta8.clone(),
            RunStatus::Completed,
            1_000,
            Some(61_000),
            Vec::new(),
        );
        let run2 = make_run(
            "run-02",
            beta8,
            RunStatus::Failed,
            10_000,
            Some(130_000),
            Vec::new(),
        );

        let files = vec![
            PathBuf::from("workflows/beta8-merge.workflow.ts"),
            PathBuf::from("workflows/nightly-audit.workflow.ts"),
        ];

        let catalog = discover_workflow_templates(&[run1, run2], &files);
        assert_eq!(catalog.len(), 2);

        let merged = catalog
            .iter()
            .find(|e| e.flow_name == "beta8-merge")
            .unwrap();
        assert_eq!(
            merged.source_path,
            Some(PathBuf::from("workflows/beta8-merge.workflow.ts"))
        );
        assert_eq!(merged.node_count, 4);
        assert!(merged.has_loops);
        assert_eq!(merged.recorded_runs, 2);
        assert_eq!(merged.completed_runs, 1);
        assert_eq!(merged.failed_runs, 1);
        assert_eq!(merged.last_run_id.as_deref(), Some("run-02"));
        // Durations: 60_000 and 120_000 => median 90_000
        assert_eq!(merged.median_run_duration_ms, Some(90_000));

        let fresh = catalog
            .iter()
            .find(|e| e.flow_name == "nightly-audit")
            .unwrap();
        assert_eq!(
            fresh.source_path,
            Some(PathBuf::from("workflows/nightly-audit.workflow.ts"))
        );
        assert_eq!(fresh.recorded_runs, 0);
        assert!(!fresh.has_loops);
        assert_eq!(fresh.last_run_id, None);
        assert_eq!(fresh.median_run_duration_ms, None);
    }

    #[test]
    fn format_preflight_markdown_renders_budgets_loops_and_issues() {
        let flow = beta8_merge_flow(None, [None, None, None, None]);
        let report =
            audit_workflow_preflight(&flow, None, &[], &PreflightOverrides::default());
        let md = format_preflight_markdown(&report);
        assert!(md.contains("Workflow Preflight: `beta8-merge`"));
        assert!(md.contains("BLOCKED"));
        assert!(md.contains("`repair_compile -> cargo_check`: **UNBOUNDED**"));
        assert!(md.contains("[BLOCKING]"));
        assert!(md.contains("Reply in plain text"));
    }

    #[test]
    fn compute_recommended_timeout_ms_rounds_up_to_30s_with_60s_floor() {
        assert_eq!(compute_recommended_timeout_ms(0), 60_000);
        assert_eq!(compute_recommended_timeout_ms(10_000), 60_000);
        assert_eq!(compute_recommended_timeout_ms(30_000), 60_000);
        assert_eq!(compute_recommended_timeout_ms(30_001), 90_000);
        assert_eq!(compute_recommended_timeout_ms(45_000), 90_000);
        assert_eq!(compute_recommended_timeout_ms(50_000), 120_000);
        assert_eq!(compute_recommended_timeout_ms(u64::MAX), u64::MAX);
    }
}
