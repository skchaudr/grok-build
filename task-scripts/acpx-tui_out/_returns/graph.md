```rust
use std::collections::HashMap;
use crate::acpx::bundle::{FlowEdge, NodeKind, NodeState, Run};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    LeftRight,
    TopDown,
}

fn escape_label(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '"' => '\'',
            '[' | ']' | '{' | '}' | '|' | '<' | '>' | '`' => ' ',
            _ => c,
        })
        .collect()
}

/// Mermaid `flowchart` source for a run: one node per flow node, labelled "<state glyph> <id>".
pub fn mermaid_source(run: &Run, direction: Direction) -> String {
    let mut lines = Vec::new();
    let dir_str = match direction {
        Direction::LeftRight => "LR",
        Direction::TopDown => "TD",
    };
    lines.push(format!("flowchart {dir_str}"));

    let mut id_map: HashMap<&str, String> = HashMap::with_capacity(run.flow.nodes.len());

    for (idx, node) in run.flow.nodes.iter().enumerate() {
        let mermaid_id = format!("n{idx}");
        id_map.insert(node.id.as_str(), mermaid_id.clone());

        let state: NodeState = run.node_state(&node.id);
        let raw_label = format!("{} {}", state.glyph(), node.id);
        let label = escape_label(&raw_label);

        let node_line = match node.kind {
            NodeKind::Acp => format!("  {mermaid_id}(\"{label}\")"),
            NodeKind::Checkpoint => format!("  {mermaid_id}{{\"{label}\"}}"),
            NodeKind::Action | NodeKind::Compute | NodeKind::Unknown => {
                format!("  {mermaid_id}[\"{label}\"]")
            }
        };
        lines.push(node_line);
    }

    for edge in &run.flow.edges {
        match edge {
            FlowEdge::Simple { from, to } => {
                let (Some(from_m), Some(to_m)) =
                    (id_map.get(from.as_str()), id_map.get(to.as_str()))
                else {
                    continue;
                };
                let arrow = edge_arrow(run, from, to);
                lines.push(format!("  {from_m} {arrow} {to_m}"));
            }
            FlowEdge::Switch { from, cases, .. } => {
                let Some(from_m) = id_map.get(from.as_str()) else {
                    continue;
                };
                for (case_val, to) in cases {
                    let Some(to_m) = id_map.get(to.as_str()) else {
                        continue;
                    };
                    let arrow = edge_arrow(run, from, to);
                    let escaped_case = escape_label(case_val);
                    lines.push(format!("  {from_m} {arrow}|\"{escaped_case}\"| {to_m}"));
                }
            }
        }
    }

    lines.join("\n")
}

