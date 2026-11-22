```diff
--- a/crates/codegen/xai-grok-pager/src/views/acpx_modal.rs
+++ b/crates/codegen/xai-grok-pager/src/views/acpx_modal.rs
@@ -23,7 +23,7 @@
 
 use self::detail::{DetailPanel, DetailTarget};
 use self::inspector::{InspectorOutcome, InspectorState};
-use self::ledger::{LedgerOutcome, LedgerState, RowExtra};
+use self::ledger::{GateAnswer, LedgerOutcome, LedgerState, RowExtra};
 use self::run::{RunOutcome, RunViewState};
 
 const MAX_TITLE_WIDTH: usize = 72;
@@ -76,6 +76,8 @@
     preview: Option<Run>,
     /// Rail and agents per run id, with the `updated_ms` they were read at.
     extras: HashMap<String, (i64, RowExtra)>,
+    /// Gate answers for runs outside `EXTRAS_LIMIT`, cached by run id.
+    answers_cache: HashMap<String, Option<GateAnswer>>,
     /// Where the detail panel was last drawn, for the mouse wheel.
     detail_rect: Option<Rect>,
 }
@@ -99,6 +101,7 @@
             detail: DetailPanel::new(),
             preview: None,
             extras: HashMap::new(),
+            answers_cache: HashMap::new(),
             detail_rect: None,
         };
         state.rescan_runs();
@@ -110,6 +113,7 @@
         match bundle::load_run(dir) {
             Ok(run) => {
                 let mut view = RunViewState::new(run);
+                view.set_answered(self.ledger.answered_choice(&view.run().summary.run_id).map(str::to_string));
                 view.set_settings(bundle::node_settings(dir), bundle::step_settings(dir));
                 self.run_view = Some(view);
                 self.inspector = None;
@@ -267,6 +271,10 @@
         self.runs_snapshot = runs.clone();
         self.load_extras(&runs);
         self.ledger.set_runs(runs);
+        if let Some(rv) = self.run_view.as_mut() {
+            let choice = self.ledger.answered_choice(&rv.run().summary.run_id).map(str::to_string);
+            rv.set_answered(choice);
+        }
         changed
     }
 
@@ -280,7 +288,16 @@
             extra.stalled = bundle::is_stalled(summary, now);
             kept.insert(summary.run_id.clone(), (at, extra));
         }
-        self.ledger.set_extras(kept.iter().map(|(id, (_, extra))| (id.clone(), extra.clone())).collect());
+        let mut extras: HashMap<String, RowExtra> = kept.iter().map(|(id, (_, extra))| (id.clone(), extra.clone())).collect();
+        for summary in runs.iter().skip(EXTRAS_LIMIT) {
+            let answers = self
+                .answers_cache
+                .entry(summary.run_id.clone())
+                .or_insert_with(|| read_run_gate_answer(&summary.dir))
+                .clone();
+            extras.insert(summary.run_id.clone(), RowExtra { answers, ..RowExtra::default() });
+        }
+        self.ledger.set_extras(extras);
         self.extras = kept;
     }
 
@@ -525,7 +542,24 @@
          agents,
         stalled: false,
         question: (run.summary.status == bundle::RunStatus::Waiting).then(|| run.summary.status_detail.clone()).flatten(),
+        answers: gate_answer(&run.input),
     }
+}
+
+fn gate_answer(input: &serde_json::Value) -> Option<GateAnswer> {
+    let gate = input.get("gate")?;
+    let run_id = gate.get("runId").and_then(serde_json::Value::as_str).filter(|s| !s.is_empty())?;
+    let choice = gate.get("choice").and_then(serde_json::Value::as_str).unwrap_or_default();
+    Some(GateAnswer {
+        run_id: run_id.to_string(),
+        choice: choice.to_string(),
+    })
+}
+
+fn read_run_gate_answer(dir: &Path) -> Option<GateAnswer> {
+    let text = std::fs::read_to_string(dir.join("projections").join("run.json")).ok()?;
+    let val = serde_json::from_str::<serde_json::Value>(&text).ok()?;
+    gate_answer(val.get("input")?)
 }
 
 fn session_replay(s: SessionSummary) -> ReplayRequest {
@@ -1061,4 +1095,69 @@
         let area = zero.area;
         render_acpx_modal(&mut zero, area, &mut state, false);
     }
+
+    #[test]
+    fn row_extra_reads_gate_answer_from_run_input() {
+        let fx = write_fixture("completed");
+        let run_json_path = fx.run_dir.join("projections").join("run.json");
+        assert!(
+            fs::write(
+                &run_json_path,
+                r#"{"input":{"gate":{"runId":"r1","node":"g","choice":"approve"}}}"#,
+            )
+            .is_ok()
+        );
+        let runs = bundle::list_runs(&fx.runs_base);
+        assert_eq!(runs.len(), 1);
+        if let Some(summary) = runs.first() {
+            let extra = row_extra(summary);
+            assert_eq!(
+                extra.answers,
+                Some(GateAnswer {
+                    run_id: "r1".into(),
+                    choice: "approve".into(),
+                })
+            );
+
+            assert!(fs::write(&run_json_path, r#"{"input":{"other":1}}"#).is_ok());
+            assert_eq!(row_extra(summary).answers, None);
+
+            assert!(fs::write(&run_json_path, r#"{"input":{"gate":"not-an-object"}}"#).is_ok());
+            assert_eq!(row_extra(summary).answers, None);
+        }
+    }
+
+    #[test]
+    fn runs_beyond_extras_limit_receive_gate_answers_with_empty_rail() {
+        let fx = write_fixture("completed");
+        assert!(
+            fs::write(
+                fx.run_dir.join("projections").join("run.json"),
+                r#"{"input":{"gate":{"runId":"waiting-run","node":"gate","choice":"ship"}}}"#,
+            )
+            .is_ok()
+        );
+        for i in 0..EXTRAS_LIMIT {
+            let dir = fx.runs_base.join(format!("newer-{i:03}"));
+            let proj = dir.join("projections");
+            assert!(fs::create_dir_all(&proj).is_ok());
+            let live = format!(
+                r#"{{"runId":"newer-{i:03}","flowName":"f","runTitle":"T","status":"completed","startedAt":"2025-03-02T10:00:{:02}Z","updatedAt":"2025-03-02T10:01:00Z"}}"#,
+                i % 60
+            );
+            assert!(fs::write(proj.join("live.json"), live).is_ok());
+        }
+
+        let state =
+            AcpxModalState::open_at(Some(fx.runs_base.clone()), Some(fx.sessions_base.clone()));
+        assert_eq!(state.runs_snapshot.len(), EXTRAS_LIMIT + 1);
+        assert_eq!(state.ledger.answered_choice("waiting-run"), Some("ship"));
+        assert_eq!(
+            state.answers_cache.get("run-001"),
+            Some(&Some(GateAnswer {
+                run_id: "waiting-run".into(),
+                choice: "ship".into(),
+            }))
+        );
+    }
 }
```