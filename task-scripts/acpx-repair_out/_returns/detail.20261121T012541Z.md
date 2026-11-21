```diff
--- a/crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs
+++ b/crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs
@@ -10,8 +10,7 @@
 use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
 
 use crate::acpx::bundle::{
-    self, AgentSettings, NodeKind, NodeState, Outcome, Run, RunStatus, Step, fmt_duration,
-    value_text,
+    self, AgentSettings, NodeKind, NodeState, Outcome, Run, RunStatus, Step, fmt_duration,
 };
 use crate::render::line_utils::truncate_str;
 use crate::theme::Theme;
@@ -370,31 +369,19 @@
 
     // Last result
     if let Some(last_step) = run.steps.last() {
-        let val = value_text(&last_step.output);
-        if !val.trim().is_empty() {
+        let lines = format_value_lines(&last_step.output, width, theme);
+        if !lines.is_empty() {
             push_section_rule(&mut out, "Last result", width, theme);
-            push_wrapped_capped(
-                &mut out,
-                &val,
-                width,
-                20,
-                Style::default().fg(theme.text_primary),
-                theme,
-            );
+            out.extend(lines);
         }
     }
 
     // Input
-    let input_text = value_text(&run.input);
-    if !input_text.trim().is_empty() {
+    let input_lines = format_value_lines(&run.input, width, theme);
+    if !input_lines.is_empty() {
         push_section_rule(&mut out, "Input", width, theme);
-        push_wrapped_capped(
-            &mut out,
-            &input_text,
-            width,
-            12,
-            Style::default().fg(theme.text_secondary),
-            theme,
+        out.extend(
+            input_lines,
         );
     }
 
@@ -420,11 +407,181 @@
     match &step.output {
         Value::Null => String::new(),
         Value::String(s) => first_non_empty_line(s),
-        other => serde_json::to_string(other).unwrap_or_default(),
+        other => value_inline_summary(other),
     }
 }
 
+fn value_inline_summary(val: &Value) -> String {
+    match val {
+        Value::Null => String::new(),
+        Value::Bool(b) => b.to_string(),
+        Value::Number(n) => n.to_string(),
+        Value::String(s) => first_non_empty_line(s),
+        Value::Array(arr) => arr
+            .iter()
+            .map(value_inline_summary)
+            .filter(|s| !s.is_empty())
+            .collect::<Vec<_>>()
+            .join(", "),
+        Value::Object(map) => map
+            .iter()
+            .map(|(k, v)| {
+                let vs = value_inline_summary(v);
+                if vs.is_empty() {
+                    k.clone()
+                } else {
+                    format!("{k} {vs}")
+                }
+            })
+            .collect::<Vec<_>>()
+            .join(", "),
+    }
+}
+
+fn format_value_lines(val: &Value, width: usize, theme: &Theme) -> Vec<Line<'static>> {
+    let mut out = Vec::new();
+    if matches!(val, Value::Null) {
+        return out;
+    }
+    if let Value::String(s) = val {
+        if s.trim().is_empty() {
+            return out;
+        }
+    }
+    append_value_lines(&mut out, val, 0, width, theme);
+    out
+}
+
+fn append_value_lines(
+    out: &mut Vec<Line<'static>>,
+    val: &Value,
+    indent: usize,
+    width: usize,
+    theme: &Theme,
+) {
+    let val_style = Style::default().fg(theme.text_primary);
+    let key_style = Style::default().fg(theme.gray);
+    let pad_indent = " ".repeat(indent);
+    let body_w = width.saturating_sub(indent).max(1);
+
+    match val {
+        Value::Null => {
+            if indent > 0 {
+                out.push(Line::from(vec![
+                    Span::raw(pad_indent),
+                    Span::styled("null", val_style),
+                ]));
+            }
+        }
+        Value::Bool(b) => {
+            out.push(Line::from(vec![
+                Span::raw(pad_indent),
+                Span::styled(b.to_string(), val_style),
+            ]));
+        }
+        Value::Number(n) => {
+            out.push(Line::from(vec![
+                Span::raw(pad_indent),
+                Span::styled(n.to_string(), val_style),
+            ]));
+        }
+        Value::String(s) => {
+            for line in wrap(s, body_w) {
+                if indent == 0 {
+                    out.push(Line::from(Span::styled(line, val_style)));
+                } else {
+                    out.push(Line::from(vec![
+                        Span::raw(pad_indent.clone()),
+                        Span::styled(line, val_style),
+                    ]));
+                }
+            }
+        }
+        Value::Array(arr) => {
+            for item in arr {
+                let bullet_prefix = format!("{pad_indent}- ");
+                let item_w = width.saturating_sub(indent + 2).max(1);
+                match item {
+                    Value::Object(_) | Value::Array(_) => {
+                        let mut sub = Vec::new();
+                        append_value_lines(&mut sub, item, 0, item_w, theme);
+                        for (idx, sub_line) in sub.into_iter().enumerate() {
+                            let mut spans = vec![Span::styled(
+                                if idx == 0 {
+                                    bullet_prefix.clone()
+                                } else {
+                                    format!("{pad_indent}  ")
+                                },
+                                key_style,
+                            )];
+                            spans.extend(sub_line.spans);
+                            out.push(Line::from(spans));
+                        }
+                    }
+                    _ => {
+                        let s = match item {
+                            Value::Null => "null".to_string(),
+                            Value::Bool(b) => b.to_string(),
+                            Value::Number(n) => n.to_string(),
+                            Value::String(s) => s.clone(),
+                            _ => String::new(),
+                        };
+                        let wrapped = wrap(&s, item_w);
+                        for (idx, line) in wrapped.into_iter().enumerate() {
+                            let pfx = if idx == 0 {
+                                bullet_prefix.clone()
+                            } else {
+                                format!("{pad_indent}  ")
+                            };
+                            out.push(Line::from(vec![
+                                Span::styled(pfx, key_style),
+                                Span::styled(line, val_style),
+                            ]));
+                        }
+                    }
+                }
+            }
+        }
+        Value::Object(map) => {
+            let max_key_w = map
+                .keys()
+                .map(|k| UnicodeWidthStr::width(k.as_str()))
+                .max()
+                .unwrap_or(0);
+            for (k, v) in map {
+                let kw = UnicodeWidthStr::width(k.as_str());
+                let key_pad = " ".repeat(max_key_w.saturating_sub(kw) + 2);
+                match v {
+                    Value::Object(sub_map) if sub_map.is_empty() => {
+                        out.push(Line::from(vec![
+                            Span::raw(pad_indent.clone()),
+                            Span::styled(k.clone(), key_style),
+                        ]));
+                    }
+                    Value::Array(sub_arr) if sub_arr.is_empty() => {
+                        out.push(Line::from(vec![
+                            Span::raw(pad_indent.clone()),
+                            Span::styled(k.clone(), key_style),
+                        ]));
+                    }
+                    Value::Object(_) | Value::Array(_) => {
+                        out.push(Line::from(vec![
+                            Span::raw(pad_indent.clone()),
+                            Span::styled(k.clone(), key_style),
+                        ]));
+                        append_value_lines(out, v, indent + 2, width, theme);
+                    }
+                    _ => {
+                        let s = match v {
+                            Value::Null => "null".to_string(),
+                            Value::Bool(b) => b.to_string(),
+                            Value::Number(n) => n.to_string(),
+                            Value::String(s) => s.clone(),
+                            _ => String::new(),
+                        };
+                        let prefix_w = indent + max_key_w + 2;
+                        let val_w = width.saturating_sub(prefix_w).max(1);
+                        let wrapped = wrap(&s, val_w);
+                        for (idx, line) in wrapped.into_iter().enumerate() {
+                            if idx == 0 {
+                                out.push(Line::from(vec![
+                                    Span::raw(pad_indent.clone()),
+                                    Span::styled(format!("{k}{key_pad}"), key_style),
+                                    Span::styled(line, val_style),
+                                ]));
+                            } else {
+                                out.push(Line::from(vec![
+                                    Span::raw(" ".repeat(prefix_w)),
+                                    Span::styled(line, val_style),
+                                ]));
+                            }
+                        }
+                    }
+                }
+            }
+        }
+    }
+}
+
 fn first_non_empty_line(s: &str) -> String {
     s.lines()
         .find(|l| !l.trim().is_empty())
@@ -564,17 +721,10 @@
     }
 
     // Result
-    let result_text = value_text(&step.output);
-    if !result_text.trim().is_empty() {
+    let result_lines = format_value_lines(&step.output, width, theme);
+    if !result_lines.is_empty() {
         push_section_rule(&mut out, "Result", width, theme);
-        push_wrapped_capped(
-            &mut out,
-            &result_text,
-            width,
-            40,
-            Style::default().fg(theme.text_primary),
-            theme,
-        );
+        out.extend(result_lines);
     }
 
     // stdout / stderr for actions
@@ -1482,6 +1632,126 @@
     }
 
     #[test]
+    fn test_last_result_object_and_array_renders_clean_without_json_punctuation() {
+        let tmp = tempfile::tempdir().expect("tempdir");
+        let mut run = make_test_run(tmp.path());
+        run.steps.push(Step {
+            attempt_id: "list#1".to_string(),
+            node_id: "list".to_string(),
+            kind: NodeKind::Action,
+            outcome: Outcome::Ok,
+            started_ms: 1_000,
+            finished_ms: 1_100,
+            prompt_text: None,
+            raw_text: None,
+            output: json!({
+                "summary": "all good",
+                "files": ["a.rs", "b.rs"]
+            }),
+            error: None,
+            agent_name: None,
+            action: None,
+            conversation: None,
+            stdout: None,
+            stderr: None,
+            output_artifact: None,
+        });
+
+        let mut panel = DetailPanel::new();
+        let mut buf = Buffer::empty(Rect::new(0, 0, 90, 35));
+        panel.render(
+            &mut buf,
+            Rect::new(0, 0, 90, 35),
+            &run,
+            &DetailTarget::Run,
+            &Theme::default(),
+            5_000,
+        );
+        let rows = buffer_lines(&buf);
+        let stage_row = rows
+            .iter()
+            .find(|r| r.contains("✓ list"))
+            .expect("list stage row");
+        assert!(stage_row.contains("summary all good"));
+        assert!(!stage_row.contains('{'));
+        assert!(!stage_row.contains('"'));
+
+        let lr_idx = rows
+            .iter()
+            .position(|r| r.starts_with("── Last result"))
+            .expect("Last result rule");
+        let inp_idx = rows
+            .iter()
+            .position(|r| r.starts_with("── Input"))
+            .expect("Input rule");
+        let section = &rows[lr_idx + 1..inp_idx];
+        assert!(section.iter().any(|r| r.contains("summary") && r.contains("all good")));
+        assert!(section.iter().any(|r| r.contains("a.rs")));
+        assert!(section.iter().any(|r| r.contains("b.rs")));
+        for line in section {
+            assert!(!line.contains('{'));
+            assert!(!line.contains('}'));
+            assert!(!line.contains('['));
+            assert!(!line.contains("\"summary\":"));
+        }
+    }
+
+    #[test]
+    fn test_string_output_wraps_and_empty_or_null_hides_section() {
+        let tmp = tempfile::tempdir().expect("tempdir");
+        let mut run = make_test_run(tmp.path());
+        run.input = json!({});
+        run.steps.push(Step {
+            attempt_id: "work#1".to_string(),
+            node_id: "work".to_string(),
+            kind: NodeKind::Acp,
+            outcome: Outcome::Ok,
+            started_ms: 1_000,
+            finished_ms: 2_000,
+            prompt_text: None,
+            raw_text: None,
+            output: json!("plain wrapped text"),
+            error: None,
+            agent_name: None,
+            action: None,
+            conversation: None,
+            stdout: None,
+            stderr: None,
+            output_artifact: None,
+        });
+
+        let lines = build_run_lines(&run, 60, &Theme::default(), 5_000);
+        let text: Vec<String> = lines
+            .iter()
+            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
+            .collect();
+        let joined = text.join("\n");
+        assert!(joined.contains("── Last result"));
+        assert!(joined.contains("plain wrapped text"));
+        assert!(!joined.contains("── Input"));
+
+        run.steps[0].output = Value::Null;
+        let lines_null = build_run_lines(&run, 60, &Theme::default(), 5_000);
+        let joined_null: String = lines_null
+            .iter()
+            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>())
+            .collect::<Vec<_>>()
+            .join("\n");
+        assert!(!joined_null.contains("── Last result"));
+
+        run.steps[0].output = json!({});
+        let lines_empty = build_run_lines(&run, 60, &Theme::default(), 5_000);
+        let joined_empty: String = lines_empty
+            .iter()
+            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>())
+            .collect::<Vec<_>>()
+            .join("\n");
+        assert!(!joined_empty.contains("── Last result"));
+    }
+
+    #[test]
+    fn test_array_output_of_40_items_renders_all_without_cap() {
+        let tmp = tempfile::tempdir().expect("tempdir");
+        let mut run = make_test_run(tmp.path());
+        let items: Vec<String> = (0..40).map(|i| format!("item-{i}")).collect();
+        run.steps.push(Step {
+            attempt_id: "work#1".to_string(),
+            node_id: "work".to_string(),
+            kind: NodeKind::Acp,
+            outcome: Outcome::Ok,
+            started_ms: 1_000,
+            finished_ms: 2_000,
+            prompt_text: None,
+            raw_text: None,
+            output: json!(items),
+            error: None,
+            agent_name: None,
+            action: None,
+            conversation: None,
+            stdout: None,
+            stderr: None,
+            output_artifact: None,
+        });
+
+        let lines = build_run_lines(&run, 60, &Theme::default(), 5_000);
+        let text: Vec<String> = lines
+            .iter()
+            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
+            .collect();
+        for i in 0..40 {
+            assert!(text.iter().any(|r| r.contains(&format!("item-{i}"))));
+        }
+        assert!(!text.iter().any(|r| r.contains("more rows")));
+    }
+
+    #[test]
     fn test_small_and_zero_rects_do_not_panic_and_stay_inside_area() {
         let tmp = tempfile::tempdir().expect("tempdir");
         let run = make_test_run(tmp.path());
```