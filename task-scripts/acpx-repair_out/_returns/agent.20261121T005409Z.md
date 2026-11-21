```diff
--- a/crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs
+++ b/crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs
@@ -579,7 +579,7 @@
             let _ = acp_send(notification, &tx).await;
         });
 
-        Ok(acp::NewSessionResponse::new(session_id))
+        Ok(acp::NewSessionResponse::new(session_id).models(session_model_state()))
     }
 
     async fn load_session(
@@ -596,7 +596,7 @@
                 .await;
         }
 
-        Ok(acp::LoadSessionResponse::new())
+        Ok(acp::LoadSessionResponse::new().models(session_model_state()))
     }
 
     async fn list_sessions(
@@ -918,6 +918,13 @@
     json!({ "task": rest })
 }
 
+fn session_model_state() -> acp::SessionModelState {
+    acp::SessionModelState::new(
+        "acpx",
+        vec![acp::ModelInfo::new("acpx", "acpx")],
+    )
+}
+
 fn available_commands() -> Vec<acp::AvailableCommand> {
     vec![
         acp::AvailableCommand::new("flow", "Run an acpx flow file").input(
@@ -1291,4 +1298,14 @@
         });
         assert_eq!(extract_gate_answer_label(&wrapped_cancelled), None);
     }
+
+    #[test]
+    fn session_model_state_has_valid_current_model() {
+        let state = session_model_state();
+        assert!(!state.current_model_id.0.is_empty());
+        assert!(
+            state.available_models.iter().any(|m| m.model_id == state.current_model_id),
+            "current_model_id must be present in available_models"
+        );
+    }
 }
```