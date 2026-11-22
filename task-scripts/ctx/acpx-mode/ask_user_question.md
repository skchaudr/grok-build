# x.ai/ask_user_question: agent->pager ExtRequest (blocks until the user answers), response, cancel

Notes: the agent calls acp_send(acp::ExtRequest::new("x.ai/ask_user_question", raw.into()), &client_tx) and awaits the ExtResponse; the JSON may be wrapped as {"result": <value>} (the adapter reads outer.get("result").unwrap_or(&outer)). Answer = answers{<question text>: [label]}; free text comes in annotations.<question>.notes with label "Other"; cancel = {"outcome":"cancelled"}. Extra hint keys the pager reads: initialText, noFreeform.

- request/response wire types
```rust
// from crates/codegen/xai-grok-tools/src/implementations/grok_build/ask_user_question/types.rs:24-116
/// Annotation on a single question's answer. Carried inside the `accepted` response alongside the
/// selected label. `preview`: verbatim `Option.preview` of the selected option (single-select
/// only). `notes`: free-text the user typed in the freeform input.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QuestionAnnotation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Mode context for the question UI. Sent as part of the ACP `ext_method` request so the pager
/// knows whether to show plan-mode-only actions (Chat about this / Skip interview).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AskUserQuestionMode {
    /// Normal mode. Client shows only Accept and Cancel.
    Default,
    /// Plan mode. Client shows Accept, Cancel, Chat about this, Skip interview.
    Plan,
}

/// ACP `ext_method` request payload (shell coordinator sends to client/pager).
///
/// Serialized as `camelCase` for the ACP JSON-RPC wire format.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskUserQuestionExtRequest {
    pub session_id: String,
    pub tool_call_id: String,
    pub questions: Vec<Question>,
    /// Controls whether the client shows plan-mode-only actions.
    pub mode: AskUserQuestionMode,
}

/// Accepts both `"value"` (old wire format) and `["value"]` (new wire format)
/// for each answer entry, normalizing strings into single-element vectors.
fn deserialize_string_or_vec_answers<'de, D>(
    deserializer: D,
) -> Result<IndexMap<String, Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum StringOrVec {
        Vec(Vec<String>),
        String(String),
    }

    let raw: IndexMap<String, StringOrVec> = serde::Deserialize::deserialize(deserializer)?;
    Ok(raw
        .into_iter()
        .map(|(k, v)| match v {
            StringOrVec::Vec(vec) => (k, vec),
            StringOrVec::String(s) => (k, vec![s]),
        })
        .collect())
}

/// ACP `ext_method` response payload (client/pager returns to shell coordinator). Internally tagged
/// on `"outcome"` with `snake_case` variant names so the JSON looks like `{ "outcome": "accepted",
/// "answers": { ... } }`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum AskUserQuestionExtResponse {
    /// User accepted and submitted answers (Path A).
    Accepted {
        /// Answered questions in original order; unanswered omitted.
        /// One element per selected option; freeform-only is `["Other"]`
        /// with typed text in `annotations[q].notes`.
        #[serde(deserialize_with = "deserialize_string_or_vec_answers")]
        answers: IndexMap<String, Vec<String>>,
        /// Per-question annotations (preview, notes). Absent when empty.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        annotations: Option<HashMap<String, QuestionAnnotation>>,
    },
    /// User chose "Chat about this" (Path B, plan mode only).
    ChatAboutThis {
        /// Partial answers: answered questions only, label only (no notes).
        /// Freeform-only => `"Other"` (notes dropped in plan-mode paths).
        #[serde(default)]
        partial_answers: HashMap<String, String>,
    },
    /// User chose "Skip interview and plan immediately" (Path C, plan mode only).
    SkipInterview {
        /// Same partial-answer rules as `ChatAboutThis`.
        #[serde(default)]
        partial_answers: HashMap<String, String>,
    },
    /// User cancelled / dismissed (Path D). NOT an error.
    Cancelled,
}
```

- Question and QuestionOption wire types
```rust
// from crates/codegen/xai-grok-tools/src/implementations/grok_build/ask_user_question/mod.rs:134-188
/// A single option within a question.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct QuestionOption {
    /// Option text shown to the user; a few words at most.
    #[schemars(description = "Option text shown to the user. A few words at most.")]
    pub label: String,

    /// What picking this option means or implies.
    #[schemars(description = "What picking this option means or implies.")]
    pub description: String,

    /// Optional content shown while the option is focused — mockups, code
    /// snippets, anything the user should compare. Single-select only.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(
        description = "Optional content shown while the option is focused — mockups, code snippets, anything the user should compare. Single-select questions only."
    )]
    pub preview: Option<String>,

    /// Opaque id; hidden from the model. Grok callers leave it `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(skip)]
    pub id: Option<String>,
}

