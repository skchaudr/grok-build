## Edit 1: Track seen finished attempts in RunViewState
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs
OLD:
~~~rust
/// Interactive state for the single-run screen.
pub struct RunViewState {
    run: Run,
    node_settings: HashMap<String, AgentSettings>,
    step_settings: HashMap<String, AgentSettings>,
    tree: FoldTree<String>,
    payloads: Vec<NodePayload>,
    fold: FoldState<String>,
    /// Index into `visible` (the output of `tree.visible(&fold)`).
    selected: usize,
    scroll: usize,
    last_pane_height: usize,
    show_graph: bool,
    show_noise: bool,
    detail_rect: Option<Rect>,
    gate_confirm: Option<GateConfirm>,
    graph_cache: Option<GraphCache>,
    answered: Option<String>,
    zoom: ModalZoom,
    graph_scroll: usize,
}

impl RunViewState {
    pub fn new(run: Run) -> Self {
        let mut st = Self {
            run,
            node_settings: HashMap::new(),
            step_settings: HashMap::new(),
            tree: FoldTree { nodes: Vec::new() },
            payloads: Vec::new(),
            fold: FoldState::new(),
            selected: 0,
            scroll: 0,
            last_pane_height: 20,
            show_graph: true,
            show_noise: false,
            detail_rect: None,
            gate_confirm: None,
            graph_cache: None,
            answered: None,
            zoom: ModalZoom::Split,
            graph_scroll: 0,
        };
        st.rebuild_tree();
        st.select_initial_stage();
        st
    }
~~~
NEW:
~~~rust
/// Interactive state for the single-run screen.
pub struct RunViewState {
    run: Run,
    node_settings: HashMap<String, AgentSettings>,
    step_settings: HashMap<String, AgentSettings>,
    tree: FoldTree<String>,
    payloads: Vec<NodePayload>,
    fold: FoldState<String>,
    seen_finished_attempts: HashSet<String>,
    /// Index into `visible` (the output of `tree.visible(&fold)`).
    selected: usize,
    scroll: usize,
    last_pane_height: usize,
    show_graph: bool,
    show_noise: bool,
    detail_rect: Option<Rect>,
    gate_confirm: Option<GateConfirm>,
    graph_cache: Option<GraphCache>,
    answered: Option<String>,
    zoom: ModalZoom,
    graph_scroll: usize,
}

impl RunViewState {
    pub fn new(run: Run) -> Self {
        let mut st = Self {
            run,
            node_settings: HashMap::new(),
            step_settings: HashMap::new(),
            tree: FoldTree { nodes: Vec::new() },
            payloads: Vec::new(),
            fold: FoldState::new(),
            seen_finished_attempts: HashSet::new(),
            selected: 0,
            scroll: 0,
            last_pane_height: 20,
            show_graph: true,
            show_noise: false,
            detail_rect: None,
            gate_confirm: None,
            graph_cache: None,
            answered: None,
            zoom: ModalZoom::Split,
            graph_scroll: 0,
        };
        st.rebuild_tree();
        st.select_initial_stage();
        st
    }
~~~

## Edit 2: Collapse newly finished attempts once, distinguish live loop attempts, and prune skipped nodes
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs
OLD:
~~~rust
    fn rebuild_tree(&mut self) {
        let mut nodes = Vec::new();
        let mut payloads = Vec::new();

        let is_waiting = self.run.summary.status == RunStatus::Waiting;
        let is_running = self.run.summary.status == RunStatus::Running;
        let waiting_node = self.run.summary.waiting_on.as_deref();
        let current_node = self.run.summary.current_node.as_deref();

        // Find if the waiting node already has a step in `run.steps` (pick the last one).
        let waiting_step_idx: Option<usize> = if is_waiting {
            waiting_node.and_then(|wn| self.run.steps.iter().rposition(|s| s.node_id == wn))
        } else {
            None
        };

        // Determine if we need a live/<node> root:
        // - Running node that has no step record, or is currently active without a finished step for this run
        //   Spec: "a root for a node that is Running but has no step record (key = `"live/<node id>"`).
        //   The WAITING stage is the last root whose node id equals `run.summary.waiting_on`: acpx records
        //   the checkpoint step BEFORE the run goes waiting, so it is normally a step root; use a `live/…`
        //   root for it only when no such step exists"
        let live_root_node: Option<(String, NodeState)> = if is_waiting {
            if waiting_step_idx.is_none() {
                waiting_node.map(|wn| (wn.to_string(), self.run.node_state(wn)))
            } else {
                None
            }
        } else if is_running {
            current_node.and_then(|cn| {
                let has_step = self.run.steps.iter().any(|s| s.node_id == cn);
                if !has_step {
                    Some((cn.to_string(), self.run.node_state(cn)))
                } else {
                    None
                }
            })
        } else {
            None
        };

        let mut stage_num = 0usize;

        // a. One root per step in `run.steps` order
        for (step_idx, step) in self.run.steps.iter().enumerate() {
            stage_num += 1;
            let is_waiting_stage = waiting_step_idx == Some(step_idx);
            let state = if is_waiting_stage && self.answered.is_none() {
                NodeState::Waiting
            } else if is_waiting_stage {
                NodeState::Done(Outcome::Ok)
            } else {
                NodeState::Done(step.outcome)
            };

            let root_idx = nodes.len();
            nodes.push(FoldNode {
                key: step.attempt_id.clone(),
                parent: None,
                depth: 0,
            });
            payloads.push(NodePayload::StepStage {
                step_idx,
                stage_num,
                state,
            });

            // Trace event children
            let evs = self.run.step_events(&step.attempt_id);
            let start_ms = if step.started_ms > 0 {
                step.started_ms
            } else {
                evs.first().map_or(0, |e| e.at_ms)
            };
            for ev in evs {
                if !self.show_noise && matches!(ev.kind.as_str(), "node_heartbeat" | "artifact_written") {
                    continue;
                }
                let rel_ms = (ev.at_ms - start_ms).max(0);
                nodes.push(FoldNode {
                    key: format!("{}/{}", step.attempt_id, ev.seq),
                    parent: Some(root_idx),
                    depth: 1,
                });
                payloads.push(NodePayload::Event {
                    owner: StageOwner::Step(step_idx),
                    rel_ms,
                    kind: ev.kind.clone(),
                    detail: ev.detail.clone(),
                });
            }

            // Result row
            nodes.push(FoldNode {
                key: format!("{}/result", step.attempt_id),
                parent: Some(root_idx),
                depth: 1,
            });
            payloads.push(NodePayload::Result { step_idx });

            // Gate children if this step is the waiting stage
            if is_waiting_stage {
                Self::push_gate_children(
                    &self.run,
                    root_idx,
                    StageOwner::Step(step_idx),
                    &mut nodes,
                    &mut payloads,
                );
            }
        }

        // b. Live root (Running with no step record, or Waiting with no step record)
        if let Some((live_node_id, state)) = live_root_node {
            stage_num += 1;
            let root_idx = nodes.len();
            nodes.push(FoldNode {
                key: format!("live/{live_node_id}"),
                parent: None,
                depth: 0,
            });
            payloads.push(NodePayload::LiveStage {
                node_id: live_node_id.clone(),
                stage_num,
                state: if state == NodeState::Waiting && self.answered.is_some() {
                    NodeState::Done(Outcome::Ok)
                } else {
                    state
                },
            });

            let step_attempts: HashSet<&str> = self
                .run
                .steps
                .iter()
                .map(|s| s.attempt_id.as_str())
                .collect();
            let live_evs: Vec<&TraceEvent> = self
                .run
                .events
                .iter()
                .filter(|ev| {
                    ev.node_id.as_deref() == Some(live_node_id.as_str())
                        && ev
                            .attempt_id
                            .as_deref()
                            .is_none_or(|a| !step_attempts.contains(a))
                })
                .collect();
            let start_ms = live_evs
                .iter()
                .find(|e| e.kind == "node_started")
                .or_else(|| live_evs.first())
                .map_or(self.run.summary.started_ms, |e| e.at_ms);

            for ev in live_evs {
                if !self.show_noise && matches!(ev.kind.as_str(), "node_heartbeat" | "artifact_written") {
                    continue;
                }
                let rel_ms = (ev.at_ms - start_ms).max(0);
                nodes.push(FoldNode {
                    key: format!("live/{live_node_id}/{}", ev.seq),
                    parent: Some(root_idx),
                    depth: 1,
                });
                payloads.push(NodePayload::Event {
                    owner: StageOwner::Live(live_node_id.clone()),
                    rel_ms,
                    kind: ev.kind.clone(),
                    detail: ev.detail.clone(),
                });
            }

            if state == NodeState::Waiting {
                Self::push_gate_children(
                    &self.run,
                    root_idx,
                    StageOwner::Live(live_node_id),
                    &mut nodes,
                    &mut payloads,
                );
            }
        }

        // c. Skipped / NotRun flow nodes
        let skipped: Vec<(String, NodeKind)> = self
            .run
            .flow
            .nodes
            .iter()
            .filter(|n| self.run.node_state(&n.id) == NodeState::NotRun)
            .map(|n| (n.id.clone(), n.kind))
            .collect();
        if !skipped.is_empty() {
            let root_idx = nodes.len();
            nodes.push(FoldNode {
                key: "skipped".to_string(),
                parent: None,
                depth: 0,
            });
            payloads.push(NodePayload::SkippedHeader);
            for (nid, kind) in skipped {
                nodes.push(FoldNode {
                    key: format!("skip/{nid}"),
                    parent: Some(root_idx),
                    depth: 1,
                });
                payloads.push(NodePayload::SkippedNode { node_id: nid, kind });
            }
        }

        self.tree = FoldTree { nodes };
        self.payloads = payloads;
    }
~~~
NEW:
~~~rust
    fn is_live_step(step: &Step) -> bool {
        step.finished_ms <= 0 || step.finished_ms < step.started_ms
    }

