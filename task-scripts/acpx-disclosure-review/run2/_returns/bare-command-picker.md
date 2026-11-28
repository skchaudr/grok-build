=== FILE: task-scripts/acpx-disclosure-returns/bare-command-picker.edits.md ===
```markdown
## Edit 1: Add RunFlowRecord and list_run_flows in driver.rs
Path: crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/driver.rs
OLD:
~~~rust
pub fn list_runs(runs_dir: &Path, limit: usize) -> Vec<(String, String, String, String, i64)> {
    if limit == 0 {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(runs_dir) else {
        return Vec::new();
    };

    let mut runs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(manifest) = read_json_file(&path.join("manifest.json")) else {
            continue;
        };
        let live = read_json_file(&path.join("projections").join("live.json"));

        let run_id = json_str(&manifest, "runId")
            .or_else(|| entry.file_name().to_str().map(ToString::to_string))
            .unwrap_or_default();
        if run_id.is_empty() {
            continue;
        }
        let flow_name = json_str(&manifest, "flowName").unwrap_or_default();
        let title = json_str(&manifest, "runTitle")
            .filter(|s| !s.is_empty())
            .or_else(|| {
                live.as_ref()
                    .and_then(|l| json_str(l, "runTitle"))
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_else(|| flow_name.clone());
        let status = live
            .as_ref()
            .and_then(|l| json_str(l, "status"))
            .filter(|s| !s.is_empty())
            .or_else(|| json_str(&manifest, "status"))
            .unwrap_or_else(|| "unknown".to_string());
        let started_ms = json_str(&manifest, "startedAt")
            .and_then(|s| parse_ts(&s))
            .unwrap_or(0);

        runs.push((run_id, flow_name, title, status, started_ms));
    }

    runs.sort_by(|a, b| b.4.cmp(&a.4).then_with(|| b.0.cmp(&a.0)));
    runs.truncate(limit);
    runs
}
~~~
NEW:
~~~rust
pub fn list_runs(runs_dir: &Path, limit: usize) -> Vec<(String, String, String, String, i64)> {
    if limit == 0 {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(runs_dir) else {
        return Vec::new();
    };

    let mut runs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(manifest) = read_json_file(&path.join("manifest.json")) else {
            continue;
        };
        let live = read_json_file(&path.join("projections").join("live.json"));

        let run_id = json_str(&manifest, "runId")
            .or_else(|| entry.file_name().to_str().map(ToString::to_string))
            .unwrap_or_default();
        if run_id.is_empty() {
            continue;
        }
        let flow_name = json_str(&manifest, "flowName").unwrap_or_default();
        let title = json_str(&manifest, "runTitle")
            .filter(|s| !s.is_empty())
            .or_else(|| {
                live.as_ref()
                    .and_then(|l| json_str(l, "runTitle"))
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_else(|| flow_name.clone());
        let status = live
            .as_ref()
            .and_then(|l| json_str(l, "status"))
            .filter(|s| !s.is_empty())
            .or_else(|| json_str(&manifest, "status"))
            .unwrap_or_else(|| "unknown".to_string());
        let started_ms = json_str(&manifest, "startedAt")
            .and_then(|s| parse_ts(&s))
            .unwrap_or(0);

        runs.push((run_id, flow_name, title, status, started_ms));
    }

    runs.sort_by(|a, b| b.4.cmp(&a.4).then_with(|| b.0.cmp(&a.0)));
    runs.truncate(limit);
    runs
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunFlowRecord {
    pub run_id: String,
    pub flow_name: String,
    pub flow_path: String,
    pub status: String,
    pub started_ms: i64,
}

pub fn list_run_flows(runs_dir: &Path, limit: usize) -> Vec<RunFlowRecord> {
    if limit == 0 {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(runs_dir) else {
        return Vec::new();
    };

    let mut runs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(manifest) = read_json_file(&path.join("manifest.json")) else {
            continue;
        };
        let Some(flow_path) = json_str(&manifest, "flowPath")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let live = read_json_file(&path.join("projections").join("live.json"));

        let run_id = json_str(&manifest, "runId")
            .or_else(|| entry.file_name().to_str().map(ToString::to_string))
            .unwrap_or_default();
        if run_id.is_empty() {
            continue;
        }
        let flow_name = json_str(&manifest, "flowName").unwrap_or_default();
        let status = live
            .as_ref()
            .and_then(|l| json_str(l, "status"))
            .filter(|s| !s.is_empty())
            .or_else(|| json_str(&manifest, "status"))
            .unwrap_or_else(|| "unknown".to_string());
        let started_ms = json_str(&manifest, "startedAt")
            .and_then(|s| parse_ts(&s))
            .unwrap_or(0);

        runs.push(RunFlowRecord {
            run_id,
            flow_name,
            flow_path,
            status,
            started_ms,
        });
    }

    runs.sort_by(|a, b| {
        b.started_ms
            .cmp(&a.started_ms)
            .then_with(|| b.run_id.cmp(&a.run_id))
    });
    runs.truncate(limit);
    runs
}
~~~

## Edit 2: Test list_run_flows in driver.rs
Path: crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/driver.rs
OLD:
~~~rust
    #[test]
    fn test_list_runs_order_and_title_fallback() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let runs_dir = tmp.path();

        let r1 = runs_dir.join("r1");
        write_file(
            &r1,
            "manifest.json",
            &json!({
                "runId": "r1",
                "flowName": "flow-one",
                "startedAt": "2026-11-20T01:00:00.000Z",
                "status": "completed"
            })
            .to_string(),
        );

        let r2 = runs_dir.join("r2");
        write_file(
            &r2,
            "manifest.json",
            &json!({
                "runId": "r2",
                "flowName": "flow-two",
                "runTitle": "Explicit Title",
                "startedAt": "2026-11-20T02:00:00.000Z",
                "status": "running"
            })
            .to_string(),
        );
        write_file(
            &r2,
            "projections/live.json",
            &json!({"status": "failed"}).to_string(),
        );

        let list = list_runs(runs_dir, 10);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].0, "r2");
        assert_eq!(list[0].2, "Explicit Title");
        assert_eq!(list[0].3, "failed");
        assert_eq!(list[1].0, "r1");
        assert_eq!(list[1].2, "flow-one");
        assert_eq!(list[1].3, "completed");

        let limited = list_runs(runs_dir, 1);
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].0, "r2");
    }
~~~
NEW:
~~~rust
    #[test]
    fn test_list_runs_order_and_title_fallback() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let runs_dir = tmp.path();

        let r1 = runs_dir.join("r1");
        write_file(
            &r1,
            "manifest.json",
            &json!({
                "runId": "r1",
                "flowName": "flow-one",
                "startedAt": "2026-11-20T01:00:00.000Z",
                "status": "completed"
            })
            .to_string(),
        );

        let r2 = runs_dir.join("r2");
        write_file(
            &r2,
            "manifest.json",
            &json!({
                "runId": "r2",
                "flowName": "flow-two",
                "runTitle": "Explicit Title",
                "startedAt": "2026-11-20T02:00:00.000Z",
                "status": "running"
            })
            .to_string(),
        );
        write_file(
            &r2,
            "projections/live.json",
            &json!({"status": "failed"}).to_string(),
        );

        let list = list_runs(runs_dir, 10);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].0, "r2");
        assert_eq!(list[0].2, "Explicit Title");
        assert_eq!(list[0].3, "failed");
        assert_eq!(list[1].0, "r1");
        assert_eq!(list[1].2, "flow-one");
        assert_eq!(list[1].3, "completed");

        let limited = list_runs(runs_dir, 1);
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].0, "r2");
    }

    #[test]
    fn test_list_run_flows_skips_missing_flow_path_and_orders_newest_first() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let runs_dir = tmp.path();

        let r_missing = runs_dir.join("r-missing");
        write_file(
            &r_missing,
            "manifest.json",
            &json!({
                "runId": "r-missing",
                "flowName": "no-path",
                "startedAt": "2026-11-20T03:00:00.000Z",
                "status": "completed"
            })
            .to_string(),
        );

        let r_blank = runs_dir.join("r-blank");
        write_file(
            &r_blank,
            "manifest.json",
            &json!({
                "runId": "r-blank",
                "flowName": "blank-path",
                "flowPath": "   ",
                "startedAt": "2026-11-20T02:30:00.000Z",
                "status": "completed"
            })
            .to_string(),
        );

        let r1 = runs_dir.join("r1");
        write_file(
            &r1,
            "manifest.json",
            &json!({
                "runId": "r1",
                "flowName": "flow-one",
                "flowPath": "flows/one.flow.ts",
                "startedAt": "2026-11-20T01:00:00.000Z",
                "status": "completed"
            })
            .to_string(),
        );

        let r2 = runs_dir.join("r2");
        write_file(
            &r2,
            "manifest.json",
            &json!({
                "runId": "r2",
                "flowName": "flow-two",
                "flowPath": "/opt/flows/two.workflow.ts",
                "startedAt": "2026-11-20T02:00:00.000Z",
                "status": "running"
            })
            .to_string(),
        );
        write_file(
            &r2,
            "projections/live.json",
            &json!({"status": "waiting"}).to_string(),
        );

        let list = list_run_flows(runs_dir, 10);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].run_id, "r2");
        assert_eq!(list[0].flow_name, "flow-two");
        assert_eq!(list[0].flow_path, "/opt/flows/two.workflow.ts");
        assert_eq!(list[0].status, "waiting");
        assert_eq!(list[1].run_id, "r1");
        assert_eq!(list[1].flow_name, "flow-one");
        assert_eq!(list[1].flow_path, "flows/one.flow.ts");
        assert_eq!(list[1].status, "completed");

        let limited = list_run_flows(runs_dir, 1);
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].run_id, "r2");
    }
~~~

## Edit 3: Add FlowPicker and OpenPicker variants to Command
Path: crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs
OLD:
~~~rust
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Flow { file: String, input: Value },
    Runs,
    Open { run_id: String },
    Answer { label: String },
    Task { text: String },
    Help,
}
~~~
NEW:
~~~rust
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Flow { file: String, input: Value },
    FlowPicker,
    Runs,
    Open { run_id: String },
    OpenPicker,
    Answer { label: String },
    Task { text: String },
    Help,
}
~~~

## Edit 4: Add picker question, open_saved_run, and run_flow_command helpers on AcpxAgent
Path: crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs
OLD:
~~~rust
    /// Send one `x.ai/ask_user_question` request and return its response
    /// object, wired to the session's cancel plumbing.
    async fn ask_question_raw(
        &self,
        session_id: &str,
        tool_call_id: String,
        params: Value,
    ) -> Option<Value> {
        let raw = serde_json::value::to_raw_value(&params).ok()?;
        let request = acp::ExtRequest::new("x.ai/ask_user_question", raw.into());

        let (cancel_tx, cancel_rx) = oneshot::channel();
        {
            let mut st = self.state.borrow_mut();
            if let Some(sess) = st.sessions.get_mut(session_id) {
                if sess.cancelled {
                    return None;
                }
                sess.pending_question_tool_call_id = Some(tool_call_id);
                sess.pending_question_cancel = Some(cancel_tx);
            }
        }

        let send_fut = acp_send(request, &self.client_tx);
        let response = tokio::select! {
            res = send_fut => res.ok(),
            _ = cancel_rx => None,
        };

        {
            let mut st = self.state.borrow_mut();
            if let Some(sess) = st.sessions.get_mut(session_id) {
                sess.pending_question_tool_call_id = None;
                sess.pending_question_cancel = None;
            }
        }

        let response = response?;
        serde_json::from_str(response.0.get()).ok()
    }
~~~
NEW:
~~~rust
    /// Send one `x.ai/ask_user_question` request and return its response
    /// object, wired to the session's cancel plumbing.
    async fn ask_question_raw(
        &self,
        session_id: &str,
        tool_call_id: String,
        params: Value,
    ) -> Option<Value> {
        let raw = serde_json::value::to_raw_value(&params).ok()?;
        let request = acp::ExtRequest::new("x.ai/ask_user_question", raw.into());

        let (cancel_tx, cancel_rx) = oneshot::channel();
        {
            let mut st = self.state.borrow_mut();
            if let Some(sess) = st.sessions.get_mut(session_id) {
                if sess.cancelled {
                    return None;
                }
                sess.pending_question_tool_call_id = Some(tool_call_id);
                sess.pending_question_cancel = Some(cancel_tx);
            }
        }

        let send_fut = acp_send(request, &self.client_tx);
        let response = tokio::select! {
            res = send_fut => res.ok(),
            _ = cancel_rx => None,
        };

        {
            let mut st = self.state.borrow_mut();
            if let Some(sess) = st.sessions.get_mut(session_id) {
                sess.pending_question_tool_call_id = None;
                sess.pending_question_cancel = None;
            }
        }

        let response = response?;
        serde_json::from_str(response.0.get()).ok()
    }

    async fn ask_picker_question(
        &self,
        session_id: &str,
        kind: &str,
        question_id: &str,
        question: &str,
        options: &[PickerOption],
    ) -> Option<String> {
        let tool_call_id = picker_tool_call_id(kind, session_id);
        let option_values: Vec<Value> = options
            .iter()
            .map(|opt| {
                json!({
                    "label": opt.label,
                    "description": opt.description,
                    "preview": Value::Null,
                    "id": opt.id,
                })
            })
            .collect();

        let params = json!({
            "sessionId": session_id,
            "toolCallId": tool_call_id,
            "questions": [{
                "question": question,
                "options": option_values,
                "multiSelect": false,
                "id": question_id,
            }],
            "mode": "default",
            "noFreeform": true,
        });

        let outer = self.ask_question_raw(session_id, tool_call_id, params).await?;
        let chosen = extract_gate_answer_label(&outer)?;
        let trimmed = chosen.trim();
        options
            .iter()
            .find(|opt| opt.id == trimmed || opt.label == trimmed)
            .map(|opt| opt.id.clone())
    }

    async fn open_saved_run(&self, session_id: &str, run_id: &str, timing: &TurnTiming) {
        let already_shown = self
            .state
            .borrow()
            .sessions
            .get(session_id)
            .is_some_and(|s| s.shown_runs.contains(run_id));
        if already_shown {
            self.send_parent_text(session_id, "Already shown above.", timing)
                .await;
            return;
        }
        let runs_dir = self.runs_dir();
        match resolve_saved_run(&runs_dir, runtime::mirror_root().as_deref(), run_id) {
            Some((run_dir, source)) => {
                if let Err(err) = self
                    .replay_run_into_session(session_id, &run_dir, source, false, Some(timing))
                    .await
                {
                    self.send_parent_text(
                        session_id,
                        format!("Failed to open run `{run_id}`: {err}"),
                        timing,
                    )
                    .await;
                }
            }
            None => {
                self.send_parent_text(
                    session_id,
                    format!("Run `{run_id}` not found."),
                    timing,
                )
                .await;
            }
        }
    }

    async fn run_flow_command(
        &self,
        session_id: &str,
        file: &str,
        input: Value,
        timing: &TurnTiming,
    ) {
        if is_workflow_file(file) {
            // A `.workflow.ts` file starts a run on the pi-workflows
            // server instead of an acpx flow.
            self.run_piw_flow(session_id, file, input, timing).await;
        } else {
            let cwd = self.session_cwd(session_id);
            let flow_path = resolve_flow_path(&cwd, file);
            self.run_flow_chain(session_id, flow_path, input, timing)
                .await;
        }
    }
~~~

## Edit 5: Handle Command::OpenPicker and Command::FlowPicker using shared helpers
Path: crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs
OLD:
~~~rust
            Command::Open { run_id } => {
                let already_shown = self
                    .state
                    .borrow()
                    .sessions
                    .get(&session_id)
                    .is_some_and(|s| s.shown_runs.contains(&run_id));
                if already_shown {
                    self.send_parent_text(&session_id, "Already shown above.", &timing)
                        .await;
                } else {
                    let runs_dir = self.runs_dir();
                    match resolve_saved_run(&runs_dir, runtime::mirror_root().as_deref(), &run_id) {
                        Some((run_dir, source)) => {
                            if let Err(err) = self
                                .replay_run_into_session(&session_id, &run_dir, source, false, Some(&timing))
                                .await
                            {
                                self.send_parent_text(
                                    &session_id,
                                    format!("Failed to open run `{run_id}`: {err}"),
                                    &timing,
                                )
                                .await;
                            }
                        }
                        None => {
                            self.send_parent_text(
                                &session_id,
                                format!("Run `{run_id}` not found."),
                                &timing,
                            )
                            .await;
                        }
                    }
                }
            }
            Command::Answer { label } => {
                let (waiting_gate, waiting_piw) = {
                    let st = self.state.borrow();
                    let sess = st.sessions.get(&session_id);
                    (
                        sess.and_then(|s| s.waiting_gate.clone()),
                        sess.and_then(|s| s.waiting_piw.clone()),
                    )
                };
                if let Some(waiting) = waiting_piw {
                    // A parked pi-workflows run: the answer goes back through
                    // the server and the same run resumes.
                    if self
                        .answer_piw_gate(&session_id, waiting.gate.clone(), &label, &timing)
                        .await
                    {
                        {
                            let mut st = self.state.borrow_mut();
                            if let Some(sess) = st.sessions.get_mut(&session_id) {
                                sess.waiting_piw = None;
                            }
                        }
                        self.pump_piw_events(&session_id, &timing).await;
                        ran_flow = true;
                    }
                } else {
                    match waiting_gate {
                        None => {
                            self.send_parent_text(&session_id, "No gate is waiting.", &timing)
                                .await;
                        }
                        Some(gate) => {
                        let matched = gate
                            .choices
                            .iter()
                            .any(|c| c.label.trim().eq_ignore_ascii_case(label.trim()));
                        if !matched {
                            let list = gate
                                .choices
                                .iter()
                                .map(|c| c.label.as_str())
                                .collect::<Vec<_>>()
                                .join(", ");
                            self.send_parent_text(
                                &session_id,
                                format!("Choices: {list}"),
                                &timing,
                            )
                            .await;
                        } else {
                            {
                                let mut st = self.state.borrow_mut();
                                if let Some(sess) = st.sessions.get_mut(&session_id) {
                                    sess.waiting_gate = None;
                                }
                            }
                            if let Some((next_flow, next_input)) = self
                                .prepare_chained_run(&session_id, &gate, &label, &timing)
                                .await
                            {
                                self.run_flow_chain(&session_id, next_flow, next_input, &timing)
                                    .await;
                                ran_flow = true;
                            } else {
                                self.remember_waiting_gate(&session_id, gate);
                            }
                        }
                        }
                    }
                }
            }
            Command::Flow { file, input } => {
                if is_workflow_file(&file) {
                    // A `.workflow.ts` file starts a run on the pi-workflows
                    // server instead of an acpx flow.
                    self.run_piw_flow(&session_id, &file, input, &timing).await;
                } else {
                    let cwd = self.session_cwd(&session_id);
                    let flow_path = resolve_flow_path(&cwd, &file);
                    self.run_flow_chain(&session_id, flow_path, input, &timing)
                        .await;
                }
                ran_flow = true;
            }
~~~
NEW:
~~~rust
            Command::Open { run_id } => {
                self.open_saved_run(&session_id, &run_id, &timing).await;
            }
            Command::OpenPicker => {
                let runs_dir = self.runs_dir();
                let runs = list_saved_runs(&runs_dir, runtime::mirror_root().as_deref(), 20);
                let options = open_picker_options(&runs);
                if options.is_empty() {
                    self.send_parent_text(&session_id, "No acpx runs found.", &timing)
                        .await;
                } else if let Some(run_id) = self
                    .ask_picker_question(
                        &session_id,
                        "open",
                        "acpx-open-picker",
                        "Choose a run to open:",
                        &options,
                    )
                    .await
                {
                    self.open_saved_run(&session_id, &run_id, &timing).await;
                } else {
                    self.send_parent_text(&session_id, "No run opened.", &timing)
                        .await;
                }
            }
            Command::Answer { label } => {
                let (waiting_gate, waiting_piw) = {
                    let st = self.state.borrow();
                    let sess = st.sessions.get(&session_id);
                    (
                        sess.and_then(|s| s.waiting_gate.clone()),
                        sess.and_then(|s| s.waiting_piw.clone()),
                    )
                };
                if let Some(waiting) = waiting_piw {
                    // A parked pi-workflows run: the answer goes back through
                    // the server and the same run resumes.
                    if self
                        .answer_piw_gate(&session_id, waiting.gate.clone(), &label, &timing)
                        .await
                    {
                        {
                            let mut st = self.state.borrow_mut();
                            if let Some(sess) = st.sessions.get_mut(&session_id) {
                                sess.waiting_piw = None;
                            }
                        }
                        self.pump_piw_events(&session_id, &timing).await;
                        ran_flow = true;
                    }
                } else {
                    match waiting_gate {
                        None => {
                            self.send_parent_text(&session_id, "No gate is waiting.", &timing)
                                .await;
                        }
                        Some(gate) => {
                        let matched = gate
                            .choices
                            .iter()
                            .any(|c| c.label.trim().eq_ignore_ascii_case(label.trim()));
                        if !matched {
                            let list = gate
                                .choices
                                .iter()
                                .map(|c| c.label.as_str())
                                .collect::<Vec<_>>()
                                .join(", ");
                            self.send_parent_text(
                                &session_id,
                                format!("Choices: {list}"),
                                &timing,
                            )
                            .await;
                        } else {
                            {
                                let mut st = self.state.borrow_mut();
                                if let Some(sess) = st.sessions.get_mut(&session_id) {
                                    sess.waiting_gate = None;
                                }
                            }
                            if let Some((next_flow, next_input)) = self
                                .prepare_chained_run(&session_id, &gate, &label, &timing)
                                .await
                            {
                                self.run_flow_chain(&session_id, next_flow, next_input, &timing)
                                    .await;
                                ran_flow = true;
                            } else {
                                self.remember_waiting_gate(&session_id, gate);
                            }
                        }
                        }
                    }
                }
            }
            Command::Flow { file, input } => {
                self.run_flow_command(&session_id, &file, input, &timing)
                    .await;
                ran_flow = true;
            }
            Command::FlowPicker => {
                let default_flow = self.default_flow();
                let cwd = self.session_cwd(&session_id);
                let runs_dir = self.runs_dir();
                let recent_flows =
                    list_saved_run_flows(&runs_dir, runtime::mirror_root().as_deref(), 20);
                let options = flow_picker_options(
                    default_flow.as_deref(),
                    &recent_flows,
                    &cwd,
                    |path| path.is_file(),
                );
                if options.is_empty() {
                    self.send_parent_text(
                        &session_id,
                        help_markdown(default_flow.as_deref()),
                        &timing,
                    )
                    .await;
                } else if let Some(selected) = self
                    .ask_picker_question(
                        &session_id,
                        "flow",
                        "acpx-flow-picker",
                        "Choose a flow to run:",
                        &options,
                    )
                    .await
                    && let Some(Command::Flow { file, input }) =
                        flow_command_from_selection(&options, &selected)
                {
                    self.run_flow_command(&session_id, &file, input, &timing)
                        .await;
                    ran_flow = true;
                } else {
                    self.send_parent_text(&session_id, "No flow started.", &timing)
                        .await;
                }
            }
~~~

## Edit 6: Return FlowPicker and OpenPicker for bare /flow and /open in parse_command
Path: crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs
OLD:
~~~rust
pub fn parse_command(text: &str, waiting_labels: &[String]) -> Command {
    let trimmed = text.trim();
    if let Some(after_slash) = trimmed.strip_prefix('/') {
        let (cmd_word, rest) = split_first_word(after_slash);
        let cmd_lower = cmd_word.to_ascii_lowercase();
        match cmd_lower.as_str() {
            "flow" => {
                let Some((file, after_file)) = parse_flow_file_and_rest(rest) else {
                    return Command::Help;
                };
                if file.is_empty() {
                    return Command::Help;
                }
                let input = parse_flow_input(after_file);
                Command::Flow { file, input }
            }
            "runs" => {
                if rest.trim().is_empty() {
                    Command::Runs
                } else {
                    Command::Help
                }
            }
            "open" => {
                let run_id = unquote_arg(rest.trim());
                if run_id.is_empty() {
                    Command::Help
                } else {
                    Command::Open { run_id }
                }
            }
            "answer" => {
                let label = unquote_arg(rest.trim());
                if label.is_empty() {
                    Command::Help
                } else {
                    let resolved = waiting_labels
                        .iter()
                        .find(|w| w.trim().eq_ignore_ascii_case(&label))
                        .cloned()
                        .unwrap_or(label);
                    Command::Answer { label: resolved }
                }
            }
            "acpx-help" => Command::Help,
            _ => Command::Help,
        }
    } else {
        if !trimmed.is_empty()
            && let Some(matched) = waiting_labels
                .iter()
                .find(|w| w.trim().eq_ignore_ascii_case(trimmed))
        {
            return Command::Answer {
                label: matched.clone(),
            };
        }
        Command::Task {
            text: trimmed.to_string(),
        }
    }
}
~~~
NEW:
~~~rust
pub fn parse_command(text: &str, waiting_labels: &[String]) -> Command {
    let trimmed = text.trim();
    if let Some(after_slash) = trimmed.strip_prefix('/') {
        let (cmd_word, rest) = split_first_word(after_slash);
        let cmd_lower = cmd_word.to_ascii_lowercase();
        match cmd_lower.as_str() {
            "flow" => {
                if rest.trim().is_empty() {
                    return Command::FlowPicker;
                }
                let Some((file, after_file)) = parse_flow_file_and_rest(rest) else {
                    return Command::FlowPicker;
                };
                if file.is_empty() {
                    return Command::Help;
                }
                let input = parse_flow_input(after_file);
                Command::Flow { file, input }
            }
            "runs" => {
                if rest.trim().is_empty() {
                    Command::Runs
                } else {
                    Command::Help
                }
            }
            "open" => {
                if rest.trim().is_empty() {
                    Command::OpenPicker
                } else {
                    let run_id = unquote_arg(rest.trim());
                    if run_id.is_empty() {
                        Command::Help
                    } else {
                        Command::Open { run_id }
                    }
                }
            }
            "answer" => {
                let label = unquote_arg(rest.trim());
                if label.is_empty() {
                    Command::Help
                } else {
                    let resolved = waiting_labels
                        .iter()
                        .find(|w| w.trim().eq_ignore_ascii_case(&label))
                        .cloned()
                        .unwrap_or(label);
                    Command::Answer { label: resolved }
                }
            }
            "acpx-help" => Command::Help,
            _ => Command::Help,
        }
    } else {
        if !trimmed.is_empty()
            && let Some(matched) = waiting_labels
                .iter()
                .find(|w| w.trim().eq_ignore_ascii_case(trimmed))
        {
            return Command::Answer {
                label: matched.clone(),
            };
        }
        Command::Task {
            text: trimmed.to_string(),
        }
    }
}
~~~

## Edit 7: Mark /flow and /open arguments optional in available_commands
Path: crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs
OLD:
~~~rust
fn available_commands() -> Vec<acp::AvailableCommand> {
    vec![
        acp::AvailableCommand::new("flow", "Run an acpx flow file").input(
            acp::AvailableCommandInput::Unstructured(acp::UnstructuredCommandInput::new(
                "<flow file> [task or JSON input]",
            )),
        ),
        acp::AvailableCommand::new("runs", "List recent acpx flow runs"),
        acp::AvailableCommand::new("open", "Replay an existing acpx run into this session").input(
            acp::AvailableCommandInput::Unstructured(acp::UnstructuredCommandInput::new(
                "<run id>",
            )),
        ),
        acp::AvailableCommand::new("answer", "Answer a waiting checkpoint gate").input(
            acp::AvailableCommandInput::Unstructured(acp::UnstructuredCommandInput::new(
                "<choice>",
            )),
        ),
        acp::AvailableCommand::new("acpx-help", "Show acpx flow commands and usage"),
    ]
}
~~~
NEW:
~~~rust
fn available_commands() -> Vec<acp::AvailableCommand> {
    vec![
        acp::AvailableCommand::new("flow", "Run an acpx flow file (or pick one when omitted)")
            .input(acp::AvailableCommandInput::Unstructured(
                acp::UnstructuredCommandInput::new("[flow file] [task or JSON input]"),
            )),
        acp::AvailableCommand::new("runs", "List recent acpx flow runs"),
        acp::AvailableCommand::new(
            "open",
            "Replay an existing acpx run into this session (or pick one when omitted)",
        )
        .input(acp::AvailableCommandInput::Unstructured(
            acp::UnstructuredCommandInput::new("[run id]"),
        )),
        acp::AvailableCommand::new("answer", "Answer a waiting checkpoint gate").input(
            acp::AvailableCommandInput::Unstructured(acp::UnstructuredCommandInput::new(
                "<choice>",
            )),
        ),
        acp::AvailableCommand::new("acpx-help", "Show acpx flow commands and usage"),
    ]
}
~~~

## Edit 8: Add pure picker option builders, saved-flow listing, and picker toolCallId helper
Path: crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs
OLD:
~~~rust
fn gate_tool_call_id(run_id: &str, attempt: &str) -> String {
    format!("acpx-gate:{run_id}:{attempt}")
}
~~~
NEW:
~~~rust
fn gate_tool_call_id(run_id: &str, attempt: &str) -> String {
    format!("acpx-gate:{run_id}:{attempt}")
}

fn picker_tool_call_id(kind: &str, session_id: &str) -> String {
    format!("acpx-picker:{kind}:{session_id}:{}", utc_now_ms())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PickerOption {
    id: String,
    label: String,
    description: String,
}

fn list_saved_run_flows(
    base: &Path,
    mirror: Option<&Path>,
    limit: usize,
) -> Vec<(driver::RunFlowRecord, SavedRunSource)> {
    let mut runs: Vec<_> = driver::list_run_flows(base, limit)
        .into_iter()
        .map(|run| (run, SavedRunSource::Native))
        .collect();
    if let Some(mirror) = mirror {
        runs.extend(
            driver::list_run_flows(mirror, limit)
                .into_iter()
                .map(|run| (run, SavedRunSource::PiwMirror)),
        );
    }
    runs.sort_by(|(a, _), (b, _)| {
        b.started_ms
            .cmp(&a.started_ms)
            .then_with(|| b.run_id.cmp(&a.run_id))
    });
    runs.truncate(limit);
    runs
}

fn open_picker_options(runs: &[(ListedRun, SavedRunSource)]) -> Vec<PickerOption> {
    let mut sorted: Vec<&(ListedRun, SavedRunSource)> = runs.iter().collect();
    sorted.sort_by(|(a, _), (b, _)| b.4.cmp(&a.4).then_with(|| b.0.cmp(&a.0)));
    sorted
        .into_iter()
        .map(|((run_id, flow_name, title, status, started_ms), source)| {
            let mut parts = Vec::new();
            if !flow_name.is_empty() {
                parts.push(flow_name.as_str());
            }
            if !title.is_empty() && title != flow_name {
                parts.push(title.as_str());
            }
            parts.push(source.label());
            if !status.is_empty() {
                parts.push(status.as_str());
            }
            let started = format_iso8601_ms(*started_ms);
            parts.push(started.as_str());
            PickerOption {
                id: run_id.clone(),
                label: run_id.clone(),
                description: parts.join(" · "),
            }
        })
        .collect()
}

fn format_flow_run_description(run: &driver::RunFlowRecord, source: SavedRunSource) -> String {
    let mut parts = Vec::new();
    if !run.flow_name.is_empty() {
        parts.push(run.flow_name.as_str());
    }
    parts.push(source.label());
    if !run.status.is_empty() {
        parts.push(run.status.as_str());
    }
    let started = format_iso8601_ms(run.started_ms);
    parts.push(started.as_str());
    parts.join(" · ")
}

fn flow_picker_options(
    default_flow: Option<&Path>,
    recent_runs: &[(driver::RunFlowRecord, SavedRunSource)],
    session_cwd: &Path,
    file_exists: impl Fn(&Path) -> bool,
) -> Vec<PickerOption> {
    let mut sorted_runs: Vec<&(driver::RunFlowRecord, SavedRunSource)> =
        recent_runs.iter().collect();
    sorted_runs.sort_by(|(a, _), (b, _)| {
        b.started_ms
            .cmp(&a.started_ms)
            .then_with(|| b.run_id.cmp(&a.run_id))
    });

    let mut options = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    if let Some(default_path) = default_flow {
        let default_str = default_path.to_string_lossy();
        let trimmed = default_str.trim();
        if !trimmed.is_empty() {
            let resolved = resolve_flow_path(session_cwd, trimmed);
            if file_exists(&resolved) && seen.insert(resolved.clone()) {
                let newest_match = sorted_runs.iter().find(|(run, _)| {
                    let run_path = run.flow_path.trim();
                    !run_path.is_empty() && resolve_flow_path(session_cwd, run_path) == resolved
                });
                let description = match newest_match {
                    Some((run, source)) => format!(
                        "default flow · {}",
                        format_flow_run_description(run, *source)
                    ),
                    None => "default flow".to_string(),
                };
                let path_str = resolved.to_string_lossy().to_string();
                options.push(PickerOption {
                    id: path_str.clone(),
                    label: path_str,
                    description,
                });
            }
        }
    }

    for (run, source) in sorted_runs {
        let trimmed = run.flow_path.trim();
        if trimmed.is_empty() {
            continue;
        }
        let resolved = resolve_flow_path(session_cwd, trimmed);
        if !file_exists(&resolved) || !seen.insert(resolved.clone()) {
            continue;
        }
        let path_str = resolved.to_string_lossy().to_string();
        options.push(PickerOption {
            id: path_str.clone(),
            label: path_str,
            description: format_flow_run_description(run, *source),
        });
    }

    options
}

fn flow_command_from_selection(options: &[PickerOption], selected: &str) -> Option<Command> {
    let trimmed = selected.trim();
    let opt = options
        .iter()
        .find(|o| o.id == trimmed || o.label == trimmed)?;
    Some(Command::Flow {
        file: opt.id.clone(),
        input: json!({}),
    })
}
~~~

## Edit 9: Mark /flow and /open arguments optional in help_markdown
Path: crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs
OLD:
~~~rust
fn help_markdown(default_flow: Option<&Path>) -> String {
    let default_desc = match default_flow {
        Some(path) => format!(
            "A plain message runs the default flow (`{}`) with `{{\"task\": \"<message>\"}}`, or answers a waiting gate if it matches one of the choice labels.",
            path.display()
        ),
        None => "A plain message answers a waiting gate if it matches one of the choice labels, or prints this help when no default flow is configured.".to_string(),
    };
    format!(
        "### acpx commands\n\
        - `/flow <file> [task or JSON]` — start an acpx flow run\n\
        - `/runs` — list recent flow runs\n\
        - `/open <run id>` — replay an existing run into this session\n\
        - `/answer <choice>` — answer a waiting checkpoint gate\n\
        - `/acpx-help` — show this command list\n\n\
        {default_desc}\n"
    )
}
~~~
NEW:
~~~rust
fn help_markdown(default_flow: Option<&Path>) -> String {
    let default_desc = match default_flow {
        Some(path) => format!(
            "A plain message runs the default flow (`{}`) with `{{\"task\": \"<message>\"}}`, or answers a waiting gate if it matches one of the choice labels.",
            path.display()
        ),
        None => "A plain message answers a waiting gate if it matches one of the choice labels, or prints this help when no default flow is configured.".to_string(),
    };
    format!(
        "### acpx commands\n\
        - `/flow [file] [task or JSON]` — start an acpx flow run (or pick a recent flow when omitted)\n\
        - `/runs` — list recent flow runs\n\
        - `/open [run id]` — replay an existing run into this session (or pick a recent run when omitted)\n\
        - `/answer <choice>` — answer a waiting checkpoint gate\n\
        - `/acpx-help` — show this command list\n\n\
        {default_desc}\n"
    )
}
~~~