/// A single question with its options.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    /// The question to ask, phrased as a full question.
    #[schemars(description = "The question to ask, phrased as a full question.")]
    pub question: String,

    /// The choices for this question.
    #[schemars(description = "The choices for this question.")]
    pub options: Vec<QuestionOption>,

    /// Let the user pick more than one option (default false). Model-facing schema name is
    /// snake_case (`multi_select`); deserialize also accepts the legacy/ACP `multiSelect` so the
    /// shared `Question` type stays wire-compatible with the camelCase ACP ext_method.
    #[serde(
        default,
        alias = "multi_select",
        deserialize_with = "crate::types::schema::deserialize_lenient_option_bool"
    )]
    #[schemars(
        rename = "multi_select",
        description = "Let the user pick more than one option (default false)."
    )]
    pub multi_select: Option<bool>,

    /// See `QuestionOption.id`. Hidden from the JSON schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(skip)]
    pub id: Option<String>,
```

- extension dialog to question: select/confirm/input options and params
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/tools.rs:860-944
    pub(super) async fn ask_extension_question(&self, event: Value) -> Result<()> {
        let id = event
            .get("id")
            .cloned()
            .ok_or_else(|| anyhow!("Pi extension UI request has no id"))?;
        let method = string(&event, &["method"])
            .unwrap_or_default()
            .to_ascii_lowercase();
        let raw_title = string(&event, &["title", "message"]).unwrap_or("Pi extension");
        // grok-pi's subagent configuration uses Pi's standard `ui.select`
        // callback, but requests the existing native QuestionView multi-select
        // affordance through a namespaced title envelope. Pi core remains
        // unchanged and other extension selects keep their single-choice
        // semantics.
        let multi_select_title = (method == "select")
            .then(|| extension_multi_select_title(raw_title))
            .flatten();
        let resource_picker = (method == "select")
            .then(|| extension_resource_picker(raw_title))
            .flatten();
        let subagent_history = (method == "select")
            .then(|| extension_subagent_history(raw_title))
            .flatten();
        let resource_picker_title = resource_picker
            .as_ref()
            .and_then(|picker| picker.get("title"))
            .and_then(Value::as_str);
        let subagent_history_title = subagent_history
            .as_ref()
            .and_then(|picker| picker.get("title"))
            .and_then(Value::as_str);
        let title = resource_picker_title
            .or(subagent_history_title)
            .or(multi_select_title.as_deref())
            .unwrap_or(raw_title);
        let mut options = Vec::new();
        if method == "select" {
            for option in event
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                options.push(json!({
                    "label": option,
                    "description": "",
                    "preview": null,
                    "id": null,
                }));
            }
        } else if method == "confirm" {
            options.push(json!({ "label": "Yes", "description": "", "preview": null, "id": null }));
            options.push(json!({ "label": "No", "description": "", "preview": null, "id": null }));
        }
        let mut question = if method == "confirm" {
            string(&event, &["message"]).unwrap_or(title).to_string()
        } else {
            title.to_string()
        };
        if method == "input"
            && let Some(placeholder) = string(&event, &["placeholder"])
            && !placeholder.is_empty()
        {
            question.push_str("\n\n");
            question.push_str(placeholder);
        }
        let initial_text = if method == "editor" {
            string(&event, &["prefill"]).unwrap_or_default()
        } else {
            ""
        };
        let tool_call_id = extension_tool_call_id(&id);
        let mut params = json!({
            "sessionId": self.session_id().0.to_string(),
            "toolCallId": tool_call_id.clone(),
            "questions": [{
                "question": question,
                "options": options,
                "multiSelect": multi_select_title.is_some(),
                "id": "pi-question",
            }],
            "mode": "default",
            "initialText": initial_text,
            "noFreeform": method == "select" || method == "confirm",
```

