use std::collections::{HashMap, HashSet};

use xai_grok_pager::acpx::step_digest::{
    DEFAULT_FAILURE_MAX_CHARS, DEFAULT_FAILURE_MAX_LINES, active_branch_nodes,
    parse_scoped_node_id, trim_failure_tail,
};

use crate::driver::{FlowSketch, GateChoice};

/// Summary of the most recent failed (or non-zero exit) step leading into a gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrecedingFailure {
    pub node_id: String,
    pub exit_code: Option<i64>,
    pub detail: String,
}

impl PrecedingFailure {
    pub fn from_raw(
        node_id: &str,
        error: Option<&str>,
        stderr: Option<&str>,
        output_tail: Option<&str>,
        stdout: Option<&str>,
        exit_code: Option<i64>,
    ) -> Self {
        let detail = trim_failure_tail(
            error,
            stderr,
            output_tail,
            stdout,
            exit_code,
            DEFAULT_FAILURE_MAX_LINES,
            DEFAULT_FAILURE_MAX_CHARS,
        )
        .unwrap_or_else(|| match exit_code {
            Some(code) if code != 0 => format!("exit code {code}"),
            _ => String::new(),
        });

        Self {
            node_id: node_id.to_string(),
            exit_code,
            detail,
        }
    }
}

/// Prune untaken branches from a flow sketch by delegating to `step_digest::active_branch_nodes`.
pub fn prune_flow_sketch(flow: &FlowSketch, states: &HashMap<String, char>) -> FlowSketch {
    let node_ids: Vec<String> = flow.nodes.iter().map(|n| n.id.clone()).collect();
    let edges: Vec<(String, String)> = flow
        .edges
        .iter()
        .map(|e| (e.from.clone(), e.to.clone()))
        .collect();
    let visited: HashSet<String> = states
        .iter()
        .filter(|(_, glyph)| **glyph != '○')
        .map(|(id, _)| id.clone())
        .collect();

    let keep = active_branch_nodes(&flow.start_at, &node_ids, &edges, &visited);

    let nodes = flow
        .nodes
        .iter()
        .filter(|n| keep.contains(&n.id))
        .cloned()
        .collect();
    let kept_edges = flow
        .edges
        .iter()
        .filter(|e| keep.contains(&e.from) && keep.contains(&e.to))
        .cloned()
        .collect();

    FlowSketch {
        name: flow.name.clone(),
        start_at: flow.start_at.clone(),
        nodes,
        edges: kept_edges,
    }
}

/// Render the active branch of `flow` as Mermaid flowchart source (no fences).
/// Uses `flowchart LR` only when the pruned flow has `<= 4` nodes and max out-degree `<= 1`,
/// otherwise `flowchart TD`.
pub fn active_mermaid(flow: &FlowSketch, states: &HashMap<String, char>) -> String {
    let pruned = prune_flow_sketch(flow, states);

    let mut out_degrees: HashMap<&str, usize> = HashMap::new();
    for edge in &pruned.edges {
        *out_degrees.entry(edge.from.as_str()).or_insert(0) += 1;
    }
    let max_out_degree = out_degrees.values().copied().max().unwrap_or(0);
    let direction = if pruned.nodes.len() <= 4 && max_out_degree <= 1 {
        "LR"
    } else {
        "TD"
    };

    let mut lines = Vec::with_capacity(1 + pruned.nodes.len() + pruned.edges.len());
    lines.push(format!("flowchart {direction}"));

    for node in &pruned.nodes {
        let sid = sanitize_mermaid_id(&node.id);
        let glyph = states.get(&node.id).copied().unwrap_or('○');
        let display_name = format_mermaid_node_name(&node.id);
        let label = escape_mermaid_text(&format!("{glyph} {display_name}"));
        lines.push(format!("    {sid}[\"{label}\"]"));
    }

    for edge in &pruned.edges {
        let from = sanitize_mermaid_id(&edge.from);
        let to = sanitize_mermaid_id(&edge.to);
        if let Some(label) = edge.label.as_deref().filter(|l| !l.is_empty()) {
            let clean_label = label.replace('|', "/").replace('\n', " ");
            lines.push(format!("    {from} -->|{clean_label}| {to}"));
        } else {
            lines.push(format!("    {from} --> {to}"));
        }
    }

    lines.join("\n")
}