    fn rebuild_tree(&mut self) {
        let mut nodes = Vec::new();
        let mut payloads = Vec::new();
        let mut newly_finished_roots = Vec::new();

        let is_waiting = self.run.summary.status == RunStatus::Waiting;
        let is_running = self.run.summary.status == RunStatus::Running;
        let waiting_node = self.run.summary.waiting_on.as_deref();
        let current_node = self.run.summary.current_node.as_deref();

        // Find if the waiting node already has a step in `run.steps` (pick the last one).
        let waiting_step_idx: Option<usize> = if is_waiting {
            waiting_node.and_then(|wn| self.run.steps.iter().rposition(|s| s.node_id == wn))
        } else {
            None
        };

        // Distinguish a live recorded step from earlier finished loop attempts by tail position
        // and unfinished timestamps.
        let running_step_idx: Option<usize> = if is_running {
            current_node.and_then(|cn| {
                let last_idx = self.run.steps.len().checked_sub(1)?;
                let last = &self.run.steps[last_idx];
                (last.node_id == cn && Self::is_live_step(last)).then_some(last_idx)
            })
        } else {
            None
        };

        // Determine if we need a live/<node> root:
        // - Running node that has no live step record at the tail of `run.steps`
        // - Waiting node that has no step record in `run.steps`
        let live_root_node: Option<(String, NodeState)> = if is_waiting {
            if waiting_step_idx.is_none() {
                waiting_node.map(|wn| (wn.to_string(), self.run.node_state(wn)))
            } else {
                None
            }
        } else if is_running {
            if running_step_idx.is_none() {
                current_node.map(|cn| (cn.to_string(), self.run.node_state(cn)))
            } else {
                None
            }
        } else {
            None
        };

        let mut stage_num = 0usize;

        // a. One root per step in `run.steps` order
        for (step_idx, step) in self.run.steps.iter().enumerate() {
            stage_num += 1;
            let is_waiting_stage = waiting_step_idx == Some(step_idx);
            let is_running_stage = running_step_idx == Some(step_idx);
            let state = if is_waiting_stage && self.answered.is_none() {
                NodeState::Waiting
            } else if is_waiting_stage {
                NodeState::Done(Outcome::Ok)
            } else if is_running_stage {
                NodeState::Running
            } else {
                NodeState::Done(step.outcome)
            };

            let root_idx = nodes.len();
            nodes.push(FoldNode {
                key: step.attempt_id.clone(),
                parent: None,
                depth: 0,
            });
            payloads.push(NodePayload::StepStage {
                step_idx,
                stage_num,
                state,
            });

            if !is_waiting_stage
                && !is_running_stage
                && self.seen_finished_attempts.insert(step.attempt_id.clone())
            {
                newly_finished_roots.push(root_idx);
            }

            // Trace event children
            let evs = self.run.step_events(&step.attempt_id);
            let start_ms = if step.started_ms > 0 {
                step.started_ms
            } else {
                evs.first().map_or(0, |e| e.at_ms)
            };
            for ev in evs {
                if !self.show_noise && matches!(ev.kind.as_str(), "node_heartbeat" | "artifact_written") {
                    continue;
                }
                let rel_ms = (ev.at_ms - start_ms).max(0);
                nodes.push(FoldNode {
                    key: format!("{}/{}", step.attempt_id, ev.seq),
                    parent: Some(root_idx),
                    depth: 1,
                });
                payloads.push(NodePayload::Event {
                    owner: StageOwner::Step(step_idx),
                    rel_ms,
                    kind: ev.kind.clone(),
                    detail: ev.detail.clone(),
                });
            }

            // Result row
            nodes.push(FoldNode {
                key: format!("{}/result", step.attempt_id),
                parent: Some(root_idx),
                depth: 1,
            });
            payloads.push(NodePayload::Result { step_idx });

            // Gate children if this step is the waiting stage
            if is_waiting_stage {
                Self::push_gate_children(
                    &self.run,
                    root_idx,
                    StageOwner::Step(step_idx),
                    &mut nodes,
                    &mut payloads,
                );
            }
        }

        // b. Live root (Running with no live step record, or Waiting with no step record)
        if let Some((live_node_id, state)) = live_root_node {
            stage_num += 1;
            let root_idx = nodes.len();
            nodes.push(FoldNode {
                key: format!("live/{live_node_id}"),
                parent: None,
                depth: 0,
            });
            payloads.push(NodePayload::LiveStage {
                node_id: live_node_id.clone(),
                stage_num,
                state: if state == NodeState::Waiting && self.answered.is_some() {
                    NodeState::Done(Outcome::Ok)
                } else {
                    state
                },
            });

            let step_attempts: HashSet<&str> = self
                .run
                .steps
                .iter()
                .map(|s| s.attempt_id.as_str())
                .collect();
            let live_evs: Vec<&TraceEvent> = self
                .run
                .events
                .iter()
                .filter(|ev| {
                    ev.node_id.as_deref() == Some(live_node_id.as_str())
                        && ev
                            .attempt_id
                            .as_deref()
                            .is_none_or(|a| !step_attempts.contains(a))
                })
                .collect();
            let start_ms = live_evs
                .iter()
                .find(|e| e.kind == "node_started")
                .or_else(|| live_evs.first())
                .map_or(self.run.summary.started_ms, |e| e.at_ms);

            for ev in live_evs {
                if !self.show_noise && matches!(ev.kind.as_str(), "node_heartbeat" | "artifact_written") {
                    continue;
                }
                let rel_ms = (ev.at_ms - start_ms).max(0);
                nodes.push(FoldNode {
                    key: format!("live/{live_node_id}/{}", ev.seq),
                    parent: Some(root_idx),
                    depth: 1,
                });
                payloads.push(NodePayload::Event {
                    owner: StageOwner::Live(live_node_id.clone()),
                    rel_ms,
                    kind: ev.kind.clone(),
                    detail: ev.detail.clone(),
                });
            }

            if state == NodeState::Waiting {
                Self::push_gate_children(
                    &self.run,
                    root_idx,
                    StageOwner::Live(live_node_id),
                    &mut nodes,
                    &mut payloads,
                );
            }
        }

        // c. Skipped / NotRun flow nodes (pruned to active branches)
        let pruned_run = prune_run_for_graph(&self.run);
        let skipped: Vec<(String, NodeKind)> = pruned_run
            .flow
            .nodes
            .into_iter()
            .filter(|n| self.run.node_state(&n.id) == NodeState::NotRun)
            .map(|n| (n.id, n.kind))
            .collect();
        if !skipped.is_empty() {
            let root_idx = nodes.len();
            nodes.push(FoldNode {
                key: "skipped".to_string(),
                parent: None,
                depth: 0,
            });
            payloads.push(NodePayload::SkippedHeader);
            for (nid, kind) in skipped {
                nodes.push(FoldNode {
                    key: format!("skip/{nid}"),
                    parent: Some(root_idx),
                    depth: 1,
                });
                payloads.push(NodePayload::SkippedNode { node_id: nid, kind });
            }
        }

        self.tree = FoldTree { nodes };
        self.payloads = payloads;
        for root_idx in newly_finished_roots {
            self.fold.collapse(&self.tree, root_idx);
        }
    }
~~~

## Edit 3: Update default-fold, child-collapse, and toggle-all tests
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs
OLD:
~~~rust
    #[test]
    fn stages_start_expanded_and_hide_heartbeats_until_noise_toggled() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        let keys_before: Vec<String> = st
            .visible()
            .into_iter()
            .map(|i| st.tree.nodes[i].key.clone())
            .collect();
        assert!(keys_before.contains(&"plan#1/1".to_string()));
        assert!(!keys_before.contains(&"plan#1/2".to_string()));
        assert!(keys_before.contains(&"plan#1/3".to_string()));