- send request (with optional timeout retract), decode, cancelled outcome, answer reply
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/tools.rs:945-1016
        });
        if let Some(resource_picker) = resource_picker.clone() {
            params
                .as_object_mut()
                .expect("extension question params must be an object")
                .insert("piGrokResourcePicker".to_owned(), resource_picker);
        }
        if let Some(subagent_history) = subagent_history.clone() {
            params
                .as_object_mut()
                .expect("extension question params must be an object")
                .insert("piGrokSubagentHistory".to_owned(), subagent_history);
        }
        let raw = serde_json::value::to_raw_value(&params)?;
        let request = acp::ExtRequest::new("x.ai/ask_user_question", raw.into());
        let response = match extension_dialog_timeout(&event) {
            Some(duration) => {
                match tokio::time::timeout(duration, acp_send(request, &self.client_tx)).await {
                    Ok(response) => response.map_err(|error| anyhow!(error.to_string()))?,
                    Err(_) => {
                        // Pi resolves its own dialog promise on the same timeout but
                        // does not emit a cancellation event. Explicitly retract the
                        // native Grok QuestionView so it cannot remain as a zombie
                        // overlay after the extension has resumed.
                        self.send_ext_notification(
                            "pi/ui/cancel_interaction",
                            json!({ "toolCallId": tool_call_id }),
                        )
                        .await;
                        self.respond_extension_cancelled(&event);
                        return Ok(());
                    }
                }
            }
            None => acp_send(request, &self.client_tx)
                .await
                .map_err(|error| anyhow!(error.to_string()))?,
        };
        let outer: Value = serde_json::from_str(response.0.get())?;
        let result = outer.get("result").unwrap_or(&outer);
        if result.get("outcome").and_then(Value::as_str) == Some("cancelled") {
            self.rpc.notify(json!({
                "type": "extension_ui_response",
                "id": id,
                "cancelled": true,
            }))?;
            return Ok(());
        }
        let answer = if resource_picker.is_some() {
            extension_resource_picker_answer(result).unwrap_or_else(|| "[]".to_string())
        } else if subagent_history.is_some() {
            extension_subagent_history_answer(result).unwrap_or_default()
        } else if multi_select_title.is_some() {
            extension_multi_select_answer(result).unwrap_or_else(|| "[]".to_string())
        } else {
            extension_answer(&method, result).unwrap_or_default()
        };
        let response = match method.as_str() {
            "confirm" => json!({
                "type": "extension_ui_response",
                "id": id,
                "confirmed": answer.eq_ignore_ascii_case("yes"),
            }),
            _ => json!({
                "type": "extension_ui_response",
                "id": id,
                "value": answer,
            }),
        };
        self.rpc.notify(response)?;
        Ok(())
    }
```

- request_ask_user_question: the same request for a tool call (questions array, mode)
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter/tools.rs:258-332
    pub(super) async fn request_ask_user_question(&self, tool_call_id: &str, args: Option<Value>) {
        let Some(_dir) = std::env::var_os("PI_GROK_ASK_USER_DIR") else {
            write_ask_user_response(
                tool_call_id,
                json!({
                    "outcome": "error",
                    "message": "ask_user_question host control missing (enable F2 Q&A and restart).",
                }),
            );
            return;
        };
        let questions = normalize_ask_user_questions(args.as_ref());
        if questions.is_empty() {
            write_ask_user_response(
                tool_call_id,
                json!({
                    "outcome": "error",
                    "message": "ask_user_question requires at least one question with options.",
                }),
            );
            return;
        }
        let mode = if self.state.borrow().plan_mode.is_active() {
            "plan"
        } else {
            "default"
        };
        let params = json!({
            "sessionId": self.session_id().0.to_string(),
            "toolCallId": tool_call_id,
            "questions": questions,
            "mode": mode,
        });
        let raw = match serde_json::value::to_raw_value(&params) {
            Ok(raw) => raw,
            Err(error) => {
                write_ask_user_response(
                    tool_call_id,
                    json!({
                        "outcome": "error",
                        "message": format!("failed to serialize Q&A request: {error}"),
                    }),
                );
                return;
            }
        };
        let request = acp::ExtRequest::new("x.ai/ask_user_question", raw.into());
        let response = match acp_send(request, &self.client_tx).await {
            Ok(response) => response,
            Err(error) => {
                write_ask_user_response(
                    tool_call_id,
                    json!({
                        "outcome": "error",
                        "message": format!("Q&A request failed: {error}"),
                    }),
                );
                return;
            }
        };
        let outer: Value = match serde_json::from_str(response.0.get()) {
            Ok(value) => value,
            Err(error) => {
                write_ask_user_response(
                    tool_call_id,
                    json!({
                        "outcome": "error",
                        "message": format!("invalid Q&A response: {error}"),
                    }),
                );
                return;
            }
        };
        let result = outer.get("result").unwrap_or(&outer);
        write_ask_user_response(tool_call_id, format_ask_user_tool_result(result));
```