/// Single-line taken-path progress breadcrumb for steps that have run or are running.
pub fn compact_path_line(
    flow: &FlowSketch,
    states: &HashMap<String, char>,
    exit_codes: &HashMap<String, i64>,
) -> String {
    flow.nodes
        .iter()
        .filter_map(|node| {
            let glyph = states.get(&node.id).copied()?;
            if glyph == '○' {
                return None;
            }
            let name = format_compact_node_name(&node.id);
            let exit_suffix = match exit_codes.get(&node.id).copied() {
                Some(code) if code != 0 => format!(" (exit {code})"),
                _ => String::new(),
            };
            Some(format!("{glyph} {name}{exit_suffix}"))
        })
        .collect::<Vec<_>>()
        .join(" → ")
}

/// Render an inline waiting-gate transcript card with optional preceding failure context
/// and numbered choices (no modal pop-up).
pub fn format_inline_waiting_gate(
    node: &str,
    question: &str,
    choices: &[GateChoice],
    failure: Option<&PrecedingFailure>,
) -> String {
    let mut sections = Vec::new();
    let node_display = format_compact_node_name(node);
    sections.push(format!("**⏸ WAITING FOR YOUR ANSWER** · `{node_display}`"));

    let trimmed_question = question.trim();
    if !trimmed_question.is_empty() {
        sections.push(trimmed_question.to_string());
    }

    if let Some(fail) = failure {
        let fail_node = format_compact_node_name(&fail.node_id);
        let heading = match fail.exit_code {
            Some(code) if code != 0 => format!("Preceding failure in `{fail_node}` (exit {code}):"),
            _ => format!("Preceding failure in `{fail_node}`:"),
        };
        let trimmed_detail = fail.detail.trim();
        if trimmed_detail.is_empty() {
            sections.push(heading);
        } else {
            let fence = if trimmed_detail.contains("```") {
                "````"
            } else {
                "```"
            };
            sections.push(format!("{heading}\n{fence}text\n{trimmed_detail}\n{fence}"));
        }
    }

    if !choices.is_empty() {
        let mut choice_lines = Vec::with_capacity(choices.len() + 1);
        choice_lines.push("Choices:".to_string());
        for (idx, choice) in choices.iter().enumerate() {
            choice_lines.push(format!("{}. **{}**", idx + 1, choice.label));
        }
        sections.push(choice_lines.join("\n"));
        sections.push(
            "Reply in chat with a choice number or label (optional notes after `:`, e.g. `repair: ...`)."
                .to_string(),
        );
    } else {
        sections.push("Reply in chat to continue.".to_string());
    }

    sections.join("\n\n")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateTextMatch {
    Matched {
        label: String,
        notes: Option<String>,
    },
    Refused {
        refusal_message: String,
    },
}

/// Fuzzy plain-text gate choice matching. Unmatched or ambiguous input is refused with
/// the valid choices so the run remains waiting rather than cancelling.
pub fn match_waiting_gate_text(
    node: &str,
    choices: &[GateChoice],
    input: &str,
) -> GateTextMatch {
    let trimmed = input.trim();
    if trimmed.is_empty() || choices.is_empty() {
        return GateTextMatch::Refused {
            refusal_message: format_gate_refusal(node, choices, trimmed, None),
        };
    }

    // 1. Try matching the full input directly (handles multi-word labels, numbers, and prefixes).
    match match_choice_token(choices, trimmed, true) {
        TokenMatch::Unique(label) => {
            return GateTextMatch::Matched { label, notes: None };
        }
        TokenMatch::Ambiguous(candidates) => {
            return GateTextMatch::Refused {
                refusal_message: format_gate_refusal(node, choices, trimmed, Some(&candidates)),
            };
        }
        TokenMatch::None => {}
    }

    // 2. Try "<choice>: <notes>" syntax.
    if let Some((head, tail)) = trimmed.split_once(':') {
        let head = head.trim();
        let notes = non_empty_trimmed(tail);
        if !head.is_empty() {
            match match_choice_token(choices, head, true) {
                TokenMatch::Unique(label) => {
                    return GateTextMatch::Matched { label, notes };
                }
                TokenMatch::Ambiguous(candidates) => {
                    return GateTextMatch::Refused {
                        refusal_message: format_gate_refusal(
                            node,
                            choices,
                            trimmed,
                            Some(&candidates),
                        ),
                    };
                }
                TokenMatch::None => {}
            }
        }
    }

    // 3. Try "<exact choice or number> <notes>" without fuzzy prefixing to avoid false positives.
    if let Some((head, tail)) = trimmed.split_once(char::is_whitespace) {
        let head = head.trim();
        let notes = non_empty_trimmed(tail);
        if !head.is_empty()
            && let TokenMatch::Unique(label) = match_choice_token(choices, head, false)
        {
            return GateTextMatch::Matched { label, notes };
        }
    }

    GateTextMatch::Refused {
        refusal_message: format_gate_refusal(node, choices, trimmed, None),
    }
}

/// Live operational control action parsed from plain text or slash commands while a workflow run is active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MidRunCommand {
    /// Show the live `/status` report for the active run (`"status"`, `"/status"`, `"?"`).
    Status,
    /// Pause the active run (`"pause"`, `"/pause"`).
    Pause,
    /// Resume a paused run (`"resume"`, `"/resume"`, `"continue"`).
    Resume,
    /// Update a scoped setting or budget mid-run (`"set <key>=<value>"` or `"/set <key>=<value>"`).
    ChangeSetting { key: String, value: String },
    /// Queue a steering instruction / follow-up prompt for the run (`"steer: <prompt>"`, `"/steer <prompt>"`, or any non-empty natural-language steering text).
    QueueFollowUp { prompt: String },
}