fn edge_arrow(run: &Run, from: &str, to: &str) -> &'static str {
    if run.edge_taken(from, to) {
        "==>"
    } else if run.steps.iter().any(|s| s.node_id == from) {
        "-.->"
    } else {
        "-->"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acpx::bundle::{
        FlowDef, FlowEdge, FlowNode, NodeKind, Outcome, Run, RunStatus, RunSummary, Step,
    };
    use serde_json::Value;
    use std::path::PathBuf;

    fn make_node(id: &str, kind: NodeKind) -> FlowNode {
        FlowNode {
            id: id.to_string(),
            kind,
            summary: None,
            is_shell: false,
        }
    }

    fn make_step(node_id: &str, attempt: usize, kind: NodeKind, outcome: Outcome) -> Step {
        Step {
            attempt_id: format!("{node_id}#{attempt}"),
            node_id: node_id.to_string(),
            kind,
            outcome,
            started_ms: 1000,
            finished_ms: 2000,
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

    fn make_run(
        status: RunStatus,
        current_node: Option<&str>,
        waiting_on: Option<&str>,
        nodes: Vec<FlowNode>,
        edges: Vec<FlowEdge>,
        steps: Vec<Step>,
    ) -> Run {
        let start_at = nodes.first().map(|n| n.id.clone()).unwrap_or_default();
        Run {
            summary: RunSummary {
                run_id: "run-1".to_string(),
                dir: PathBuf::from("/tmp/run-1"),
                flow_name: "test-flow".to_string(),
                title: "Test Flow".to_string(),
                status,
                started_ms: 1000,
                updated_ms: 2000,
                finished_ms: None,
                current_node: current_node.map(str::to_string),
                waiting_on: waiting_on.map(str::to_string),
                status_detail: None,
                error: None,
            },
            flow: FlowDef {
                name: "test-flow".to_string(),
                start_at,
                nodes,
                edges,
            },
            steps,
            events: Vec::new(),
            input: Value::Null,
            sessions: Vec::new(),
        }
    }

    #[test]
    fn test_linear_flow_all_done() {
        let run = make_run(
            RunStatus::Completed,
            None,
            None,
            vec![
                make_node("init", NodeKind::Action),
                make_node("plan", NodeKind::Acp),
                make_node("finish", NodeKind::Compute),
            ],
            vec![
                FlowEdge::Simple {
                    from: "init".to_string(),
                    to: "plan".to_string(),
                },
                FlowEdge::Simple {
                    from: "plan".to_string(),
                    to: "finish".to_string(),
                },
            ],
            vec![
                make_step("init", 1, NodeKind::Action, Outcome::Ok),
                make_step("plan", 1, NodeKind::Acp, Outcome::Ok),
                make_step("finish", 1, NodeKind::Compute, Outcome::Ok),
            ],
        );

        let src = mermaid_source(&run, Direction::LeftRight);
        assert_eq!(
            src.lines().collect::<Vec<_>>(),
            vec![
                "flowchart LR",
                "  n0[\"✓ init\"]",
                "  n1(\"✓ plan\")",
                "  n2[\"✓ finish\"]",
                "  n0 ==> n1",
                "  n1 ==> n2",
            ]
        );
    }

    #[test]
    fn test_switch_with_one_case_taken() {
        let run = make_run(
            RunStatus::Completed,
            None,
            None,
            vec![
                make_node("size", NodeKind::Compute),
                make_node("fast", NodeKind::Action),
                make_node("deep", NodeKind::Acp),
            ],
            vec![FlowEdge::Switch {
                from: "size".to_string(),
                on: "bucket".to_string(),
                cases: vec![
                    ("large".to_string(), "deep".to_string()),
                    ("small".to_string(), "fast".to_string()),
                ],
            }],
            vec![
                make_step("size", 1, NodeKind::Compute, Outcome::Ok),
                make_step("fast", 1, NodeKind::Action, Outcome::Ok),
            ],
        );

        let src = mermaid_source(&run, Direction::LeftRight);
        assert_eq!(
            src.lines().collect::<Vec<_>>(),
            vec![
                "flowchart LR",
                "  n0[\"✓ size\"]",
                "  n1[\"✓ fast\"]",
                "  n2(\"○ deep\")",
                "  n0 -.->|\"large\"| n2",
                "  n0 ==>|\"small\"| n1",
            ]
        );
    }

    #[test]
    fn test_node_currently_running() {
        let run = make_run(
            RunStatus::Running,
            Some("work"),
            None,
            vec![
                make_node("prep", NodeKind::Action),
                make_node("work", NodeKind::Acp),
            ],
            vec![FlowEdge::Simple {
                from: "prep".to_string(),
                to: "work".to_string(),
            }],
            vec![make_step("prep", 1, NodeKind::Action, Outcome::Ok)],
        );

        let src = mermaid_source(&run, Direction::LeftRight);
        assert_eq!(
            src.lines().collect::<Vec<_>>(),
            vec![
                "flowchart LR",
                "  n0[\"✓ prep\"]",
                "  n1(\"● work\")",
                "  n0 -.-> n1",
            ]
        );
    }

    #[test]
    fn test_waiting_checkpoint() {
        let run = make_run(
            RunStatus::Waiting,
            Some("gate"),
            Some("gate"),
            vec![
                make_node("build", NodeKind::Action),
                make_node("gate", NodeKind::Checkpoint),
                make_node("deploy", NodeKind::Action),
            ],
            vec![
                FlowEdge::Simple {
                    from: "build".to_string(),
                    to: "gate".to_string(),
                },
                FlowEdge::Simple {
                    from: "gate".to_string(),
                    to: "deploy".to_string(),
                },
            ],
            vec![make_step("build", 1, NodeKind::Action, Outcome::Ok)],
        );

        let src = mermaid_source(&run, Direction::LeftRight);
        assert_eq!(
            src.lines().collect::<Vec<_>>(),
            vec![
                "flowchart LR",
                "  n0[\"✓ build\"]",
                "  n1{\"◐ gate\"}",
                "  n2[\"○ deploy\"]",
                "  n0 -.-> n1",
                "  n1 --> n2",
            ]
        );
    }

    #[test]
    fn test_nothing_run_yet() {
        let run = make_run(
            RunStatus::Running,
            None,
            None,
            vec![
                make_node("a", NodeKind::Action),
                make_node("b", NodeKind::Unknown),
                make_node("c", NodeKind::Acp),
            ],
            vec![
                FlowEdge::Simple {
                    from: "a".to_string(),
                    to: "b".to_string(),
                },
                FlowEdge::Switch {
                    from: "b".to_string(),
                    on: "res".to_string(),
                    cases: vec![("ok".to_string(), "c".to_string())],
                },
            ],
            vec![],
        );

        let src = mermaid_source(&run, Direction::LeftRight);
        assert_eq!(
            src.lines().collect::<Vec<_>>(),
            vec![
                "flowchart LR",
                "  n0[\"○ a\"]",
                "  n1[\"○ b\"]",
                "  n2(\"○ c\")",
                "  n0 --> n1",
                "  n1 -->|\"ok\"| n2",
            ]
        );
    }

    #[test]
    fn test_label_escaping_special_characters() {
        let tricky_id = "a\"b[c]d{e}f|g<h>i`j";
        let tricky_case = "x\"y[z]{w}|<v>`u";
        let run = make_run(
            RunStatus::Failed,
            None,
            None,
            vec![
                make_node(tricky_id, NodeKind::Action),
                make_node("end", NodeKind::Action),
            ],
            vec![FlowEdge::Switch {
                from: tricky_id.to_string(),
                on: "k".to_string(),
                cases: vec![(tricky_case.to_string(), "end".to_string())],
            }],
            vec![make_step(tricky_id, 1, NodeKind::Action, Outcome::Failed)],
        );

        let src = mermaid_source(&run, Direction::LeftRight);
        assert_eq!(
            src.lines().collect::<Vec<_>>(),
            vec![
                "flowchart LR",
                "  n0[\"✗ a'b c d e f g h i j\"]",
                "  n1[\"○ end\"]",
                "  n0 -.->|\"x'y z  w   v  u\"| n1",
            ]
        );
    }

    #[test]
    fn test_top_down_header() {
        let run = make_run(
            RunStatus::Completed,
            None,
            None,
            vec![make_node("only", NodeKind::Compute)],
            vec![],
            vec![make_step("only", 1, NodeKind::Compute, Outcome::Ok)],
        );

        let src = mermaid_source(&run, Direction::TopDown);
        assert_eq!(
            src.lines().collect::<Vec<_>>(),
            vec!["flowchart TD", "  n0[\"✓ only\"]"]
        );
    }

    #[test]
    fn test_edge_to_unknown_node_skipped() {
        let run = make_run(
            RunStatus::Completed,
            None,
            None,
            vec![
                make_node("a", NodeKind::Action),
                make_node("b", NodeKind::Action),
            ],
            vec![
                FlowEdge::Simple {
                    from: "a".to_string(),
                    to: "ghost".to_string(),
                },
                FlowEdge::Simple {
                    from: "ghost".to_string(),
                    to: "b".to_string(),
                },
                FlowEdge::Switch {
                    from: "a".to_string(),
                    on: "v".to_string(),
                    cases: vec![
                        ("missing".to_string(), "ghost".to_string()),
                        ("present".to_string(), "b".to_string()),
                    ],
                },
                FlowEdge::Switch {
                    from: "ghost".to_string(),
                    on: "v".to_string(),
                    cases: vec![("any".to_string(), "b".to_string())],
                },
            ],
            vec![
                make_step("a", 1, NodeKind::Action, Outcome::Ok),
                make_step("b", 1, NodeKind::Action, Outcome::Ok),
            ],
        );

        let src = mermaid_source(&run, Direction::LeftRight);
        assert_eq!(
            src.lines().collect::<Vec<_>>(),
            vec![
                "flowchart LR",
                "  n0[\"✓ a\"]",
                "  n1[\"✓ b\"]",
                "  n0 ==>|\"present\"| n1",
            ]
        );
    }

    #[test]
    fn test_node_ran_twice_loop_yields_one_node_line() {
        let run = make_run(
            RunStatus::Completed,
            None,
            None,
            vec![
                make_node("try", NodeKind::Acp),
                make_node("check", NodeKind::Compute),
            ],
            vec![
                FlowEdge::Simple {
                    from: "try".to_string(),
                    to: "check".to_string(),
                },
                FlowEdge::Switch {
                    from: "check".to_string(),
                    on: "ok".to_string(),
                    cases: vec![("retry".to_string(), "try".to_string())],
                },
            ],
            vec![
                make_step("try", 1, NodeKind::Acp, Outcome::Failed),
                make_step("check", 1, NodeKind::Compute, Outcome::Ok),
                make_step("try", 2, NodeKind::Acp, Outcome::Ok),
                make_step("check", 2, NodeKind::Compute, Outcome::Ok),
            ],
        );

        let src = mermaid_source(&run, Direction::LeftRight);
        assert_eq!(
            src.lines().collect::<Vec<_>>(),
            vec![
                "flowchart LR",
                "  n0(\"✓ try\")",
                "  n1[\"✓ check\"]",
                "  n0 ==> n1",
                "  n1 ==>|\"retry\"| n0",
            ]
        );
    }
}
```