## Edit 10: Update parse_command test and add unit tests for open and flow pickers
Path: crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs
OLD:
~~~rust
    #[test]
    fn parse_command_unknown_slash_and_acpx_help() {
        assert_eq!(parse_command("/acpx-help", &[]), Command::Help);
        assert_eq!(parse_command("/unknown-cmd arg", &[]), Command::Help);
        assert_eq!(parse_command("/flow", &[]), Command::Help);
        assert_eq!(parse_command("/open", &[]), Command::Help);
    }
~~~
NEW:
~~~rust
    #[test]
    fn parse_command_unknown_slash_and_acpx_help() {
        assert_eq!(parse_command("/acpx-help", &[]), Command::Help);
        assert_eq!(parse_command("/unknown-cmd arg", &[]), Command::Help);
        assert_eq!(parse_command("/runs extra", &[]), Command::Help);
        assert_eq!(parse_command("/answer", &[]), Command::Help);
        assert_eq!(parse_command("/flow", &[]), Command::FlowPicker);
        assert_eq!(parse_command("/open", &[]), Command::OpenPicker);
    }

    #[test]
    fn open_picker_options_orders_newest_first_and_formats_descriptions() {
        let runs = vec![
            (
                (
                    "run-older".to_string(),
                    "build".to_string(),
                    "build".to_string(),
                    "completed".to_string(),
                    1_700_000_000_000,
                ),
                SavedRunSource::Native,
            ),
            (
                (
                    "run-newer".to_string(),
                    "release".to_string(),
                    "Ship v2".to_string(),
                    "waiting".to_string(),
                    1_700_000_060_000,
                ),
                SavedRunSource::PiwMirror,
            ),
        ];

        let options = open_picker_options(&runs);
        assert_eq!(options.len(), 2);

        assert_eq!(options[0].id, "run-newer");
        assert_eq!(options[0].label, "run-newer");
        assert!(options[0].description.contains("release"));
        assert!(options[0].description.contains("Ship v2"));
        assert!(options[0].description.contains("PIW mirror"));
        assert!(options[0].description.contains("waiting"));
        assert!(
            options[0]
                .description
                .contains(&format_iso8601_ms(1_700_000_060_000))
        );

        assert_eq!(options[1].id, "run-older");
        assert_eq!(options[1].label, "run-older");
        assert_eq!(options[1].description.matches("build").count(), 1);
        assert!(options[1].description.contains("ACPX"));
        assert!(options[1].description.contains("completed"));
        assert!(
            options[1]
                .description
                .contains(&format_iso8601_ms(1_700_000_000_000))
        );

        assert!(open_picker_options(&[]).is_empty());
    }

    #[test]
    fn flow_picker_options_resolves_filters_dedups_and_maps_selection() {
        let cwd = Path::new("/workspace/project");
        let default_flow = Path::new("flows/default.flow.ts");
        let existing: HashSet<PathBuf> = [
            PathBuf::from("/workspace/project/flows/default.flow.ts"),
            PathBuf::from("/workspace/project/flows/ship.flow.ts"),
            PathBuf::from("/opt/workflows/deploy.workflow.ts"),
        ]
        .into_iter()
        .collect();

        let recent_runs = vec![
            (
                driver::RunFlowRecord {
                    run_id: "r-old-ship".to_string(),
                    flow_name: "ship-old".to_string(),
                    flow_path: "/workspace/project/flows/ship.flow.ts".to_string(),
                    status: "failed".to_string(),
                    started_ms: 1_700_000_010_000,
                },
                SavedRunSource::Native,
            ),
            (
                driver::RunFlowRecord {
                    run_id: "r-gone".to_string(),
                    flow_name: "gone".to_string(),
                    flow_path: "/private/tmp/gone.flow.ts".to_string(),
                    status: "completed".to_string(),
                    started_ms: 1_700_000_090_000,
                },
                SavedRunSource::Native,
            ),
            (
                driver::RunFlowRecord {
                    run_id: "r-default".to_string(),
                    flow_name: "default".to_string(),
                    flow_path: "/workspace/project/flows/default.flow.ts".to_string(),
                    status: "completed".to_string(),
                    started_ms: 1_700_000_050_000,
                },
                SavedRunSource::Native,
            ),
            (
                driver::RunFlowRecord {
                    run_id: "r-new-ship".to_string(),
                    flow_name: "ship-new".to_string(),
                    flow_path: "flows/ship.flow.ts".to_string(),
                    status: "completed".to_string(),
                    started_ms: 1_700_000_080_000,
                },
                SavedRunSource::Native,
            ),
            (
                driver::RunFlowRecord {
                    run_id: "r-piw".to_string(),
                    flow_name: "deploy".to_string(),
                    flow_path: "/opt/workflows/deploy.workflow.ts".to_string(),
                    status: "waiting".to_string(),
                    started_ms: 1_700_000_070_000,
                },
                SavedRunSource::PiwMirror,
            ),
        ];

        let options = flow_picker_options(
            Some(default_flow),
            &recent_runs,
            cwd,
            |path| existing.contains(path),
        );

        assert_eq!(options.len(), 3);
        assert_eq!(options[0].id, "/workspace/project/flows/default.flow.ts");
        assert!(options[0].description.contains("default flow"));
        assert!(options[0].description.contains("ACPX"));

        assert_eq!(options[1].id, "/workspace/project/flows/ship.flow.ts");
        assert!(options[1].description.contains("ship-new"));
        assert!(options[1].description.contains("completed"));
        assert!(!options[1].description.contains("ship-old"));

        assert_eq!(options[2].id, "/opt/workflows/deploy.workflow.ts");
        assert!(options[2].description.contains("deploy"));
        assert!(options[2].description.contains("PIW mirror"));
        assert!(options[2].description.contains("waiting"));

        let selected_cmd = flow_command_from_selection(&options, &options[2].id);
        assert_eq!(
            selected_cmd,
            Some(parse_command("/flow /opt/workflows/deploy.workflow.ts", &[]))
        );
        assert_eq!(flow_command_from_selection(&options, "unknown"), None);

        let empty = flow_picker_options(None, &recent_runs, cwd, |_| false);
        assert!(empty.is_empty());

        let picker_id = picker_tool_call_id("flow", "sess-1");
        assert!(!picker_id.starts_with("acpx-gate:"));
    }
~~~
```