/// Classify user input typed while a workflow run is active (replacing the hard `"A run is in progress; press Esc to cancel it."` block).
pub fn classify_mid_run_input(input: &str) -> Option<MidRunCommand> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    let (had_slash, body) = match trimmed.strip_prefix('/') {
        Some(rest) => (true, rest.trim()),
        None => (false, trimmed),
    };
    if body.is_empty() {
        return None;
    }

    if body.eq_ignore_ascii_case("status") || body == "?" {
        return Some(MidRunCommand::Status);
    }
    if body.eq_ignore_ascii_case("pause") {
        return Some(MidRunCommand::Pause);
    }
    if body.eq_ignore_ascii_case("resume") || body.eq_ignore_ascii_case("continue") {
        return Some(MidRunCommand::Resume);
    }

    if let Some(rest) = strip_ascii_prefix_ci(body, "set ") {
        let rest = rest.trim();
        let split = rest
            .split_once('=')
            .or_else(|| rest.split_once(char::is_whitespace));
        if let Some((raw_key, raw_value)) = split {
            let key = raw_key.trim();
            let value = raw_value.trim();
            if !key.is_empty() && !value.is_empty() {
                return Some(MidRunCommand::ChangeSetting {
                    key: key.to_string(),
                    value: value.to_string(),
                });
            }
        }
        return None;
    }

    if let Some(rest) =
        strip_ascii_prefix_ci(body, "steer:").or_else(|| strip_ascii_prefix_ci(body, "steer "))
    {
        let prompt = rest.trim();
        return (!prompt.is_empty()).then(|| MidRunCommand::QueueFollowUp {
            prompt: prompt.to_string(),
        });
    }

    if had_slash {
        return None;
    }

    Some(MidRunCommand::QueueFollowUp {
        prompt: trimmed.to_string(),
    })
}