        assert_eq!(st.handle_key(&key(KeyCode::Char('n'))), RunOutcome::Changed);
        let keys_after: Vec<String> = st
            .visible()
            .into_iter()
            .map(|i| st.tree.nodes[i].key.clone())
            .collect();
        assert!(keys_after.contains(&"plan#1/2".to_string()));
    }

    #[test]
    fn collapse_on_child_jumps_to_parent() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        st.select_step(0);
        assert_eq!(st.selected, 0);
        // Move down onto the first child of plan#1
        st.handle_key(&key(KeyCode::Char('j')));
        assert_eq!(st.selected, 1);
        assert_eq!(st.detail_target(), DetailTarget::Step(0));
        // Left on a leaf child jumps back to parent root (index 0)
        st.handle_key(&key(KeyCode::Left));
        assert_eq!(st.selected, 0);
        // Left again collapses the root
        st.handle_key(&key(KeyCode::Char('h')));
        assert!(st.fold.is_collapsed(&"plan#1".to_string()));
    }

    #[test]
    fn shift_e_toggles_all_folds() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        let expanded_len = st.visible().len();
        st.handle_key(&key(KeyCode::Char('E')));
        let collapsed_len = st.visible().len();
        assert!(collapsed_len < expanded_len);
        // Only root headers remain visible (3 steps + 1 skipped root = 4)
        assert_eq!(collapsed_len, 4);
        st.handle_key(&key(KeyCode::Char('E')));
        assert_eq!(st.visible().len(), expanded_len);
    }
