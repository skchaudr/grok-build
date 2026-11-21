```diff
--- a/crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs
+++ b/crates/codegen/xai-grok-pager/src/views/acpx_modal/run.rs
@@ -10,7 +10,7 @@
 use ratatui::style::{Color, Modifier, Style};
 use ratatui::text::{Line, Span};
 use serde_json::Value;
-use unicode_width::UnicodeWidthStr;
+use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
 
 use crate::acpx::bundle::{
     self, AgentSettings, NodeKind, NodeState, Outcome, Run, RunStatus, Step, TraceEvent,
@@ -108,6 +108,7 @@
     detail_rect: Option<Rect>,
     gate_confirm: Option<GateConfirm>,
     graph_cache: Option<GraphCache>,
+    answered: Option<String>,
 }
 
 impl RunViewState {
@@ -127,6 +128,7 @@
             detail_rect: None,
             gate_confirm: None,
             graph_cache: None,
+            answered: None,
         };
         st.rebuild_tree();
         st.select_initial_stage();
@@ -142,6 +144,14 @@
         self.step_settings = steps;
     }
 
+    pub fn set_answered(&mut self, choice: Option<String>) {
+        self.answered = choice;
+    }
+
+    pub fn answered(&self) -> Option<&str> {
+        self.answered.as_deref()
+    }
+
     pub fn detail_target(&self) -> DetailTarget {
         let vis = self.visible();
         let Some(&node_idx) = vis.get(self.selected) else {
@@ -247,7 +257,9 @@
 
     pub fn shortcuts(&self) -> Vec<&'static str> {
         let mut out = vec!["Enter step", "r replay"];
-        if self.run.summary.status == RunStatus::Waiting && !bundle::gate_choices(&self.run).is_empty()
+        if self.run.summary.status == RunStatus::Waiting
+            && self.answered.is_none()
+            && !bundle::gate_choices(&self.run).is_empty()
         {
             out.push("1-9 gate");
         }
@@ -439,7 +451,7 @@
                 }
             }
             KeyCode::Char(ch @ '1'..='9') => {
-                if self.run.summary.status != RunStatus::Waiting {
+                if self.run.summary.status != RunStatus::Waiting || self.answered.is_some() {
                     return RunOutcome::Unchanged;
                 }
                 let idx = (ch as usize) - ('1' as usize);
@@ -463,6 +475,9 @@
                     label, flow_file, ..
                 }) = self.payloads.get(node_idx)
                 {
+                    if self.run.summary.status == RunStatus::Waiting && self.answered.is_some() {
+                        return RunOutcome::Unchanged;
+                    }
                     self.gate_confirm = Some(GateConfirm {
                         choice: label.clone(),
                         flow_file: flow_file.clone(),
@@ -509,11 +524,18 @@
             y = y.saturating_add(1);
         }
 
-        // 2. Banner row (when needed)
-        if y < max_y
-            && let Some(banner_line) = self.build_banner(area.width as usize, theme, now_ms)
-        {
-            buf.set_line(area.x, y, &banner_line, area.width);
+        // 2. Banner rows (when needed)
+        if y < max_y {
+            let banner_lines = self.build_banner(area.width as usize, theme, now_ms);
+            if !banner_lines.is_empty() {
+                let avail = max_y.saturating_sub(y) as usize;
+                let max_banner = avail.saturating_sub(2).max(usize::from(avail >= 2));
+                for bline in banner_lines.into_iter().take(max_banner) {
+                    if y >= max_y {
+                        break;
+                    }
+                    buf.set_line(area.x, y, &bline, area.width);
+                    y = y.saturating_add(1);
+                }
+            }
         }
 
         // 3. Flow graph (when toggled on and area height >= 16)
@@ -881,7 +903,12 @@
 
     fn render_header(&self, buf: &mut Buffer, area: Rect, theme: &Theme, now_ms: i64) {
         let status = self.run.summary.status;
-        let status_color = status_color(status, theme);
+        let header_status = if status == RunStatus::Waiting && self.answered.is_some() {
+            RunStatus::Completed
+        } else {
+            status
+        };
+        let status_color = status_color(header_status, theme);
         let bold = Style::default()
             .fg(theme.text_primary)
             .add_modifier(Modifier::BOLD);
@@ -890,7 +917,7 @@
 
         let mut spans = vec![
             Span::styled(
-                format!("{} ", status.glyph()),
+                format!("{} ", header_status.glyph()),
                 Style::default()
                     .fg(status_color)
                     .add_modifier(Modifier::BOLD),
@@ -929,14 +956,14 @@
         buf.set_line(area.x, area.y, &Line::from(spans), area.width);
     }
 
-    fn build_banner(&self, width: usize, theme: &Theme, now_ms: i64) -> Option<Line<'static>> {
+    fn build_banner(&self, width: usize, theme: &Theme, now_ms: i64) -> Vec<Line<'static>> {
         let sum = &self.run.summary;
-        let (text, color) = if bundle::is_stalled(sum, now_ms) {
+        let (text, style) = if bundle::is_stalled(sum, now_ms) {
             let node = sum.current_node.as_deref().unwrap_or("?");
             (
                 format!("⊘ stalled at {node}: the acpx process is gone"),
-                theme.gray,
+                Style::default().fg(theme.gray).add_modifier(Modifier::BOLD),
             )
         } else {
             match sum.status {
                 RunStatus::Waiting => {
@@ -945,8 +972,22 @@
                         .as_deref()
                         .or(sum.current_node.as_deref())
                         .unwrap_or("?");
-                    let detail = sum.status_detail.as_deref().unwrap_or("waiting");
-                    (format!("◐ NEEDS YOU at {node}: {detail}"), theme.warning)
+                    if let Some(choice) = self.answered.as_deref() {
+                        let msg = if choice.is_empty() {
+                            format!("✓ answered at {node}")
+                        } else {
+                            format!("✓ answered at {node}: {choice}")
+                        };
+                        (msg, Style::default().fg(theme.gray))
+                    } else {
+                        let detail = sum.status_detail.as_deref().unwrap_or("waiting");
+                        (
+                            format!("◐ NEEDS YOU at {node}: {detail}"),
+                            Style::default()
+                                .fg(theme.warning)
+                                .add_modifier(Modifier::BOLD),
+                        )
+                    }
                 }
                 RunStatus::Failed | RunStatus::TimedOut => {
                     let err = sum
@@ -954,7 +995,12 @@
                         .as_deref()
                         .or(sum.status_detail.as_deref())
                         .unwrap_or(sum.status.label());
-                    (format!("✗ {err}"), theme.accent_error)
+                    (
+                        format!("✗ {err}"),
+                        Style::default()
+                            .fg(theme.accent_error)
+                            .add_modifier(Modifier::BOLD),
+                    )
                 }
                 RunStatus::Running => {
                     let node = sum.current_node.as_deref().unwrap_or("?");
@@ -962,17 +1008,21 @@
                         Some(d) if !d.is_empty() => format!("● {node} · {d}"),
                         _ => format!("● {node}"),
                     };
-                    (msg, theme.accent_running)
+                    (
+                        msg,
+                        Style::default()
+                            .fg(theme.accent_running)
+                            .add_modifier(Modifier::BOLD),
+                    )
                 }
-                RunStatus::Completed | RunStatus::Unknown => return None,
+                RunStatus::Completed | RunStatus::Unknown => return Vec::new(),
             }
         };
 
-        let cut = truncate_str(&text, width);
-        Some(Line::from(Span::styled(
-            cut,
-            Style::default().fg(color).add_modifier(Modifier::BOLD),
-        )))
+        wrap_banner(&text, width)
+            .into_iter()
+            .map(|line| Line::from(Span::styled(line, style)))
+            .collect()
     }
 
     fn prepare_graph_lines(
@@ -1225,7 +1275,15 @@
             } => {
                 let guide = if is_last_child { "  ╰ " } else { "  │ " };
                 let avail = width.saturating_sub(4);
-                let body = format!("[{choice_idx}] {label} → {flow_file}");
+                let is_chosen = self
+                    .answered
+                    .as_deref()
+                    .is_some_and(|c| c.eq_ignore_ascii_case(label));
+                let body = if is_chosen {
+                    format!("[{choice_idx}] ✓ {label} → {flow_file}")
+                } else {
+                    format!("[{choice_idx}] {label} → {flow_file}")
+                };
                 let cut = truncate_str(&body, avail);
                 Line::from(vec![
                     Span::styled(guide, style_with(theme.gray_dim, false)),
@@ -1521,6 +1579,51 @@
     s.lines().next().unwrap_or("").trim().to_string()
 }
 
+fn wrap_banner(text: &str, width: usize) -> Vec<String> {
+    if width == 0 || text.is_empty() {
+        return Vec::new();
+    }
+    let mut lines = Vec::new();
+    let mut current = String::new();
+    let mut cur_w = 0usize;
+
+    for word in text.split_whitespace() {
+        let w = UnicodeWidthStr::width(word);
+        if current.is_empty() {
+            if w <= width {
+                current.push_str(word);
+                cur_w = w;
+            } else {
+                for ch in word.chars() {
+                    let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
+                    if cur_w + cw > width && !current.is_empty() {
+                        lines.push(std::mem::take(&mut current));
+                        cur_w = 0;
+                    }
+                    current.push(ch);
+                    cur_w += cw;
+                }
+            }
+        } else if cur_w + 1 + w <= width {
+            current.push(' ');
+            current.push_str(word);
+            cur_w += 1 + w;
+        } else if w <= width {
+            lines.push(std::mem::take(&mut current));
+            current.push_str(word);
+            cur_w = w;
+        } else {
+            lines.push(std::mem::take(&mut current));
+            cur_w = 0;
+            for ch in word.chars() {
+                let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
+                if cur_w + cw > width && !current.is_empty() {
+                    lines.push(std::mem::take(&mut current));
+                    cur_w = 0;
+                }
+                current.push(ch);
+                cur_w += cw;
+            }
+        }
+    }
+    if !current.is_empty() {
+        lines.push(current);
+    }
+    lines
+}
+
 fn render_mermaid_lines(src: &str, width: usize) -> Vec<Line<'static>> {
     if width == 0 {
         return Vec::new();
@@ -1968,4 +2071,79 @@
         st.render(&mut buf_small, small, &theme, 10_000);
         st.render(&mut buf_zero, Rect::new(0, 0, 0, 0), &theme, 10_000);
     }
+
+    #[test]
+    fn waiting_banner_wraps_120_char_question_at_width_80() {
+        let question = "Should we proceed with rolling out the canary deployment to production cluster alpha after verifying all smoke suites pass?";
+        assert_eq!(question.len(), 123);
+        let mut run = sample_waiting_run(json!({"next": {"approve": "land.flow.ts"}}));
+        run.summary.status_detail = Some(question.to_string());
+        let mut st = RunViewState::new(run);
+        let theme = Theme::default();
+        let area = Rect::new(0, 0, 80, 30);
+        let mut buf = Buffer::empty(area);
+        st.render(&mut buf, area, &theme, 10_000);
+
+        let text = buf_text(&buf);
+        for word in question.split_whitespace() {
+            assert!(text.contains(word), "missing word {word:?} in:\n{text}");
+        }
+    }
+
+    #[test]
+    fn short_waiting_question_takes_one_row_and_preserves_rows_below() {
+        let mut run = sample_waiting_run(json!({"next": {"approve": "land.flow.ts"}}));
+        run.summary.status_detail = Some("ship it?".to_string());
+        let mut st = RunViewState::new(run);
+        st.handle_key(&key(KeyCode::Char('v')));
+        let theme = Theme::default();
+        let area = Rect::new(0, 0, 120, 30);
+        let mut buf = Buffer::empty(area);
+        st.render(&mut buf, area, &theme, 10_000);
+
+        let text = buf_text(&buf);
+        let lines: Vec<&str> = text.lines().collect();
+        assert!(lines[1].contains("◐ NEEDS YOU at gate: ship it?"));
+        assert!(lines[2].starts_with('─'));
+        assert_eq!(st.detail_area().map(|r| r.y), Some(3));
+    }
+
+    #[test]
+    fn set_answered_replaces_needs_you_banner_marks_choice_and_disables_gate_keys() {
+        let mut run = sample_waiting_run(json!({
+            "next": {
+                "approve": "land.flow.ts",
+                "reject": "fix.flow.ts"
+            }
+        }));
+        run.summary.status_detail = Some("awaiting human approval".to_string());
+        let mut st = RunViewState::new(run);
+
+        assert!(st.shortcuts().contains(&"1-9 gate"));
+        assert_eq!(st.handle_key(&key(KeyCode::Char('1'))), RunOutcome::Changed);
+        assert_eq!(st.handle_key(&key(KeyCode::Esc)), RunOutcome::Changed);
+
+        st.set_answered(Some("approve".to_string()));
+        assert_eq!(st.answered(), Some("approve"));
+        assert!(!st.shortcuts().contains(&"1-9 gate"));
+        assert_eq!(
+            st.handle_key(&key(KeyCode::Char('1'))),
+            RunOutcome::Unchanged
+        );
+
+        let theme = Theme::default();
+        let area = Rect::new(0, 0, 120, 35);
+        let mut buf = Buffer::empty(area);
+        st.render(&mut buf, area, &theme, 10_000);
+
+        let text = buf_text(&buf);
+        assert!(text.contains("answered at"));
+        assert!(text.contains("approve"));
+        assert!(!text.contains("NEEDS YOU"));
+        assert!(text.contains("[1] ✓ approve → land.flow.ts"));
+        assert!(text.contains("[2] reject → fix.flow.ts"));
+    }
 }
```