enum TokenMatch {
    Unique(String),
    Ambiguous(Vec<String>),
    None,
}

fn match_choice_token(choices: &[GateChoice], token: &str, allow_prefix: bool) -> TokenMatch {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return TokenMatch::None;
    }

    // 1-based numeric index.
    if let Ok(index) = trimmed.parse::<usize>()
        && index >= 1
        && let Some(choice) = choices.get(index - 1)
    {
        return TokenMatch::Unique(choice.label.clone());
    }

    // Exact case-insensitive match.
    let lower = trimmed.to_ascii_lowercase();
    let exact: Vec<String> = choices
        .iter()
        .filter(|c| c.label.trim().to_ascii_lowercase() == lower)
        .map(|c| c.label.clone())
        .collect();
    if let Some(outcome) = collapse_matches(exact) {
        return outcome;
    }

    // Normalized exact match (hyphens/underscores/whitespace treated equivalently).
    let norm = normalize_choice_text(trimmed);
    if !norm.is_empty() {
        let normalized_exact: Vec<String> = choices
            .iter()
            .filter(|c| normalize_choice_text(&c.label) == norm)
            .map(|c| c.label.clone())
            .collect();
        if let Some(outcome) = collapse_matches(normalized_exact) {
            return outcome;
        }
    }

    // Unique prefix match.
    if allow_prefix && !norm.is_empty() {
        let prefixed: Vec<String> = choices
            .iter()
            .filter(|c| {
                c.label.trim().to_ascii_lowercase().starts_with(&lower)
                    || normalize_choice_text(&c.label).starts_with(&norm)
            })
            .map(|c| c.label.clone())
            .collect();
        if let Some(outcome) = collapse_matches(prefixed) {
            return outcome;
        }
    }

    TokenMatch::None
}

fn collapse_matches(matches: Vec<String>) -> Option<TokenMatch> {
    match matches.len() {
        0 => None,
        1 => matches.into_iter().next().map(TokenMatch::Unique),
        _ => Some(TokenMatch::Ambiguous(matches)),
    }
}

fn normalize_choice_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = true;
    for ch in s.chars() {
        if ch.is_whitespace() || ch == '-' || ch == '_' {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            for lc in ch.to_lowercase() {
                out.push(lc);
            }
            prev_space = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

fn non_empty_trimmed(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn format_gate_refusal(
    node: &str,
    choices: &[GateChoice],
    input: &str,
    ambiguous: Option<&[String]>,
) -> String {
    let node_display = format_compact_node_name(node);
    if choices.is_empty() {
        return format!("Gate `{node_display}` has no selectable choices; run is still waiting.");
    }

    let valid = choices
        .iter()
        .enumerate()
        .map(|(idx, choice)| format!("{}. {}", idx + 1, choice.label))
        .collect::<Vec<_>>()
        .join(", ");

    if let Some(candidates) = ambiguous {
        format!(
            "Ambiguous reply {input:?} for gate `{node_display}` (matches: {}). Run is still waiting; valid choices: {valid}",
            candidates.join(", ")
        )
    } else {
        format!(
            "Unrecognized reply {input:?} for gate `{node_display}`. Run is still waiting; valid choices: {valid}"
        )
    }
}

fn strip_ascii_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) {
        s.get(prefix.len()..)
    } else {
        None
    }
}

fn format_mermaid_node_name(node_id: &str) -> String {
    match parse_scoped_node_id(node_id) {
        (Some(mount), _, Some(exit_name)) => format!("{mount} ↳ exit:{exit_name}"),
        (Some(mount), local, None) => format!("{mount} / {local}"),
        (None, _, Some(exit_name)) => format!("exit:{exit_name}"),
        (None, local, None) => local.to_string(),
    }
}

fn format_compact_node_name(node_id: &str) -> String {
    match parse_scoped_node_id(node_id) {
        (Some(mount), _, Some(exit_name)) => format!("{mount}↳exit:{exit_name}"),
        (Some(mount), local, None) => format!("{mount}/{local}"),
        (None, _, Some(exit_name)) => format!("exit:{exit_name}"),
        (None, local, None) => local.to_string(),
    }
}

fn sanitize_mermaid_id(id: &str) -> String {
    let mut out = String::with_capacity(id.len().max(1));
    for ch in id.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "_".to_string()
    } else if is_mermaid_keyword(&out) {
        format!("n_{out}")
    } else {
        out
    }
}

