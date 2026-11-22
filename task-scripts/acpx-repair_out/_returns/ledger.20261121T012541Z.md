```diff
--- a/crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
+++ b/crates/codegen/xai-grok-pager/src/views/acpx_modal/ledger.rs
@@ -30,12 +30,20 @@
     Close,
 }
 
+/// The gate this run answered: read from this run's own input, written by `bundle::gate_input` when a gate answer
+/// starts the next flow file. `projections/run.json` -> `input.gate = { "runId": <the waiting run>, "node": .., "choice": .. }`.
+#[derive(Debug, Clone, Default, PartialEq, Eq)]
+pub struct GateAnswer {
+    pub run_id: String,   // input.gate.runId: the run that was waiting at the gate
+    pub choice: String,   // input.gate.choice: the label that was picked
+}
+
 /// Per-run facts the parent loads (cached) so the ledger does no I/O.
 #[derive(Debug, Clone, Default, PartialEq)]
 pub struct RowExtra {
     pub rail: Vec<NodeState>,
     pub agents: String,
     pub stalled: bool,
     pub question: Option<String>,
+    pub answers: Option<GateAnswer>,
 }
 
 #[derive(Debug, Clone, Copy, PartialEq, Eq)]
@@ -112,6 +120,7 @@
     Header { kind: SectionKind, count: usize },
     Blank,
     Run(usize), // index in `runs`
+    Continuation { indent: usize, text: String },
 }
 
 impl FlowListRow {
@@ -124,6 +133,7 @@
     tab: LedgerTab,
     runs: Vec<RunSummary>,
     extras: HashMap<String, RowExtra>,
+    answered: HashMap<String, String>,
     sessions: Vec<SessionSummary>,
 
     // Per-tab filter, order, active-only, selection & scroll
@@ -156,6 +166,7 @@
             tab: LedgerTab::Flows,
             runs: Vec::new(),
             extras: HashMap::new(),
+            answered: HashMap::new(),
             sessions: Vec::new(),
 
             flow_filter: String::new(),
@@ -178,12 +189,45 @@
 
     pub fn set_runs(&mut self, runs: Vec<RunSummary>) {
         self.runs = runs;
+        self.rebuild_answered();
         self.reconcile_flow_selection();
     }
 
     pub fn set_extras(&mut self, extras: HashMap<String, RowExtra>) {
         self.extras = extras;
+        self.rebuild_answered();
         self.reconcile_flow_selection();
+    }
+
+    pub fn answered_choice(&self, run_id: &str) -> Option<&str> {
+        if self
+            .runs
+            .iter()
+            .find(|r| r.run_id == run_id)
+            .is_some_and(|r| r.status != RunStatus::Waiting)
+        {
+            return None;
+        }
+        self.answered.get(run_id).map(String::as_str)
+    }
+
+    fn rebuild_answered(&mut self) {
+        let mut winners: HashMap<String, (&str, Option<i64>, &str)> = HashMap::new();
+        let mut keys: Vec<&String> = self.extras.keys().collect();
+        keys.sort();
+        for answering_id in keys {
+            if let Some(extra) = self.extras.get(answering_id)
+                && let Some(ref ans) = extra.answers
+            {
+                let started = self
+                    .runs
+                    .iter()
+                    .find(|r| &r.run_id == answering_id)
+                    .map(|r| r.started_ms);
+                let replace = match winners.get(&ans.run_id) {
+                    None => true,
+                    Some(&(prev_id, prev_started, _)) => match (prev_started, started) {
+                        (Some(ps), Some(ns)) => (ns, answering_id.as_str()) > (ps, prev_id),
+                        _ => answering_id.as_str() > prev_id,
+                    },
+                };
+                if replace {
+                    winners.insert(
+                        ans.run_id.clone(),
+                        (answering_id.as_str(), started, ans.choice.as_str()),
+                    );
+                }
+            }
+        }
+        self.answered = winners
+            .into_iter()
+            .map(|(k, (_, _, choice))| (k, choice.to_string()))
+            .collect();
     }
 
     pub fn selected_run(&self) -> Option<&RunSummary> {
@@ -240,6 +284,10 @@
             .unwrap_or(false)
     }
 
+    fn is_answered(&self, run: &RunSummary) -> bool {
+        run.status == RunStatus::Waiting && self.answered.contains_key(&run.run_id)
+    }
+
     fn active_filter(&self) -> &str {
         match self.tab {
             LedgerTab::Flows => &self.flow_filter,
@@ -255,7 +303,9 @@
     }
 
     fn is_run_active(&self, run: &RunSummary) -> bool {
-        matches!(run.status, RunStatus::Running | RunStatus::Waiting) && !self.is_stalled(run)
+        matches!(run.status, RunStatus::Running | RunStatus::Waiting)
+            && !self.is_stalled(run)
+            && !self.is_answered(run)
     }
 
     fn matches_flow_filter(&self, run: &RunSummary) -> bool {
@@ -295,7 +345,7 @@
 
     fn run_section(&self, run: &RunSummary) -> SectionKind {
         let stalled = self.is_stalled(run);
-        if run.status == RunStatus::Waiting {
+        if run.status == RunStatus::Waiting && !self.is_answered(run) {
             SectionKind::NeedsYou
         } else if run.status == RunStatus::Running && !stalled {
             SectionKind::Running
@@ -359,13 +409,22 @@
 
     /// Build visual rows for the Flows tab. `list_height` is used to decide whether blank rows
     /// separate sections when the list is shorter than the area.
-    fn build_flow_rows(&self, list_height: usize) -> Vec<FlowListRow> {
+    fn build_flow_rows(&self, list_height: usize, now_ms: i64) -> Vec<FlowListRow> {
         let indices = self.filtered_flow_indices();
         if indices.is_empty() {
             return Vec::new();
         }
         if self.flow_order != FlowOrder::Grouped {
-            return indices.into_iter().map(FlowListRow::Run).collect();
+            let mut out = Vec::new();
+            for idx in indices {
+                out.push(FlowListRow::Run(idx));
+                self.append_continuations(&mut out, idx, now_ms);
+            }
+            return out;
         }
 
         let mut needs_you = Vec::new();
@@ -390,7 +449,17 @@
         .filter(|(_, items)| !items.is_empty())
         .collect();
 
-        let non_blank_rows: usize = sections.iter().map(|(_, items)| 1 + items.len()).sum();
+        let extra_rows: usize = sections
+            .iter()
+            .flat_map(|(_, items)| items.iter())
+            .map(|&idx| self.continuation_lines_for(idx, now_ms).len())
+            .sum();
+        let non_blank_rows: usize = sections
+            .iter()
+            .map(|(_, items)| 1 + items.len())
+            .sum::<usize>()
+            + extra_rows;
         let blank_count = sections.len().saturating_sub(1);
         let insert_blanks = non_blank_rows + blank_count <= list_height;
 
         let mut out = Vec::with_capacity(non_blank_rows + blank_count);
@@ -404,9 +473,21 @@
             });
             for idx in items {
                 out.push(FlowListRow::Run(idx));
+                self.append_continuations(&mut out, idx, now_ms);
             }
         }
         out
+    }
+
+    fn append_continuations(&self, out: &mut Vec<FlowListRow>, idx: usize, now_ms: i64) {
+        for (indent, text) in self.continuation_lines_for(idx, now_ms) {
+            out.push(FlowListRow::Continuation { indent, text });
+        }
+    }
+
+    fn continuation_lines_for(&self, idx: usize, now_ms: i64) -> Vec<(usize, String)> {
+        let Some(run) = self.runs.get(idx) else {
+            return Vec::new();
+        };
+        let is_selected = self.flow_selected_run_id.as_deref() == Some(run.run_id.as_str());
+        if !is_selected || run.status != RunStatus::Waiting || self.is_answered(run) {
+            return Vec::new();
+        }
+        let (_, indent, lines) = self.flow_question_lines(run, self.last_render_width, now_ms);
+        lines.into_iter().skip(1).map(|t| (indent, t)).collect()
     }
 
     fn filtered_session_indices(&self) -> Vec<usize> {
@@ -743,7 +824,7 @@
             height: area.height.saturating_sub(1),
         };
 
-        let overflow_counter = self.compute_overflow_counter(list_area.height as usize);
+        let overflow_counter = self.compute_overflow_counter(list_area.height as usize, now_ms);
         self.render_tab_row(
             buf,
             Rect {
@@ -766,13 +847,13 @@
         }
     }
 
-    fn compute_overflow_counter(&self, list_height: usize) -> Option<String> {
+    fn compute_overflow_counter(&self, list_height: usize, now_ms: i64) -> Option<String> {
         if list_height == 0 {
             return None;
         }
         match self.tab {
             LedgerTab::Flows => {
-                let rows = self.build_flow_rows(list_height);
+                let rows = self.build_flow_rows(list_height, now_ms);
                 if rows.len() <= list_height {
                     return None;
                 }
@@ -895,7 +976,7 @@
         let waiting = self
             .runs
             .iter()
-            .filter(|r| r.status == RunStatus::Waiting)
+            .filter(|r| r.status == RunStatus::Waiting && !self.is_answered(r))
             .count();
         let running = self
             .runs
@@ -946,7 +1027,7 @@
 
     fn render_flows(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, now_ms: i64) {
         let list_h = area.height as usize;
-        let rows = self.build_flow_rows(list_h);
+        let rows = self.build_flow_rows(list_h, now_ms);
 
         if rows.is_empty() {
             let msg = if !self.flow_filter.is_empty() || self.flow_active_only {
@@ -968,17 +1049,21 @@
 
         // Adjust scroll to keep selection visible (and include header row immediately above if at top)
         if let Some(sel_v) = selected_visual_idx {
+            let mut sel_end = sel_v + 1;
+            while matches!(rows.get(sel_end), Some(FlowListRow::Continuation { .. })) {
+                sel_end += 1;
+            }
+            let sel_h = sel_end - sel_v;
             if sel_v < self.flow_scroll {
                 self.flow_scroll = sel_v;
                 // If the row right above `sel_v` is a section header, show it too when possible
                 if sel_v > 0
                     && matches!(rows.get(sel_v - 1), Some(FlowListRow::Header { .. }))
-                    && list_h > 1
+                    && list_h > sel_h
                 {
                     self.flow_scroll = sel_v - 1;
                 }
-            } else if sel_v >= self.flow_scroll + list_h {
-                self.flow_scroll = (sel_v + 1).saturating_sub(list_h);
+            } else if sel_end > self.flow_scroll + list_h {
+                self.flow_scroll = sel_end.saturating_sub(list_h).min(sel_v);
             }
         }
         let max_scroll = rows.len().saturating_sub(list_h);
@@ -1011,6 +1096,19 @@
                         self.render_flow_row(buf, row_rect, run, is_selected, theme, now_ms);
                     }
                 }
+                FlowListRow::Continuation { indent, text } => {
+                    buf.set_style(row_rect, Style::default().bg(theme.bg_highlight));
+                    let content = format!("{}{}", " ".repeat(*indent), text);
+                    let line = Line::from(Span::styled(
+                        truncate_str(&content, row_rect.width as usize),
+                        Style::default()
+                            .fg(theme.warning)
+                            .add_modifier(Modifier::BOLD),
+                    ));
+                    buf.set_line(row_rect.x, row_rect.y, &line, row_rect.width);
+                }
             }
         }
     }
@@ -1066,10 +1164,13 @@
 
         let extra = self.extras.get(&run.run_id);
         let stalled = extra.map(|e| e.stalled).unwrap_or(false);
+        let answered = self.is_answered(run);
 
         // Status glyph
         let (glyph, glyph_color) = if stalled {
             ("⊘", theme.gray)
+        } else if answered {
+            (RunStatus::Completed.glyph(), theme.accent_success)
         } else {
             let c = match run.status {
                 RunStatus::Running => theme.accent_running,
@@ -1097,6 +1198,11 @@
             && !ex.rail.is_empty()
         {
             for &ns in &ex.rail {
+                let ns = if answered && ns == NodeState::Waiting {
+                    NodeState::Done(Outcome::Ok)
+                } else {
+                    ns
+                };
                 let g = ns.glyph();
                 let color = match ns {
                     NodeState::Running => theme.accent_running,
@@ -1191,15 +1297,22 @@
 
         // Title and state text share what is left. Every row uses the same title width, so the columns line up.
         let longest_title = self.runs.iter().map(|r| r.title.width()).max().unwrap_or(0);
+        let wrap_selected_waiting =
+            selected && run.status == RunStatus::Waiting && !answered && !raw_state_text.is_empty();
         let (title_str, state_str) = if flex_budget <= 17 || raw_state_text.is_empty() && rail_width == 0 {
-            (truncate_str(&run.title, flex_budget), String::new())
+            let first = if wrap_selected_waiting {
+                let (first, _, _) = self.flow_question_lines(run, w, now_ms);
+                first
+            } else {
+                String::new()
+            };
+            (truncate_str(&run.title, flex_budget), first)
         } else {
             let half = (flex_budget - 1).div_euclid(2);
             let title_col = longest_title.clamp(16, half.max(16));
             let title = truncate_str(&run.title, title_col);
             let pad = title_col.saturating_sub(title.width());
-            let state = truncate_str(&raw_state_text, flex_budget - 1 - title_col);
+            let state = if wrap_selected_waiting {
+                let (first, _, _) = self.flow_question_lines(run, w, now_ms);
+                first
+            } else {
+                truncate_str(&raw_state_text, flex_budget - 1 - title_col)
+            };
             (format!("{title}{}", " ".repeat(pad)), state)
         };
 
@@ -1260,6 +1373,77 @@
         buf.set_line(row.x, row.y, &line, row.width);
     }
 
+    fn flow_question_lines(
+        &self,
+        run: &RunSummary,
+        w: usize,
+        now_ms: i64,
+    ) -> (String, usize, Vec<String>) {
+        let extra = self.extras.get(&run.run_id);
+        let stalled = extra.map(|e| e.stalled).unwrap_or(false);
+        let (raw_state_text, _) = self.flow_state_text(run, extra, stalled, &Theme::default());
+        if raw_state_text.is_empty() || w == 0 {
+            return (String::new(), 0, Vec::new());
+        }
+
+        let show_rail = w >= 48;
+        let show_age = w >= 64;
+        let show_rail_count = w >= 80;
+        let show_duration = w >= 96;
+        let show_agents = w >= 120;
+
+        let rail_width = if show_rail {
+            let count_w = |n: usize| {
+                if show_rail_count {
+                    format!(" {n}/{n}").width()
+                } else {
+                    0
+                }
+            };
+            self.extras
+                .values()
+                .map(|e| {
+                    if e.rail.is_empty() {
+                        0
+                    } else {
+                        e.rail.len() + count_w(e.rail.len())
+                    }
+                })
+                .max()
+                .unwrap_or(0)
+        } else {
+            0
+        };
+
+        let agents_w = if show_agents {
+            extra
+                .map(|e| e.agents.as_str())
+                .filter(|s| !s.is_empty())
+                .map(|s| 1 + truncate_str(s, 26).width())
+                .unwrap_or(0)
+        } else {
+            0
+        };
+        let age_w = if show_age {
+            1 + fmt_age(now_ms.saturating_sub(run.updated_ms)).width()
+        } else {
+            0
+        };
+        let dur_w = if show_duration {
+            let end_ms = if run.status.is_live() && !stalled {
+                now_ms
+            } else {
+                run.finished_ms.unwrap_or(run.updated_ms)
+            };
+            let dur_ms = (end_ms - run.started_ms).max(0) as u64;
+            1 + fmt_duration(dur_ms).width()
+        } else {
+            0
+        };
+
+        let prefix_w = run.status.glyph().width() + 1;
+        let rail_col_w = if rail_width > 0 { 1 + rail_width } else { 0 };
+        let fixed_w = prefix_w + rail_col_w + agents_w + age_w + dur_w;
+        let flex_budget = w.saturating_sub(fixed_w);
+        let longest_title = self.runs.iter().map(|r| r.title.width()).max().unwrap_or(0);
+
+        if flex_budget <= 17 {
+            let indent = prefix_w;
+            let wrap_w = w.saturating_sub(indent).max(1);
+            let mut lines = vec![String::new()];
+            lines.extend(wrap_words(&raw_state_text, wrap_w));
+            (String::new(), indent, lines)
+        } else {
+            let half = (flex_budget - 1).div_euclid(2);
+            let title_col = longest_title.clamp(16, half.max(16));
+            let indent = prefix_w + title_col + rail_col_w + 1;
+            let state_w = (flex_budget - 1 - title_col).max(1);
+            let lines = wrap_words(&raw_state_text, state_w);
+            let first = lines.first().cloned().unwrap_or_default();
+            (first, indent, lines)
+        }
+    }
+
     fn flow_state_text(
         &self,
         run: &RunSummary,
@@ -1268,6 +1452,14 @@
         theme: &Theme,
     ) -> (String, Color) {
         if run.status == RunStatus::Waiting {
+            if let Some(choice) = self.answered.get(&run.run_id) {
+                let text = if choice.is_empty() {
+                    "answered".to_string()
+                } else {
+                    format!("answered: {choice}")
+                };
+                return (text, theme.gray);
+            }
             if let Some(q) = extra.and_then(|e| e.question.as_deref()).filter(|q| !q.is_empty()) {
                 return (q.to_string(), theme.warning);
             }
@@ -1561,6 +1753,40 @@
     }
 }
 
+fn wrap_words(text: &str, max_w: usize) -> Vec<String> {
+    let max_w = max_w.max(1);
+    let mut lines: Vec<String> = Vec::new();
+    let mut cur = String::new();
+    let mut cur_w = 0;
+    for word in text.split_whitespace() {
+        let ww = word.width();
+        if cur.is_empty() {
+            if ww <= max_w {
+                cur.push_str(word);
+                cur_w = ww;
+            } else {
+                for ch in word.chars() {
+                    let cw = ch.to_string().width();
+                    if cur_w + cw > max_w && !cur.is_empty() {
+                        lines.push(std::mem::take(&mut cur));
+                        cur_w = 0;
+                    }
+                    cur.push(ch);
+                    cur_w += cw;
+                }
+            }
+        } else if cur_w + 1 + ww <= max_w {
+            cur.push(' ');
+            cur.push_str(word);
+            cur_w += 1 + ww;
+        } else {
+            lines.push(std::mem::take(&mut cur));
+            cur.clear();
+            cur_w = 0;
+            if ww <= max_w {
+                cur.push_str(word);
+                cur_w = ww;
+            } else {
+                for ch in word.chars() {
+                    let cw = ch.to_string().width();
+                    if cur_w + cw > max_w && !cur.is_empty() {
+                        lines.push(std::mem::take(&mut cur));
+                        cur_w = 0;
+                    }
+                    cur.push(ch);
+                    cur_w += cw;
+                }
+            }
+        }
+    }
+    if !cur.is_empty() {
+        lines.push(cur);
+    }
+    lines
+}
+
 fn pad_right(s: &str, width: usize) -> String {
     let w = s.width();
     if w >= width {
@@ -1747,6 +1973,7 @@
                 agents: String::new(),
                 stalled: true,
                 question: None,
+                answers: None,
             },
         );
         state.set_extras(extras);
@@ -1968,6 +2195,7 @@
                 agents: "grok-build · grok-4".to_string(),
                 stalled: false,
                 question: Some("Ship to prod?".to_string()),
+                answers: None,
             },
         );
         state.set_extras(extras);
@@ -2007,6 +2235,7 @@
                 agents: "grok-agent".to_string(),
                 stalled: false,
                 question: Some("Approve?".to_string()),
+                answers: None,
             },
         );
         state.set_extras(extras);
@@ -2037,4 +2266,227 @@
         let full = all_text(&empty_buf);
         assert!(full.contains("No acpx flow runs found"));
     }
+
+    #[test]
+    fn answered_gate_counts_only_unanswered_and_moves_answered_to_finished() {
+        let mut state = LedgerState::new();
+        state.set_runs(vec![
+            make_run("r-old-wait", "Old Gate", RunStatus::Waiting, 1_000),
+            make_run("r-new-wait", "New Gate", RunStatus::Waiting, 2_000),
+            make_run("r-next", "Next Flow", RunStatus::Completed, 3_000),
+        ]);
+        let mut extras = HashMap::new();
+        extras.insert(
+            "r-old-wait".to_string(),
+            RowExtra {
+                rail: vec![NodeState::Waiting],
+                agents: String::new(),
+                stalled: false,
+                question: Some("Continue old?".to_string()),
+                answers: None,
+            },
+        );
+        extras.insert(
+            "r-new-wait".to_string(),
+            RowExtra {
+                rail: vec![NodeState::Waiting],
+                agents: String::new(),
+                stalled: false,
+                question: Some("Continue new?".to_string()),
+                answers: None,
+            },
+        );
+        extras.insert(
+            "r-next".to_string(),
+            RowExtra {
+                rail: vec![NodeState::Done(Outcome::Ok)],
+                agents: String::new(),
+                stalled: false,
+                question: None,
+                answers: Some(GateAnswer {
+                    run_id: "r-old-wait".to_string(),
+                    choice: "Approve".to_string(),
+                }),
+            },
+        );
+        state.set_extras(extras);
+
+        let theme = Theme::default();
+        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 14));
+        let area = buf.area;
+        state.render(&mut buf, area, &theme, 10_000);
+        let full = all_text(&buf);
+        assert!(full.contains("1 needs you"));
+        assert!(full.contains("NEEDS YOU 1"));
+        assert!(full.contains("FINISHED 2"));
+        let fin_pos = full.find("FINISHED 2").unwrap_or(0);
+        let old_pos = full.find("Old Gate").unwrap_or(0);
+        assert!(old_pos > fin_pos);
+    }
+
+    #[test]
+    fn unanswered_waiting_run_remains_in_needs_you_and_reverts_when_answer_removed() {
+        let mut state = LedgerState::new();
+        state.set_runs(vec![
+            make_run("r-wait", "Gate Run", RunStatus::Waiting, 1_000),
+            make_run("r-ans", "Answering Run", RunStatus::Failed, 2_000),
+        ]);
+        let mut extras_unanswered = HashMap::new();
+        extras_unanswered.insert(
+            "r-wait".to_string(),
+            RowExtra {
+                rail: vec![NodeState::Waiting],
+                agents: String::new(),
+                stalled: false,
+                question: Some("Proceed?".to_string()),
+                answers: None,
+            },
+        );
+        state.set_extras(extras_unanswered.clone());
+
+        let theme = Theme::default();
+        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 12));
+        let area = buf.area;
+        state.render(&mut buf, area, &theme, 10_000);
+        let full = all_text(&buf);
+        assert!(full.contains("1 needs you"));
+        assert!(full.contains("NEEDS YOU 1"));
+
+        let mut extras_answered = extras_unanswered.clone();
+        extras_answered.insert(
+            "r-ans".to_string(),
+            RowExtra {
+                rail: vec![],
+                agents: String::new(),
+                stalled: false,
+                question: None,
+                answers: Some(GateAnswer {
+                    run_id: "r-wait".to_string(),
+                    choice: "Yes".to_string(),
+                }),
+            },
+        );
+        state.set_extras(extras_answered);
+        let mut buf2 = Buffer::empty(Rect::new(0, 0, 100, 12));
+        state.render(&mut buf2, area, &theme, 10_000);
+        assert!(!all_text(&buf2).contains("needs you"));
+
+        // Replace extras without the answering run: counts as needing you again
+        state.set_extras(extras_unanswered);
+        let mut buf3 = Buffer::empty(Rect::new(0, 0, 100, 12));
+        state.render(&mut buf3, area, &theme, 10_000);
+        let full3 = all_text(&buf3);
+        assert!(full3.contains("1 needs you"));
+        assert!(full3.contains("NEEDS YOU 1"));
+    }
+
+    #[test]
+    fn set_extras_before_set_runs_matches_reverse_and_latest_started_wins() {
+        let runs = vec![
+            make_run("r-wait", "Waiting Gate", RunStatus::Waiting, 1_000),
+            make_run("r-ans-1", "Answer 1", RunStatus::Completed, 2_000),
+            make_run("r-ans-2", "Answer 2", RunStatus::Completed, 5_000),
+        ];
+        let mut extras = HashMap::new();
+        extras.insert(
+            "r-wait".to_string(),
+            RowExtra {
+                rail: vec![NodeState::Waiting],
+                agents: String::new(),
+                stalled: false,
+                question: Some("Which?".to_string()),
+                answers: None,
+            },
+        );
+        extras.insert(
+            "r-ans-1".to_string(),
+            RowExtra {
+                rail: vec![],
+                agents: String::new(),
+                stalled: false,
+                question: None,
+                answers: Some(GateAnswer {
+                    run_id: "r-wait".to_string(),
+                    choice: "OldChoice".to_string(),
+                }),
+            },
+        );
+        extras.insert(
+            "r-ans-2".to_string(),
+            RowExtra {
+                rail: vec![],
+                agents: String::new(),
+                stalled: false,
+                question: None,
+                answers: Some(GateAnswer {
+                    run_id: "r-wait".to_string(),
+                    choice: "NewChoice".to_string(),
+                }),
+            },
+        );
+
+        let mut s1 = LedgerState::new();
+        s1.set_extras(extras.clone());
+        s1.set_runs(runs.clone());
+
+        let mut s2 = LedgerState::new();
+        s2.set_runs(runs);
+        s2.set_extras(extras);
+
+        assert_eq!(s1.answered_choice("r-wait"), Some("NewChoice"));
+        assert_eq!(s2.answered_choice("r-wait"), Some("NewChoice"));
+        assert_eq!(s1.answered_choice("r-ans-1"), None);
+        assert_eq!(s1.answered_choice("nonexistent"), None);
+
+        let theme = Theme::default();
+        let mut b1 = Buffer::empty(Rect::new(0, 0, 100, 12));
+        let mut b2 = Buffer::empty(Rect::new(0, 0, 100, 12));
+        let area = b1.area;
+        s1.render(&mut b1, area, &theme, 10_000);
+        s2.render(&mut b2, area, &theme, 10_000);
+        let t1 = all_text(&b1);
+        assert_eq!(t1, all_text(&b2));
+        assert!(t1.contains("answered: NewChoice"));
+        assert!(!t1.contains("Which?"));
+    }
+
+    #[test]
+    fn selected_unanswered_waiting_run_wraps_200_char_question_and_unselected_stays_one_row() {
+        let words: Vec<String> = (1..=28).map(|i| format!("word{i:02}")).collect();
+        let question = format!("{} tailend", words.join(" "));
+        assert_eq!(question.len(), 203);
+
+        let mut state = LedgerState::new();
+        state.set_runs(vec![
+            make_run("r-wait-1", "First Gate", RunStatus::Waiting, 2_000),
+            make_run("r-wait-2", "Second Gate", RunStatus::Waiting, 1_000),
+        ]);
+        let mut extras = HashMap::new();
+        extras.insert(
+            "r-wait-1".to_string(),
+            RowExtra {
+                rail: vec![NodeState::Waiting],
+                agents: String::new(),
+                stalled: false,
+                question: Some(question.clone()),
+                answers: None,
+            },
+        );
+        extras.insert(
+            "r-wait-2".to_string(),
+            RowExtra {
+                rail: vec![NodeState::Waiting],
+                agents: String::new(),
+                stalled: false,
+                question: Some("Short?".to_string()),
+                answers: None,
+            },
+        );
+        state.set_extras(extras);
+
+        let theme = Theme::default();
+        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 20));
+        let area = buf.area;
+        state.render(&mut buf, area, &theme, 10_000);
+        let text = all_text(&buf);
+        for w in question.split_whitespace() {
+            assert!(text.contains(w), "missing word {w} in:\n{text}");
+        }
+
+        // Move selection to r-wait-2 so r-wait-1 is unselected and takes a single row (row 2)
+        state.handle_key(&key(KeyCode::Down));
+        let mut buf2 = Buffer::empty(Rect::new(0, 0, 100, 20));
+        state.render(&mut buf2, area, &theme, 10_000);
+        assert!(row_text(&buf2, 2).contains("First Gate"));
+        assert!(row_text(&buf2, 3).contains("Second Gate"));
+    }
 }
```