~~~
NEW:
~~~rust
    #[test]
    fn finished_stages_start_collapsed_and_preserve_expansion_on_noise_toggle() {
        let run = sample_waiting_run(json!({"next": {"approve": "land.flow.ts"}}));
        let mut st = RunViewState::new(run);
        let vis_keys = |st: &RunViewState| -> Vec<String> {
            st.visible()
                .into_iter()
                .map(|i| st.tree.nodes[i].key.clone())
                .collect()
        };

        assert_eq!(st.selected_step(), Some(2));
        let keys_initial = vis_keys(&st);
        // Finished attempts start collapsed; the waiting stage and its pending choice stay visible.
        assert!(!keys_initial.contains(&"plan#1/1".to_string()));
        assert!(!keys_initial.contains(&"plan#1/result".to_string()));
        assert!(!keys_initial.contains(&"test#1/result".to_string()));
        assert!(keys_initial.contains(&"gate#1/result".to_string()));
        assert!(keys_initial.contains(&"gate/1".to_string()));

        // Expanding plan#1 with Space reveals non-noise evidence and result.
        st.select_step(0);
        assert_eq!(st.handle_key(&key(KeyCode::Char(' '))), RunOutcome::Changed);
        let keys_expanded = vis_keys(&st);
        assert!(keys_expanded.contains(&"plan#1/1".to_string()));
        assert!(!keys_expanded.contains(&"plan#1/2".to_string()));
        assert!(keys_expanded.contains(&"plan#1/3".to_string()));
        assert!(keys_expanded.contains(&"plan#1/result".to_string()));
        assert!(!keys_expanded.contains(&"test#1/result".to_string()));

        // Toggling noise keeps plan#1 expanded and leaves test#1 collapsed.
        assert_eq!(st.handle_key(&key(KeyCode::Char('n'))), RunOutcome::Changed);
        let keys_after_noise = vis_keys(&st);
        assert!(keys_after_noise.contains(&"plan#1/2".to_string()));
        assert!(keys_after_noise.contains(&"plan#1/result".to_string()));
        assert!(!keys_after_noise.contains(&"test#1/result".to_string()));

        // Right also expands a collapsed attempt.
        st.select_step(1);
        assert_eq!(st.handle_key(&key(KeyCode::Right)), RunOutcome::Changed);
        assert!(vis_keys(&st).contains(&"test#1/result".to_string()));
    }

    #[test]
    fn collapse_on_child_jumps_to_parent() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        st.select_step(0);
        assert_eq!(st.selected, 0);
        // Expand plan#1 explicitly before navigating into its children
        st.handle_key(&key(KeyCode::Right));
        // Move down onto the first child of plan#1
        st.handle_key(&key(KeyCode::Char('j')));
        assert_eq!(st.selected, 1);
        assert_eq!(st.detail_target(), DetailTarget::Step(0));
        // Left on a leaf child jumps back to parent root (index 0)
        st.handle_key(&key(KeyCode::Left));
        assert_eq!(st.selected, 0);
        // Left again collapses the root
        st.handle_key(&key(KeyCode::Char('h')));
        assert!(st.fold.is_collapsed(&"plan#1".to_string()));
    }

    #[test]
    fn shift_e_toggles_all_folds() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        let initial_len = st.visible().len();
        // Finished attempts start collapsed, so the first E expands all.
        st.handle_key(&key(KeyCode::Char('E')));
        let expanded_len = st.visible().len();
        assert!(expanded_len > initial_len);
        // Second E collapses all foldable roots.
        st.handle_key(&key(KeyCode::Char('E')));
        let collapsed_len = st.visible().len();
        assert!(collapsed_len < expanded_len);
        // Only root headers remain visible (3 steps + 1 skipped root = 4)
        assert_eq!(collapsed_len, 4);
        st.handle_key(&key(KeyCode::Char('E')));
        assert_eq!(st.visible().len(), expanded_len);
    }