fn is_mermaid_keyword(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "end" | "subgraph" | "graph" | "flowchart" | "style" | "classdef" | "class" | "click"
    )
}

fn escape_mermaid_text(s: &str) -> String {
    s.replace('"', "'").replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::{SketchEdge, SketchNode, StepKind};

    fn node(id: &str, kind: StepKind) -> SketchNode {
        SketchNode {
            id: id.to_string(),
            kind,
            summary: None,
            profile: None,
            model: None,
            effort: None,
        }
    }

    fn edge(from: &str, to: &str, label: Option<&str>) -> SketchEdge {
        SketchEdge {
            from: from.to_string(),
            to: to.to_string(),
            label: label.map(ToString::to_string),
        }
    }

    fn sample_choices() -> Vec<GateChoice> {
        vec![
            GateChoice {
                label: "continue".to_string(),
                flow_file: "continue.flow".to_string(),
            },
            GateChoice {
                label: "repair".to_string(),
                flow_file: "repair.flow".to_string(),
            },
            GateChoice {
                label: "abort".to_string(),
                flow_file: "abort.flow".to_string(),
            },
        ]
    }

    #[test]
    fn prune_flow_sketch_drops_untaken_branch() {
        let flow = FlowSketch {
            name: "host-route".to_string(),
            start_at: "route_host".to_string(),
            nodes: vec![
                node("route_host", StepKind::Compute),
                node("resolve_group1_air", StepKind::Acp),
                node("resolve_group2_shell", StepKind::Shell),
                node("finish", StepKind::Compute),
            ],
            edges: vec![
                edge("route_host", "resolve_group1_air", Some("air")),
                edge("route_host", "resolve_group2_shell", Some("shell")),
                edge("resolve_group1_air", "finish", None),
                edge("resolve_group2_shell", "finish", None),
            ],
        };

        let mut states = HashMap::new();
        states.insert("route_host".to_string(), '✓');
        states.insert("resolve_group2_shell".to_string(), '◐');

        let pruned = prune_flow_sketch(&flow, &states);
        let ids: Vec<&str> = pruned.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(!ids.contains(&"resolve_group1_air"));
        assert!(ids.contains(&"route_host"));
        assert!(ids.contains(&"resolve_group2_shell"));
        assert!(ids.contains(&"finish"));
        assert!(pruned.edges.iter().all(|e| {
            e.from != "resolve_group1_air" && e.to != "resolve_group1_air"
        }));
    }

    #[test]
    fn active_mermaid_uses_lr_for_short_linear_and_td_for_longer_or_branching() {
        let linear3 = FlowSketch {
            name: "short".to_string(),
            start_at: "plan".to_string(),
            nodes: vec![
                node("plan", StepKind::Acp),
                node("implement", StepKind::Acp),
                node("verify/__piw_exit_passed", StepKind::Other),
            ],
            edges: vec![
                edge("plan", "implement", None),
                edge("implement", "verify/__piw_exit_passed", None),
            ],
        };
        let mut states = HashMap::new();
        states.insert("plan".to_string(), '✓');
        states.insert("implement".to_string(), '✓');
        states.insert("verify/__piw_exit_passed".to_string(), '✓');

        let chart3 = active_mermaid(&linear3, &states);
        assert!(chart3.starts_with("flowchart LR\n"));
        assert!(chart3.contains("✓ verify ↳ exit:passed"));

        let linear5 = FlowSketch {
            name: "long".to_string(),
            start_at: "n1".to_string(),
            nodes: vec![
                node("n1", StepKind::Acp),
                node("n2", StepKind::Acp),
                node("n3", StepKind::Acp),
                node("n4", StepKind::Acp),
                node("n5", StepKind::Acp),
            ],
            edges: vec![
                edge("n1", "n2", None),
                edge("n2", "n3", None),
                edge("n3", "n4", None),
                edge("n4", "n5", None),
            ],
        };
        let chart5 = active_mermaid(&linear5, &HashMap::new());
        assert!(chart5.starts_with("flowchart TD\n"));

        let branching3 = FlowSketch {
            name: "branch".to_string(),
            start_at: "root".to_string(),
            nodes: vec![
                node("root", StepKind::Compute),
                node("left", StepKind::Acp),
                node("right", StepKind::Acp),
            ],
            edges: vec![
                edge("root", "left", Some("l")),
                edge("root", "right", Some("r")),
            ],
        };
        let chart_branch = active_mermaid(&branching3, &HashMap::new());
        assert!(chart_branch.starts_with("flowchart TD\n"));
    }

    #[test]
    fn compact_path_line_formats_exit_codes_and_scoped_exits() {
        let flow = FlowSketch {
            name: "ci".to_string(),
            start_at: "cargo_check".to_string(),
            nodes: vec![
                node("cargo_check", StepKind::Shell),
                node("verify/__piw_exit_passed", StepKind::Other),
                node("unreached", StepKind::Acp),
            ],
            edges: vec![
                edge("cargo_check", "verify/__piw_exit_passed", None),
                edge("verify/__piw_exit_passed", "unreached", None),
            ],
        };
        let mut states = HashMap::new();
        states.insert("cargo_check".to_string(), '✗');
        states.insert("verify/__piw_exit_passed".to_string(), '✓');
        states.insert("unreached".to_string(), '○');

        let mut exit_codes = HashMap::new();
        exit_codes.insert("cargo_check".to_string(), 101);
        exit_codes.insert("verify/__piw_exit_passed".to_string(), 0);

        let line = compact_path_line(&flow, &states, &exit_codes);
        assert_eq!(line, "✗ cargo_check (exit 101) → ✓ verify↳exit:passed");
    }

    #[test]
    fn format_inline_waiting_gate_renders_failure_tail_and_numbered_choices() {
        let failure = PrecedingFailure::from_raw(
            "cargo_check",
            None,
            Some("error: duplicate key `serde` in Cargo.toml"),
            None,
            None,
            Some(101),
        );
        let choices = sample_choices();
        let card = format_inline_waiting_gate(
            "repair_gate",
            "Cargo check failed. How should we proceed?",
            &choices,
            Some(&failure),
        );

        assert!(card.contains("WAITING FOR YOUR ANSWER"));
        assert!(card.contains("`repair_gate`"));
        assert!(card.contains("Cargo check failed. How should we proceed?"));
        assert!(card.contains("Preceding failure in `cargo_check` (exit 101):"));
        assert!(card.contains("```text\n"));
        assert!(card.contains("duplicate key `serde` in Cargo.toml"));
        assert!(card.contains("1. **continue**"));
        assert!(card.contains("2. **repair**"));
        assert!(card.contains("3. **abort**"));
    }

    #[test]
    fn match_waiting_gate_text_handles_exact_case_index_prefix_notes_and_refusal() {
        let choices = sample_choices();

        assert_eq!(
            match_waiting_gate_text("repair_gate", &choices, "repair"),
            GateTextMatch::Matched {
                label: "repair".to_string(),
                notes: None,
            }
        );
        assert_eq!(
            match_waiting_gate_text("repair_gate", &choices, "REPAIR"),
            GateTextMatch::Matched {
                label: "repair".to_string(),
                notes: None,
            }
        );
        assert_eq!(
            match_waiting_gate_text("repair_gate", &choices, "2"),
            GateTextMatch::Matched {
                label: "repair".to_string(),
                notes: None,
            }
        );
        assert_eq!(
            match_waiting_gate_text("repair_gate", &choices, "rep"),
            GateTextMatch::Matched {
                label: "repair".to_string(),
                notes: None,
            }
        );
        assert_eq!(
            match_waiting_gate_text(
                "repair_gate",
                &choices,
                "repair: fix duplicate key in Cargo.toml"
            ),
            GateTextMatch::Matched {
                label: "repair".to_string(),
                notes: Some("fix duplicate key in Cargo.toml".to_string()),
            }
        );

        match match_waiting_gate_text("repair_gate", &choices, "hello world") {
            GateTextMatch::Refused { refusal_message } => {
                assert!(refusal_message.contains("repair_gate"));
                assert!(refusal_message.contains("1. continue"));
                assert!(refusal_message.contains("2. repair"));
                assert!(refusal_message.contains("3. abort"));
            }
            other => panic!("expected Refused, got {other:?}"),
        }

        // Ambiguous prefix should also refuse rather than guess.
        let ambiguous_choices = vec![
            GateChoice {
                label: "repair".to_string(),
                flow_file: "repair.flow".to_string(),
            },
            GateChoice {
                label: "retry".to_string(),
                flow_file: "retry.flow".to_string(),
            },
        ];
        assert!(matches!(
            match_waiting_gate_text("gate", &ambiguous_choices, "re"),
            GateTextMatch::Refused { .. }
        ));
    }

    #[test]
    fn classify_mid_run_input_maps_commands_and_steering() {
        assert_eq!(classify_mid_run_input("/status"), Some(MidRunCommand::Status));
        assert_eq!(classify_mid_run_input("STATUS"), Some(MidRunCommand::Status));
        assert_eq!(classify_mid_run_input("?"), Some(MidRunCommand::Status));

        assert_eq!(classify_mid_run_input("pause"), Some(MidRunCommand::Pause));
        assert_eq!(classify_mid_run_input("/pause"), Some(MidRunCommand::Pause));

        assert_eq!(classify_mid_run_input("resume"), Some(MidRunCommand::Resume));
        assert_eq!(
            classify_mid_run_input("continue"),
            Some(MidRunCommand::Resume)
        );

        assert_eq!(
            classify_mid_run_input("set timeoutMs=600000"),
            Some(MidRunCommand::ChangeSetting {
                key: "timeoutMs".to_string(),
                value: "600000".to_string(),
            })
        );
        assert_eq!(
            classify_mid_run_input("/set maxSteps 50"),
            Some(MidRunCommand::ChangeSetting {
                key: "maxSteps".to_string(),
                value: "50".to_string(),
            })
        );

        assert_eq!(
            classify_mid_run_input("focus on Cargo.toml duplicate keys"),
            Some(MidRunCommand::QueueFollowUp {
                prompt: "focus on Cargo.toml duplicate keys".to_string(),
            })
        );
        assert_eq!(
            classify_mid_run_input("steer: focus on Cargo.toml"),
            Some(MidRunCommand::QueueFollowUp {
                prompt: "focus on Cargo.toml".to_string(),
            })
        );
        assert_eq!(
            classify_mid_run_input("/steer check unit tests"),
            Some(MidRunCommand::QueueFollowUp {
                prompt: "check unit tests".to_string(),
            })
        );

        assert_eq!(classify_mid_run_input("   "), None);
        assert_eq!(classify_mid_run_input("/"), None);
        assert_eq!(classify_mid_run_input("/unknown"), None);
        assert_eq!(classify_mid_run_input("steer:   "), None);
    }
}
