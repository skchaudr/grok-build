//! Trajectory outline: one session's events as a `FoldTree`. A turn starts at each user message
//! (events before the first one form a synthetic turn), runs of consecutive tool calls are grouped,
//! and every message or tool event gets a `Body` child holding its full text.
//!
//! `items[i]` describes `tree.nodes[i]`, in preorder.

use xai_grok_traces::EventRow;

use super::traces_fold::{FoldNode, FoldTree};

/// Runs of at least this many consecutive tool calls are grouped.
const MIN_RUN: usize = 3;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum OutlineKey {
    /// Line of the turn's first event (the user message, or the first event of the synthetic turn).
    Turn(i64),
    /// Line of the run's first call.
    Run(i64),
    Ev(i64),
    Body(i64),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum OutlineItem {
    /// `event` indexes the turn's first event; `tools` counts visible tool calls, `msgs` visible
    /// user/assistant messages after the opener, `errors` visible rows that failed.
    Turn {
        event: usize,
        tools: usize,
        msgs: usize,
        errors: usize,
    },
    Run {
        first: usize,
        count: usize,
        errors: usize,
    },
    Event(usize),
    Body(usize),
}

/// Same classification as `traces_modal.rs` (`is_result`, `is_call`, the USER chip).
fn is_user(e: &EventRow) -> bool {
    e.role.as_deref() == Some("user") && !e.noise
}

fn is_result(e: &EventRow) -> bool {
    e.role.as_deref() == Some("tool_result")
}

fn is_call(e: &EventRow) -> bool {
    e.kind == "tool" && e.role.as_deref() == Some("tool")
}

/// A visible user or assistant message; injected (noise) context does not count.
fn is_message(e: &EventRow) -> bool {
    matches!(e.role.as_deref(), Some("user" | "assistant")) && !e.noise
}

fn has_body(e: &EventRow) -> bool {
    is_message(e) || is_call(e) || is_result(e)
}

/// Builds the outline of `events`. `hidden[i]` marks a tool result already folded into the call row
/// before it; hidden rows are skipped, and an error on one is charged to the visible row it folds into.
// `events[..]` lookups use event indices from this fn's own enumerate/turn bookkeeping, always in bounds.
#[allow(clippy::indexing_slicing)]
pub fn outline(events: &[EventRow], hidden: &[bool]) -> (FoldTree<OutlineKey>, Vec<OutlineItem>) {
    let is_hidden = |i: usize| hidden.get(i).copied().unwrap_or(false);

    // Visible row failed: its own error, or a folded result that errored.
    let mut failed = vec![false; events.len()];
    let mut owner = None;
    for (i, e) in events.iter().enumerate() {
        if is_hidden(i) {
            if let (Some(o), true) = (owner, e.is_error) {
                if let Some(f) = failed.get_mut(o) {
                    *f = true;
                }
            }
        } else {
            if let Some(f) = failed.get_mut(i) {
                *f = e.is_error;
            }
            owner = Some(i);
        }
    }
    let failed_at = |j: usize| failed.get(j).copied().unwrap_or(false);

    // Turns: each opens at a visible user message; leading non-user events open a synthetic turn.
    let mut turns: Vec<Vec<usize>> = Vec::new();
    for (i, e) in events.iter().enumerate().filter(|(i, _)| !is_hidden(*i)) {
        if turns.is_empty() || is_user(e) {
            turns.push(Vec::new());
        }
        turns.last_mut().expect("pushed above").push(i);
    }

    let mut nodes = Vec::new();
    let mut items = Vec::new();
    for turn in &turns {
        let Some(&head) = turn.first() else {
            continue;
        };
        let opener = is_user(&events[head]);
        let rest = turn.get(usize::from(opener)..).unwrap_or_default();
        let turn_node = push(
            &mut nodes,
            &mut items,
            OutlineKey::Turn(events[head].line),
            None,
            0,
            OutlineItem::Turn {
                event: head,
                tools: turn.iter().filter(|&&i| is_call(&events[i])).count(),
                msgs: rest.iter().filter(|&&i| is_message(&events[i])).count(),
                errors: turn.iter().filter(|&&i| failed_at(i)).count(),
            },
        );
        if opener {
            // The opener is the turn's first child; its body stays collapsed until expanded.
            push_event(&mut nodes, &mut items, events, head, turn_node);
        }

        let mut k = 0;
        while let Some(&i) = rest.get(k) {
            if !is_call(&events[i]) {
                push_event(&mut nodes, &mut items, events, i, turn_node);
                k += 1;
                continue;
            }
            let end = rest
                .get(k..)
                .unwrap_or_default()
                .iter()
                .position(|&j| !is_call(&events[j]))
                .map_or(rest.len(), |p| k + p);
            let run = rest.get(k..end).unwrap_or_default();
            match run.first() {
                Some(&first) if run.len() >= MIN_RUN => {
                    let item = OutlineItem::Run {
                        first,
                        count: run.len(),
                        errors: run.iter().filter(|&&j| failed_at(j)).count(),
                    };
                    let run_node = push(
                        &mut nodes,
                        &mut items,
                        OutlineKey::Run(events[first].line),
                        Some(turn_node),
                        1,
                        item,
                    );
                    for &c in run {
                        push_event(&mut nodes, &mut items, events, c, run_node);
                    }
                }
                _ => {
                    for &c in run {
                        push_event(&mut nodes, &mut items, events, c, turn_node);
                    }
                }
            }
            k = end;
        }
    }
    (FoldTree { nodes }, items)
}

fn push(
    nodes: &mut Vec<FoldNode<OutlineKey>>,
    items: &mut Vec<OutlineItem>,
    key: OutlineKey,
    parent: Option<usize>,
    depth: usize,
    item: OutlineItem,
) -> usize {
    nodes.push(FoldNode { key, parent, depth });
    items.push(item);
    nodes.len() - 1
}

/// An event row, with its `Body` child when it is a message or tool event.
// `parent` is a node index returned by `push`; `i` is an event index from `outline`. Both in bounds.
#[allow(clippy::indexing_slicing)]
fn push_event(
    nodes: &mut Vec<FoldNode<OutlineKey>>,
    items: &mut Vec<OutlineItem>,
    events: &[EventRow],
    i: usize,
    parent: usize,
) {
    let depth = nodes[parent].depth + 1;
    let node = push(
        nodes,
        items,
        OutlineKey::Ev(events[i].line),
        Some(parent),
        depth,
        OutlineItem::Event(i),
    );
    if has_body(&events[i]) {
        push_body(nodes, items, events, i, node);
    }
}

// `parent` is a node index returned by `push`; `i` is an event index from `outline`. Both in bounds.
#[allow(clippy::indexing_slicing)]
fn push_body(
    nodes: &mut Vec<FoldNode<OutlineKey>>,
    items: &mut Vec<OutlineItem>,
    events: &[EventRow],
    i: usize,
    parent: usize,
) {
    let depth = nodes[parent].depth + 1;
    push(
        nodes,
        items,
        OutlineKey::Body(events[i].line),
        Some(parent),
        depth,
        OutlineItem::Body(i),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(line: i64, kind: &str, role: Option<&str>) -> EventRow {
        EventRow {
            line,
            event_type: "x".into(),
            kind: kind.into(),
            role: role.map(Into::into),
            title: String::new(),
            preview: String::new(),
            timestamp: None,
            duration_ms: None,
            input_tokens: None,
            output_tokens: None,
            turn: None,
            step: None,
            depth: 0,
            model: None,
            is_error: false,
            noise: false,
            detail: None,
        }
    }

    fn user(line: i64) -> EventRow {
        row(line, "message", Some("user"))
    }

    fn says(line: i64) -> EventRow {
        row(line, "message", Some("assistant"))
    }

    fn call(line: i64) -> EventRow {
        EventRow {
            title: "Bash".into(),
            ..row(line, "tool", Some("tool"))
        }
    }

    fn result(line: i64, error: bool) -> EventRow {
        EventRow {
            is_error: error,
            ..row(line, "tool", Some("tool_result"))
        }
    }

    /// Hidden flags exactly as `set_events` computes them: a result right after a call (not already
    /// folded into it) is folded.
    fn folded(events: &[EventRow]) -> Vec<bool> {
        let mut hidden = vec![false; events.len()];
        for i in 1..events.len() {
            if is_result(&events[i]) && is_call(&events[i - 1]) && !hidden[i - 1] {
                hidden[i] = true;
            }
        }
        hidden
    }

    fn run(events: &[EventRow]) -> (FoldTree<OutlineKey>, Vec<OutlineItem>) {
        let hidden = folded(events);
        let (tree, items) = outline(events, &hidden);
        check(events, &tree, &items);
        (tree, items)
    }

    /// Preorder and parent/depth invariants, plus each key and item matching the event it names.
    fn check(events: &[EventRow], tree: &FoldTree<OutlineKey>, items: &[OutlineItem]) {
        assert_eq!(tree.nodes.len(), items.len(), "one item per node");
        let mut stack: Vec<usize> = Vec::new();
        for (i, n) in tree.nodes.iter().enumerate() {
            while stack
                .last()
                .is_some_and(|&t| tree.nodes[t].depth >= n.depth)
            {
                stack.pop();
            }
            assert_eq!(n.parent, stack.last().copied(), "parent of node {i}");
            assert_eq!(
                n.depth,
                stack.len(),
                "depth of node {i} is its ancestor count"
            );
            stack.push(i);
            let line = |e: usize| events[e].line;
            let ok = match (&n.key, &items[i]) {
                (OutlineKey::Turn(l), OutlineItem::Turn { event, .. }) => *l == line(*event),
                (OutlineKey::Run(l), OutlineItem::Run { first, .. }) => *l == line(*first),
                (OutlineKey::Ev(l), OutlineItem::Event(e))
                | (OutlineKey::Body(l), OutlineItem::Body(e)) => *l == line(*e),
                _ => false,
            };
            assert!(ok, "key {:?} does not match item {:?}", n.key, items[i]);
        }
        let keys: std::collections::HashSet<_> = tree.nodes.iter().map(|n| n.key.clone()).collect();
        assert_eq!(keys.len(), tree.nodes.len(), "keys are unique");
    }

    fn turns(items: &[OutlineItem]) -> Vec<(usize, usize, usize, usize)> {
        items
            .iter()
            .filter_map(|it| match it {
                OutlineItem::Turn {
                    event,
                    tools,
                    msgs,
                    errors,
                } => Some((*event, *tools, *msgs, *errors)),
                _ => None,
            })
            .collect()
    }

    fn kinds(items: &[OutlineItem]) -> Vec<&'static str> {
        items
            .iter()
            .map(|it| match it {
                OutlineItem::Turn { .. } => "turn",
                OutlineItem::Run { .. } => "run",
                OutlineItem::Event(_) => "ev",
                OutlineItem::Body(_) => "body",
            })
            .collect()
    }

    #[test]
    fn empty_trajectory_is_empty() {
        let (tree, items) = run(&[]);
        assert!(tree.nodes.is_empty() && items.is_empty());
    }

    #[test]
    fn turns_split_on_user_messages() {
        let ev = [user(1), says(2), user(3), says(4), says(5)];
        let (tree, items) = run(&ev);
        assert_eq!(turns(&items), vec![(0, 0, 1, 0), (2, 0, 2, 0)]);
        let roots: Vec<_> = tree
            .nodes
            .iter()
            .filter(|n| n.parent.is_none())
            .map(|n| n.key.clone())
            .collect();
        assert_eq!(roots, vec![OutlineKey::Turn(1), OutlineKey::Turn(3)]);
    }

    #[test]
    fn opener_is_the_turn_first_child_with_its_body_under_it() {
        let ev = [user(1), says(2)];
        let (tree, _) = run(&ev);
        let turn = tree.nodes.iter().position(|n| n.key == OutlineKey::Turn(1)).unwrap();
        assert_eq!(tree.nodes[turn + 1].key, OutlineKey::Ev(1));
        assert_eq!((tree.nodes[turn + 1].parent, tree.nodes[turn + 1].depth), (Some(turn), 1));
        assert_eq!(tree.nodes[turn + 2].key, OutlineKey::Body(1));
        assert_eq!(tree.nodes[turn + 2].parent, Some(turn + 1));
        assert!(tree.has_children(turn + 1), "the opener folds to hide its body");
    }

    #[test]
    fn leading_events_form_a_synthetic_turn() {
        let ev = [says(1), call(2), user(3), says(4)];
        let (tree, items) = run(&ev);
        // Synthetic turn keyed by its first event (line 1), with no opener body of its own.
        assert_eq!(turns(&items), vec![(0, 1, 1, 0), (2, 0, 1, 0)]);
        assert_eq!(tree.nodes[0].key, OutlineKey::Turn(1));
        assert!(
            tree.nodes
                .iter()
                .all(|n| n.key != OutlineKey::Body(1) || n.depth == 2)
        );
    }

    #[test]
    fn no_user_message_is_one_synthetic_turn() {
        let ev = [says(1), says(2)];
        let (_, items) = run(&ev);
        assert_eq!(turns(&items), vec![(0, 0, 2, 0)]);
    }

    #[test]
    fn noise_user_message_does_not_start_a_turn() {
        let mut injected = user(2);
        injected.noise = true;
        let ev = [user(1), injected, says(3)];
        let (_, items) = run(&ev);
        assert_eq!(turns(&items), vec![(0, 0, 1, 0)]);
    }

    #[test]
    fn three_consecutive_calls_group_into_a_run() {
        let ev = [
            user(1),
            call(2),
            result(3, false),
            call(4),
            result(5, false),
            call(6),
            result(7, false),
        ];
        let (tree, items) = run(&ev);
        assert_eq!(turns(&items), vec![(0, 3, 0, 0)]);
        assert_eq!(
            kinds(&items),
            vec![
                "turn", "ev", "body", "run", "ev", "body", "ev", "body", "ev", "body"
            ]
        );
        assert!(items.contains(&OutlineItem::Run {
            first: 1,
            count: 3,
            errors: 0
        }));
        let run_node = tree
            .nodes
            .iter()
            .position(|n| matches!(n.key, OutlineKey::Run(_)))
            .unwrap();
        assert_eq!(
            (tree.nodes[run_node].depth, tree.nodes[run_node].parent),
            (1, Some(0))
        );
        let calls = tree
            .nodes
            .iter()
            .filter(|n| n.parent == Some(run_node) && n.depth == 2)
            .count();
        assert_eq!(calls, 3, "each grouped call is a direct child of the run");
    }

    #[test]
    fn two_calls_stay_as_plain_events() {
        let ev = [
            user(1),
            call(2),
            result(3, false),
            call(4),
            result(5, false),
        ];
        let (_, items) = run(&ev);
        assert!(
            items
                .iter()
                .all(|it| !matches!(it, OutlineItem::Run { .. }))
        );
        assert_eq!(
            kinds(&items),
            vec!["turn", "ev", "body", "ev", "body", "ev", "body"]
        );
    }

    #[test]
    fn a_visible_message_breaks_a_run() {
        // Two calls before the text and two after: neither side reaches MIN_RUN.
        let ev = [user(1), call(2), call(3), says(4), call(5), call(6)];
        let (_, items) = run(&ev);
        assert!(
            items
                .iter()
                .all(|it| !matches!(it, OutlineItem::Run { .. }))
        );
    }

    #[test]
    fn a_visible_result_breaks_a_run() {
        // Without the unfolded result at index 4, calls 2, 4, 6, 8 would be one run of four.
        let ev = [
            user(1),
            call(2),
            result(3, false),
            call(4),
            result(5, false),
            call(6),
            result(7, false),
            call(8),
            result(9, false),
        ];
        let hidden = [false, false, true, false, false, false, true, false, true];
        let (tree, items) = outline(&ev, &hidden);
        check(&ev, &tree, &items);
        assert!(
            items
                .iter()
                .all(|it| !matches!(it, OutlineItem::Run { .. }))
        );
        assert!(
            items.contains(&OutlineItem::Event(4)),
            "the visible result stays a row"
        );
        assert!(
            items
                .iter()
                .all(|it| !matches!(it, OutlineItem::Event(2) | OutlineItem::Body(2))),
            "folded result 2 is skipped"
        );
    }

    #[test]
    fn hidden_results_are_skipped_entirely() {
        let ev = [user(1), call(2), result(3, true), says(4)];
        let (tree, items) = run(&ev);
        assert!(
            items
                .iter()
                .all(|it| !matches!(it, OutlineItem::Event(2) | OutlineItem::Body(2)))
        );
        assert!(
            tree.nodes
                .iter()
                .all(|n| n.key != OutlineKey::Ev(3) && n.key != OutlineKey::Body(3))
        );
    }

    #[test]
    fn short_hidden_slice_treats_the_rest_as_visible() {
        let ev = [user(1), call(2), result(3, false)];
        let (_, items) = outline(&ev, &[]);
        assert!(items.contains(&OutlineItem::Event(2)));
    }

    #[test]
    fn bodies_follow_message_and_tool_events() {
        let ev = [
            user(1),
            says(2),
            call(3),
            result(4, false),
            row(5, "telemetry", None),
            row(6, "step", None),
        ];
        let (tree, items) = run(&ev);
        // Opener, assistant and call bodies; the folded result has no node of its own; telemetry and
        // step rows have no body.
        let bodies: Vec<_> = items
            .iter()
            .filter_map(|it| {
                if let OutlineItem::Body(e) = it {
                    Some(*e)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(bodies, vec![0, 1, 2]);
        let parents: Vec<_> = tree
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| matches!(n.key, OutlineKey::Body(_)))
            .map(|(i, _)| i)
            .collect();
        assert!(
            parents.iter().all(
                |&i| tree.nodes[tree.nodes[i].parent.unwrap()].depth == tree.nodes[i].depth - 1
            )
        );
        assert_eq!(
            kinds(&items),
            vec!["turn", "ev", "body", "ev", "body", "ev", "body", "ev", "ev"]
        );
    }

    #[test]
    fn unfolded_result_gets_a_body() {
        let ev = [user(1), result(2, true)];
        let hidden = [false, false];
        let (tree, items) = outline(&ev, &hidden);
        check(&ev, &tree, &items);
        assert!(items.contains(&OutlineItem::Body(1)));
    }

    #[test]
    fn errors_count_failed_rows_once() {
        // c4 failed itself; c6 failed only through its folded result; c2 and c8 are clean.
        let ev = [
            user(1),
            call(2),
            result(3, false),
            EventRow {
                is_error: true,
                ..call(4)
            },
            result(5, true),
            call(6),
            result(7, true),
            call(8),
            result(9, false),
        ];
        let (_, items) = run(&ev);
        assert_eq!(turns(&items), vec![(0, 4, 0, 2)]);
        assert!(items.contains(&OutlineItem::Run {
            first: 1,
            count: 4,
            errors: 2
        }));
    }

    #[test]
    fn failed_message_counts_on_its_turn() {
        let mut bad = says(3);
        bad.is_error = true;
        let ev = [user(1), says(2), bad, user(4), call(5)];
        let (_, items) = run(&ev);
        assert_eq!(turns(&items), vec![(0, 0, 2, 1), (3, 1, 0, 0)]);
    }

    #[test]
    fn errors_only_within_their_turn() {
        let mut bad = call(3);
        bad.is_error = true;
        let ev = [user(1), bad, user(4), call(5)];
        let (_, items) = run(&ev);
        assert_eq!(turns(&items), vec![(0, 1, 0, 1), (2, 1, 0, 0)]);
    }

    #[test]
    fn keys_survive_live_growth() {
        let base = [
            user(1),
            call(2),
            result(3, false),
            call(4),
            result(5, false),
            call(6),
            result(7, false),
        ];
        let (before, _) = run(&base);
        let grown = [&base[..], &[call(8), result(9, false)][..]].concat();
        let (after, items) = run(&grown);
        // Appending a call keeps every existing key, including the run keyed by its first line.
        assert!(
            before
                .nodes
                .iter()
                .all(|b| after.nodes.iter().any(|a| a.key == b.key))
        );
        assert!(items.contains(&OutlineItem::Run {
            first: 1,
            count: 4,
            errors: 0
        }));
    }

    #[test]
    fn mixed_trajectory_satisfies_tree_invariants() {
        let ev = [
            says(1),
            call(2),
            call(3),
            result(4, false),
            user(5),
            call(6),
            result(7, true),
            call(8),
            result(9, false),
            call(10),
            result(11, false),
            says(12),
            user(13),
            says(14),
            call(15),
            result(16, false),
        ];
        let (tree, items) = run(&ev);
        assert_eq!(turns(&items).len(), 3);
        // Turns are the roots; every other node's parent precedes it (preorder).
        assert!(
            tree.nodes
                .iter()
                .enumerate()
                .all(|(i, n)| n.parent.map_or(n.depth == 0, |p| p < i))
        );
    }
}