- tool call id for a dialog
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter.rs:1397-1403
fn extension_tool_call_id(id: &Value) -> String {
    let id = id
        .as_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| id.to_string());
    format!("pi-extension-ui:{id}")
}
```

- answer extraction helpers
```rust
// from crates/codegen/pi-grok-adapter/src/pi_adapter.rs:1648-1696
fn selected_answer(value: &Value) -> Option<String> {
    let answers = value.get("answers").and_then(Value::as_object)?;
    for answer in answers.values() {
        if let Some(text) = answer.as_str() {
            return Some(text.to_string());
        }
        if let Some(text) = answer
            .as_array()
            .and_then(|items| items.first())
            .and_then(Value::as_str)
        {
            return Some(text.to_string());
        }
    }
    None
}

fn annotated_answer(value: &Value) -> Option<String> {
    let annotations = value.get("annotations").and_then(Value::as_object)?;
    for annotation in annotations.values() {
        if let Some(notes) = annotation.get("notes").and_then(Value::as_str) {
            return Some(notes.to_string());
        }
    }
    None
}

/// Translate Grok QuestionView's response into the value Pi expects.
///
/// Freeform rows are represented by the native question component as the
/// selected option `Other`, with the actual editor text under
/// `annotations.<question>.notes`. Pi input/editor must therefore prefer notes;
/// select/confirm must prefer the selected option.
fn extension_answer(method: &str, value: &Value) -> Option<String> {
    let direct = || {
        value
            .get("value")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    };
    match method {
        "input" | "editor" => annotated_answer(value)
            .or_else(|| selected_answer(value))
            .or_else(direct),
        _ => selected_answer(value)
            .or_else(|| annotated_answer(value))
            .or_else(direct),
    }
}
```

- pager side: cancel an open question when the agent withdraws it (pi/ui/cancel_interaction)
```rust
// from crates/codegen/xai-grok-pager/src/app/acp_handler/pi_ui.rs:170-178
        "pi/ui/cancel_interaction" => {
            let Some(tool_call_id) = str_of(&["toolCallId", "tool_call_id"]) else {
                return false;
            };
            let mut changed = false;
            for agent in app.agents.values_mut() {
                changed |= dismiss(agent, &tool_call_id);
            }
            changed
```

- pager side: parse request, route by sessionId
```rust
// from crates/codegen/xai-grok-pager/src/app/acp_handler/interactions.rs:239-300
pub(crate) fn handle_ask_user_question(
    ext: xai_acp_lib::AcpArgs<acp::ExtRequest>,
    app: &mut AppView,
) -> bool {
    use crate::views::question_view::QuestionViewState;
    use xai_grok_tools::implementations::grok_build::ask_user_question::{
        AskUserQuestionExtRequest, AskUserQuestionExtResponse,
    };

    // Parse both the typed request and the narrow Pi adapter extensions.
    // `initialText` and `noFreeform` are deliberately client-side hints: the
    // canonical ask-user-question wire type remains unchanged.
    let raw_params: serde_json::Value = match serde_json::from_str(ext.request.params.get()) {
        Ok(value) => value,
        Err(e) => {
            tracing::error!(error = %e, "Failed to parse ask-user-question params");
            ext.response_tx
                .send(Err(acp::Error::new(-32602, format!("Invalid params: {e}"))))
                .ok();
            return false;
        }
    };
    let initial_text = raw_params
        .get("initialText")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let no_freeform = raw_params
        .get("noFreeform")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let resource_picker_request = match pi_grok_resource_picker_request(&raw_params) {
        Ok(request) => request,
        Err(error) => {
            ext.response_tx
                .send(Err(acp::Error::new(-32602, error)))
                .ok();
            return false;
        }
    };
    let subagent_history_request = match pi_grok_subagent_history_request(&raw_params) {
        Ok(request) => request,
        Err(error) => {
            ext.response_tx
                .send(Err(acp::Error::new(-32602, error)))
                .ok();
            return false;
        }
    };

    let ext_req: AskUserQuestionExtRequest = match serde_json::from_value(raw_params.clone()) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "Failed to parse AskUserQuestionExtRequest");
            ext.response_tx
                .send(Err(acp::Error::new(-32602, format!("Invalid params: {e}"))))
                .ok();
            return false;
        }
    };

    // Route by the request's session id (like `session/update`), so a question raised by a BACKGROUND session lands on its own view
    // The user may be on the dashboard or another session and never have entered this one; the question must not fail for that
```