~~~

## Edit 4: Connect cleanup node to active branch in skipped-node detail target test
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs
OLD:
~~~rust
        // Add an extra unreached node so skipped section also exists
        run.flow.nodes.push(FlowNode {
            id: "cleanup".to_string(),
            kind: NodeKind::Action,
            summary: None,
            is_shell: true,
            ..Default::default()
        });
~~~
NEW:
~~~rust
        // Add an extra unreached node on the active branch so skipped section also exists
        run.flow.nodes.push(FlowNode {
            id: "cleanup".to_string(),
            kind: NodeKind::Action,
            summary: None,
            is_shell: true,
            ..Default::default()
        });
        run.flow.edges.push(FlowEdge::Simple {
            from: "deploy".to_string(),
            to: "cleanup".to_string(),
        });
~~~

## Edit 5: Expand finished step before checking its rendered result line
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs
OLD:
~~~rust
    #[test]
    fn step_tree_result_line_is_plain_text_not_raw_json() {
        let mut run = sample_waiting_run(json!({}));
        run.steps[0].output = json!({"verdict": "approve", "findings": []});
        let mut st = RunViewState::new(run);
        let theme = Theme::default();
        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        st.render(&mut buf, area, &theme, 10_000);
~~~
NEW:
~~~rust
    #[test]
    fn step_tree_result_line_is_plain_text_not_raw_json() {
        let mut run = sample_waiting_run(json!({}));
        run.steps[0].output = json!({"verdict": "approve", "findings": []});
        let mut st = RunViewState::new(run);
        st.select_step(0);
        st.handle_key(&key(KeyCode::Right));
        let theme = Theme::default();
        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        st.render(&mut buf, area, &theme, 10_000);
~~~

## Edit 6: Add tests for repeated attempts, live loop stages, switch pruning, sync preservation, and answered gate display
Path: crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs
OLD:
~~~rust
    #[test]
    fn answer_arriving_while_confirm_is_open_closes_the_confirm() {
        let run = sample_waiting_run(json!({
            "next": {
                "approve": "land.flow.ts",
                "reject": "fix.flow.ts"
            }
        }));
        let mut st = RunViewState::new(run);
        assert_eq!(st.handle_key(&key(KeyCode::Char('1'))), RunOutcome::Changed);
        assert!(st.gate_confirm.is_some());

        st.set_answered(Some("approve".to_string()));
        assert!(st.gate_confirm.is_none());
        assert!(!matches!(
            st.handle_key(&key(KeyCode::Enter)),
            RunOutcome::Gate { .. }
        ));
    }
}
~~~
NEW:
~~~rust
    #[test]
    fn answer_arriving_while_confirm_is_open_closes_the_confirm() {
        let run = sample_waiting_run(json!({
            "next": {
                "approve": "land.flow.ts",
                "reject": "fix.flow.ts"
            }
        }));
        let mut st = RunViewState::new(run);
        assert_eq!(st.handle_key(&key(KeyCode::Char('1'))), RunOutcome::Changed);
        assert!(st.gate_confirm.is_some());

        st.set_answered(Some("approve".to_string()));
        assert!(st.gate_confirm.is_none());
        assert!(!matches!(
            st.handle_key(&key(KeyCode::Enter)),
            RunOutcome::Gate { .. }
        ));
    }

    fn sample_loop_flow() -> FlowDef {
        FlowDef {
            name: "build-repair".to_string(),
            start_at: "build".to_string(),
            nodes: vec![
                FlowNode {
                    id: "build".to_string(),
                    kind: NodeKind::Action,
                    summary: None,
                    is_shell: true,
                    ..Default::default()
                },
                FlowNode {
                    id: "repair".to_string(),
                    kind: NodeKind::Acp,
                    summary: None,
                    is_shell: false,
                    ..Default::default()
                },
            ],
            edges: vec![
                FlowEdge::Simple {
                    from: "build".to_string(),
                    to: "repair".to_string(),
                },
                FlowEdge::Simple {
                    from: "repair".to_string(),
                    to: "build".to_string(),
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn distinct_attempts_of_one_node_stay_in_order_and_start_collapsed() {
        let run = Run {
            summary: RunSummary {
                run_id: "run-loop-done".to_string(),
                dir: PathBuf::from("/tmp/run-loop-done"),
                flow_name: "build-repair".to_string(),
                title: "Build Loop".to_string(),
                status: RunStatus::Completed,
                started_ms: 1_000,
                updated_ms: 6_000,
                finished_ms: Some(6_000),
                current_node: None,
                waiting_on: None,
                status_detail: None,
                error: None,
            },
            flow: sample_loop_flow(),
            steps: vec![
                make_step("build#1", "build", NodeKind::Action, Outcome::Failed, Value::Null),
                make_step("repair#1", "repair", NodeKind::Acp, Outcome::Ok, json!({"route": "retry"})),
                make_step("build#2", "build", NodeKind::Action, Outcome::Ok, Value::Null),
            ],
            events: vec![],
            input: json!({}),
            sessions: vec![],
        };

        let mut st = RunViewState::new(run);
        let vis_keys: Vec<String> = st
            .visible()
            .into_iter()
            .map(|i| st.tree.nodes[i].key.clone())
            .collect();
        assert_eq!(vis_keys, vec!["build#1", "repair#1", "build#2"]);

        st.select_step(0);
        assert_eq!(st.detail_target(), DetailTarget::Step(0));
        st.select_step(1);
        assert_eq!(st.detail_target(), DetailTarget::Step(1));
        st.select_step(2);
        assert_eq!(st.detail_target(), DetailTarget::Step(2));
    }

    #[test]
    fn running_loop_without_recorded_step_shows_live_stage_after_repair() {
        let mut build_1 = make_step("build#1", "build", NodeKind::Action, Outcome::Failed, Value::Null);
        build_1.started_ms = 1_000;
        build_1.finished_ms = 2_000;
        let mut repair_1 = make_step("repair#1", "repair", NodeKind::Acp, Outcome::Ok, json!({}));
        repair_1.started_ms = 2_000;
        repair_1.finished_ms = 3_000;

        let run = Run {
            summary: RunSummary {
                run_id: "run-loop-live".to_string(),
                dir: PathBuf::from("/tmp/run-loop-live"),
                flow_name: "build-repair".to_string(),
                title: "Build Loop Live".to_string(),
                status: RunStatus::Running,
                started_ms: 1_000,
                updated_ms: 4_000,
                finished_ms: None,
                current_node: Some("build".to_string()),
                waiting_on: None,
                status_detail: None,
                error: None,
            },
            flow: sample_loop_flow(),
            steps: vec![build_1, repair_1],
            events: vec![TraceEvent {
                seq: 10,
                at_ms: 3_500,
                scope: "node".to_string(),
                kind: "node_started".to_string(),
                node_id: Some("build".to_string()),
                attempt_id: Some("build#2".to_string()),
                detail: "rebuilding".to_string(),
            }],
            input: json!({}),
            sessions: vec![],
        };

        let st = RunViewState::new(run);
        let vis_keys: Vec<String> = st
            .visible()
            .into_iter()
            .map(|i| st.tree.nodes[i].key.clone())
            .collect();
        assert_eq!(vis_keys, vec!["build#1", "repair#1", "live/build", "live/build/10"]);
        assert_eq!(st.detail_target(), DetailTarget::Node("build".to_string()));
    }

    #[test]
    fn running_loop_with_recorded_live_step_keeps_live_attempt_expanded_after_repair() {
        let mut build_1 = make_step("build#1", "build", NodeKind::Action, Outcome::Failed, Value::Null);
        build_1.started_ms = 1_000;
        build_1.finished_ms = 2_000;
        let mut repair_1 = make_step("repair#1", "repair", NodeKind::Acp, Outcome::Ok, json!({}));
        repair_1.started_ms = 2_000;
        repair_1.finished_ms = 3_000;
        let mut build_2 = make_step("build#2", "build", NodeKind::Action, Outcome::Unknown, Value::Null);
        build_2.started_ms = 3_000;
        build_2.finished_ms = 0;

        let run = Run {
            summary: RunSummary {
                run_id: "run-loop-recorded".to_string(),
                dir: PathBuf::from("/tmp/run-loop-recorded"),
                flow_name: "build-repair".to_string(),
                title: "Build Loop Recorded".to_string(),
                status: RunStatus::Running,
                started_ms: 1_000,
                updated_ms: 4_000,
                finished_ms: None,
                current_node: Some("build".to_string()),
                waiting_on: None,
                status_detail: None,
                error: None,
            },
            flow: sample_loop_flow(),
            steps: vec![build_1, repair_1, build_2],
            events: vec![],
            input: json!({}),
            sessions: vec![],
        };

        let st = RunViewState::new(run);
        let vis_keys: Vec<String> = st
            .visible()
            .into_iter()
            .map(|i| st.tree.nodes[i].key.clone())
            .collect();
        assert_eq!(vis_keys, vec!["build#1", "repair#1", "build#2", "build#2/result"]);
        assert_eq!(st.selected_step(), Some(2));
        assert_eq!(st.detail_target(), DetailTarget::Step(2));
    }

    #[test]
    fn switch_pruning_omits_untaken_branch_from_skipped_section() {
        let flow = FlowDef {
            name: "switch-flow".to_string(),
            start_at: "check".to_string(),
            nodes: vec![
                FlowNode {
                    id: "check".to_string(),
                    kind: NodeKind::Compute,
                    summary: None,
                    is_shell: false,
                    ..Default::default()
                },
                FlowNode {
                    id: "fast_path".to_string(),
                    kind: NodeKind::Action,
                    summary: None,
                    is_shell: true,
                    ..Default::default()
                },
                FlowNode {
                    id: "slow_path".to_string(),
                    kind: NodeKind::Action,
                    summary: None,
                    is_shell: true,
                    ..Default::default()
                },
                FlowNode {
                    id: "finish".to_string(),
                    kind: NodeKind::Action,
                    summary: None,
                    is_shell: true,
                    ..Default::default()
                },
            ],
            edges: vec![
                FlowEdge::Switch {
                    from: "check".to_string(),
                    on: "route".to_string(),
                    cases: vec![
                        ("fast".to_string(), "fast_path".to_string()),
                        ("slow".to_string(), "slow_path".to_string()),
                    ],
                },
                FlowEdge::Simple {
                    from: "fast_path".to_string(),
                    to: "finish".to_string(),
                },
                FlowEdge::Simple {
                    from: "slow_path".to_string(),
                    to: "finish".to_string(),
                },
            ],
            ..Default::default()
        };

        let run = Run {
            summary: RunSummary {
                run_id: "run-switch".to_string(),
                dir: PathBuf::from("/tmp/run-switch"),
                flow_name: "switch-flow".to_string(),
                title: "Switch Flow".to_string(),
                status: RunStatus::Completed,
                started_ms: 1_000,
                updated_ms: 4_000,
                finished_ms: Some(4_000),
                current_node: None,
                waiting_on: None,
                status_detail: None,
                error: None,
            },
            flow,
            steps: vec![
                make_step("check#1", "check", NodeKind::Compute, Outcome::Ok, json!({"route": "fast"})),
                make_step("fast_path#1", "fast_path", NodeKind::Action, Outcome::Ok, Value::Null),
            ],
            events: vec![],
            input: json!({}),
            sessions: vec![],
        };

        let st = RunViewState::new(run);
        let vis_keys: Vec<String> = st
            .visible()
            .into_iter()
            .map(|i| st.tree.nodes[i].key.clone())
            .collect();
        assert!(vis_keys.contains(&"skip/finish".to_string()));
        assert!(!vis_keys.contains(&"skip/slow_path".to_string()));
    }

    #[test]
    fn manual_expansion_persists_across_sync_while_newly_finished_step_collapses() {
        let run = sample_waiting_run(json!({}));
        let mut st = RunViewState::new(run);
        let vis_keys = |st: &RunViewState| -> Vec<String> {
            st.visible()
                .into_iter()
                .map(|i| st.tree.nodes[i].key.clone())
                .collect()
        };

        st.select_step(0);
        assert_eq!(st.handle_key(&key(KeyCode::Char(' '))), RunOutcome::Changed);
        assert!(vis_keys(&st).contains(&"plan#1/result".to_string()));

        st.run_mut().steps.push(make_step(
            "deploy#1",
            "deploy",
            NodeKind::Action,
            Outcome::Ok,
            Value::Null,
        ));
        st.sync();

        let keys_after = vis_keys(&st);
        assert!(keys_after.contains(&"plan#1/result".to_string()));
        assert!(!keys_after.contains(&"test#1/result".to_string()));
        assert!(keys_after.contains(&"deploy#1".to_string()));
        assert!(!keys_after.contains(&"deploy#1/result".to_string()));
    }

    #[test]
    fn answered_gate_keeps_expanded_choices_across_set_answered_and_sync() {
        let run = sample_waiting_run(json!({
            "next": {
                "approve": "land.flow.ts",
                "reject": "fix.flow.ts"
            }
        }));
        let mut st = RunViewState::new(run);

        st.set_answered(Some("approve".to_string()));
        assert!(!st.fold.is_collapsed(&"gate#1".to_string()));
        let keys_after_answer: Vec<String> = st
            .visible()
            .into_iter()
            .map(|i| st.tree.nodes[i].key.clone())
            .collect();
        assert!(keys_after_answer.contains(&"gate/1".to_string()));
        assert!(keys_after_answer.contains(&"gate/2".to_string()));

        st.sync();
        assert!(!st.fold.is_collapsed(&"gate#1".to_string()));
        let keys_after_sync: Vec<String> = st
            .visible()
            .into_iter()
            .map(|i| st.tree.nodes[i].key.clone())
            .collect();
        assert!(keys_after_sync.contains(&"gate/1".to_string()));
        assert!(keys_after_sync.contains(&"gate/2".to_string()));
    }
}
~~~
