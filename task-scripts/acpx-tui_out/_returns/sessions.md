```rust
use std::fs;
use std::path::{Path, PathBuf};

use chrono::DateTime;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct SessionSummary {
    pub record_id: String,
    pub acp_session_id: String,
    pub name: String,
    pub title: String,
    pub agent: String,
    pub cwd: String,
    pub created_ms: i64,
    pub last_used_ms: i64,
    pub closed: bool,
    pub messages: usize,
    pub tokens: u64,
    pub cost_usd: Option<f64>,
    pub stream_path: PathBuf,
}

pub fn sessions_base() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())?;
    Some(PathBuf::from(home).join(".acpx").join("sessions"))
}

pub fn list_sessions(base: &Path) -> Vec<SessionSummary> {
    let Ok(entries) = fs::read_dir(base) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !ft.is_file() {
            continue;
        }
        let is_json = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"));
        if !is_json {
            continue;
        }
        if let Some(summary) = parse_session_file(base, &path) {
            out.push(summary);
        }
    }

    out.sort_by(|a, b| {
        b.last_used_ms
            .cmp(&a.last_used_ms)
            .then_with(|| b.created_ms.cmp(&a.created_ms))
            .then_with(|| a.record_id.cmp(&b.record_id))
    });
    out
}

pub fn agent_label(agent_command: &str) -> String {
    let mut words = agent_command.split_whitespace().peekable();
    let Some(first) = words.peek().copied() else {
        return "?".to_string();
    };

    let first_base = basename_str(first);
    if first_base == "env" {
        words.next();
    }

    while let Some(&word) = words.peek() {
        if word == "-u" || word == "--unset" {
            words.next();
            words.next();
            continue;
        }
        if word.starts_with("-u") && word.len() > 2 {
            words.next();
            continue;
        }
        if let Some(rest) = word.strip_prefix("--unset=") {
            let _ = rest;
            words.next();
            continue;
        }
        if word == "-i" || word == "-" || word == "--ignore-environment" {
            words.next();
            continue;
        }
        if is_env_assignment(word) {
            words.next();
            continue;
        }
        break;
    }

    let Some(cmd) = words.next() else {
        return "?".to_string();
    };

    let base = basename_str(cmd);
    if base.is_empty() {
        return "?".to_string();
    }

    let stripped = base
        .strip_suffix("-agent-acp")
        .or_else(|| base.strip_suffix("-code-acp"))
        .or_else(|| base.strip_suffix("-acp"))
        .unwrap_or(base);

    if stripped.is_empty() {
        "?".to_string()
    } else {
        stripped.to_string()
    }
}

fn basename_str(s: &str) -> &str {
    let trimmed = s.trim_end_matches(['/', '\\']);
    match trimmed.rfind(['/', '\\']) {
        Some(idx) => &trimmed[idx + 1..],
        None => trimmed,
    }
}

fn is_env_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    if name.is_empty() || name.contains(['/', '\\']) {
        return false;
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn parse_session_file(base: &Path, path: &Path) -> Option<SessionSummary> {
    let bytes = fs::read(path).ok()?;
    let val: Value = serde_json::from_slice(&bytes).ok()?;
    let obj = val.as_object()?;

    let record_id = get_non_empty_str(obj, &["acpx_record_id", "acpxRecordId"])?;
    let acp_session_id =
        get_str(obj, &["acp_session_id", "acpSessionId"]).unwrap_or_default();
    let name = get_str(obj, &["name"]).unwrap_or_default();
    let raw_title = get_str(obj, &["title"]).unwrap_or_default();

    let title = if !raw_title.is_empty() {
        raw_title
    } else if !name.is_empty() {
        name.clone()
    } else {
        record_id.clone()
    };

    let agent_cmd = get_str(obj, &["agent_command", "agentCommand"])
        .filter(|s| !s.is_empty())
        .or_else(|| {
            get_field(obj, &["agent_argv", "agentArgv"])
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .filter(|s| !s.is_empty())
        })
        .or_else(|| get_str(obj, &["agent_name", "agentName"]).filter(|s| !s.is_empty()))
        .unwrap_or_default();

    let agent = agent_label(&agent_cmd);
    let cwd = get_str(obj, &["cwd"]).unwrap_or_default();

    let created_ms = get_str(obj, &["created_at", "createdAt"])
        .map(|s| parse_ts_ms(&s))
        .unwrap_or(0);

    let last_used_ms = get_str(
        obj,
        &["last_used_at", "lastUsedAt", "updated_at", "updatedAt"],
    )
    .map(|s| parse_ts_ms(&s))
    .unwrap_or(created_ms);

    let closed = get_field(obj, &["closed"])
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let messages = get_field(obj, &["messages"])
        .and_then(Value::as_array)
        .map_or(0, Vec::len);

    let tokens = get_field(obj, &["cumulative_token_usage", "cumulativeTokenUsage"])
        .map(extract_tokens)
        .unwrap_or(0);

    let cost_usd =
        get_field(obj, &["cumulative_cost", "cumulativeCost"]).and_then(extract_cost);

    let stream_path = base.join(format!("{record_id}.stream.ndjson"));

    Some(SessionSummary {
        record_id,
        acp_session_id,
        name,
        title,
        agent,
        cwd,
        created_ms,
        last_used_ms,
        closed,
        messages,
        tokens,
        cost_usd,
        stream_path,
    })
}

fn get_field<'a>(
    obj: &'a serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<&'a Value> {
    for &k in keys {
        if let Some(v) = obj.get(k) {
            if !v.is_null() {
                return Some(v);
            }
        }
    }
    None
}

fn get_str(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    for &k in keys {
        if let Some(s) = obj.get(k).and_then(Value::as_str) {
            return Some(s.to_string());
        }
    }
    None
}

fn get_non_empty_str(
    obj: &serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<String> {
    for &k in keys {
        if let Some(s) = obj.get(k).and_then(Value::as_str) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn parse_ts_ms(s: &str) -> i64 {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(0)
}

fn value_as_u64(v: &Value) -> Option<u64> {
    if let Some(u) = v.as_u64() {
        Some(u)
    } else if let Some(i) = v.as_i64() {
        u64::try_from(i).ok()
    } else if let Some(f) = v.as_f64() {
        if f.is_finite() && f >= 0.0 {
            Some(f as u64)
        } else {
            None
        }
    } else {
        None
    }
}

fn extract_tokens(val: &Value) -> u64 {
    if let Some(n) = value_as_u64(val) {
        return n;
    }
    if let Some(map) = val.as_object() {
        for key in ["total", "total_tokens", "totalTokens"] {
            if let Some(v) = map.get(key).and_then(value_as_u64) {
                return v;
            }
        }
        let mut sum: u64 = 0;
        for v in map.values() {
            if let Some(n) = value_as_u64(v) {
                sum = sum.saturating_add(n);
            }
        }
        return sum;
    }
    0
}

fn extract_cost(val: &Value) -> Option<f64> {
    if let Some(f) = val.as_f64().filter(|f| f.is_finite()) {
        return Some(f);
    }
    if let Some(map) = val.as_object() {
        for key in ["amount", "usd", "total"] {
            if let Some(f) = map
                .get(key)
                .and_then(Value::as_f64)
                .filter(|f| f.is_finite())
            {
                return Some(f);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_json(dir: &Path, filename: &str, value: &Value) {
        let path = dir.join(filename);
        let text = serde_json::to_string(value).unwrap_or_default();
        let _ = fs::write(path, text);
    }

    #[test]
    fn test_ordering_by_last_use() {
        let Ok(tmp) = tempfile::tempdir() else {
            panic!("tempdir failed");
        };
        write_json(
            tmp.path(),
            "older.json",
            &json!({
                "acpx_record_id": "rec-older",
                "acp_session_id": "sess-1",
                "name": "older",
                "created_at": "2026-11-18T19:00:00Z",
                "last_used_at": "2026-11-18T19:10:00Z"
            }),
        );
        write_json(
            tmp.path(),
            "newer.json",
            &json!({
                "acpx_record_id": "rec-newer",
                "acp_session_id": "sess-2",
                "name": "newer",
                "created_at": "2026-11-18T18:00:00Z",
                "last_used_at": "2026-11-18T19:20:00Z"
            }),
        );
        write_json(
            tmp.path(),
            "middle.json",
            &json!({
                "acpx_record_id": "rec-middle",
                "acp_session_id": "sess-3",
                "name": "middle",
                "created_at": "2026-11-18T19:05:00Z",
                "last_used_at": "2026-11-18T19:15:00Z"
            }),
        );

        let list = list_sessions(tmp.path());
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].record_id, "rec-newer");
        assert_eq!(list[1].record_id, "rec-middle");
        assert_eq!(list[2].record_id, "rec-older");
        assert_eq!(
            list[0].stream_path,
            tmp.path().join("rec-newer.stream.ndjson")
        );
    }

    #[test]
    fn test_title_fallbacks() {
        let Ok(tmp) = tempfile::tempdir() else {
            panic!("tempdir failed");
        };
        write_json(
            tmp.path(),
            "s1.json",
            &json!({
                "acpx_record_id": "id-1",
                "name": "my-name",
                "title": "Explicit Title",
                "last_used_at": "2026-11-18T19:03:00Z"
            }),
        );
        write_json(
            tmp.path(),
            "s2.json",
            &json!({
                "acpx_record_id": "id-2",
                "name": "fallback-name",
                "title": "",
                "last_used_at": "2026-11-18T19:02:00Z"
            }),
        );
        write_json(
            tmp.path(),
            "s3.json",
            &json!({
                "acpx_record_id": "id-3",
                "last_used_at": "2026-11-18T19:01:00Z"
            }),
        );

        let list = list_sessions(tmp.path());
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].title, "Explicit Title");
        assert_eq!(list[1].title, "fallback-name");
        assert_eq!(list[2].title, "id-3");
    }

    #[test]
    fn test_both_casings_and_bundle_sample() {
        let Ok(tmp) = tempfile::tempdir() else {
            panic!("tempdir failed");
        };
        write_json(
            tmp.path(),
            "snake.json",
            &json!({
                "schema": "acpx.session.v1",
                "acpx_record_id": "00000000-0000-4000-8000-000000000002",
                "acp_session_id": "00000000-0000-4000-8000-000000000000",
                "agent_command": "env -u CLAUDECODE CLAUDE_CODE_EXECUTABLE=/Users/me/.local/bin/claude /Users/me/.local/bin/claude-agent-acp",
                "cwd": "/private/tmp/acpx-feel",
                "name": "feel-main-c20d0796-4832e848",
                "title": "Directory size classification",
                "created_at": "2026-11-18T19:12:15.617Z",
                "last_used_at": "2026-11-18T19:12:25.549Z",
                "closed": false,
                "messages": [{"User": {}}, {"Agent": {}}],
                "cumulative_token_usage": {
                    "input_tokens": 6,
                    "output_tokens": 233,
                    "cache_creation_input_tokens": 9220,
                    "cache_read_input_tokens": 34694,
                    "total_tokens": 44153
                },
                "cumulative_cost": {"amount": 0.2960819, "currency": "USD"}
            }),
        );
        write_json(
            tmp.path(),
            "camel.json",
            &json!({
                "acpxRecordId": "rec-camel",
                "acpSessionId": "sess-camel",
                "agentCommand": "codex-acp --model o3",
                "cwd": "/work/dir",
                "name": "camel-session",
                "createdAt": "2026-11-18T10:00:00Z",
                "lastUsedAt": "2026-11-18T11:00:00Z",
                "closed": true,
                "messages": [{"a": 1}, {"b": 2}, {"c": 3}],
                "cumulativeTokenUsage": {"totalTokens": 999},
                "cumulativeCost": {"usd": 1.25}
            }),
        );

        let list = list_sessions(tmp.path());
        assert_eq!(list.len(), 2);

        let snake = &list[0];
        assert_eq!(snake.record_id, "00000000-0000-4000-8000-000000000002");
        assert_eq!(snake.acp_session_id, "00000000-0000-4000-8000-000000000000");
        assert_eq!(snake.agent, "claude");
        assert_eq!(snake.cwd, "/private/tmp/acpx-feel");
        assert_eq!(snake.title, "Directory size classification");
        assert!(!snake.closed);
        assert_eq!(snake.messages, 2);
        assert_eq!(snake.tokens, 44153);
        assert_eq!(snake.cost_usd, Some(0.2960819));

        let camel = &list[1];
        assert_eq!(camel.record_id, "rec-camel");
        assert_eq!(camel.acp_session_id, "sess-camel");
        assert_eq!(camel.agent, "codex");
        assert_eq!(camel.cwd, "/work/dir");
        assert_eq!(camel.title, "camel-session");
        assert!(camel.closed);
        assert_eq!(camel.messages, 3);
        assert_eq!(camel.tokens, 999);
        assert_eq!(camel.cost_usd, Some(1.25));
    }

    #[test]
    fn test_token_object_vs_number() {
        let Ok(tmp) = tempfile::tempdir() else {
            panic!("tempdir failed");
        };
        write_json(
            tmp.path(),
            "num.json",
            &json!({
                "acpx_record_id": "t-num",
                "last_used_at": "2026-11-18T19:04:00Z",
                "cumulative_token_usage": 12345
            }),
        );
        write_json(
            tmp.path(),
            "total_field.json",
            &json!({
                "acpx_record_id": "t-total",
                "last_used_at": "2026-11-18T19:03:00Z",
                "cumulative_token_usage": {
                    "input_tokens": 10,
                    "output_tokens": 20,
                    "total": 500
                }
            }),
        );
        write_json(
            tmp.path(),
            "sum_fields.json",
            &json!({
                "acpx_record_id": "t-sum",
                "last_used_at": "2026-11-18T19:02:00Z",
                "cumulative_token_usage": {
                    "input_tokens": 100,
                    "output_tokens": 250,
                    "cache_read": 50
                }
            }),
        );
        write_json(
            tmp.path(),
            "absent.json",
            &json!({
                "acpx_record_id": "t-absent",
                "last_used_at": "2026-11-18T19:01:00Z"
            }),
        );

        let list = list_sessions(tmp.path());
        assert_eq!(list.len(), 4);
        assert_eq!(list[0].tokens, 12345);
        assert_eq!(list[1].tokens, 500);
        assert_eq!(list[2].tokens, 400);
        assert_eq!(list[3].tokens, 0);
    }

    #[test]
    fn test_cost_shapes() {
        let Ok(tmp) = tempfile::tempdir() else {
            panic!("tempdir failed");
        };
        write_json(
            tmp.path(),
            "c1.json",
            &json!({
                "acpx_record_id": "c-num",
                "last_used_at": "2026-11-18T19:05:00Z",
                "cumulative_cost": 0.42
            }),
        );
        write_json(
            tmp.path(),
            "c2.json",
            &json!({
                "acpx_record_id": "c-amount",
                "last_used_at": "2026-11-18T19:04:00Z",
                "cumulative_cost": {"amount": 0.29, "currency": "USD"}
            }),
        );
        write_json(
            tmp.path(),
            "c3.json",
            &json!({
                "acpx_record_id": "c-usd",
                "last_used_at": "2026-11-18T19:03:00Z",
                "cumulative_cost": {"usd": 1.75}
            }),
        );
        write_json(
            tmp.path(),
            "c4.json",
            &json!({
                "acpx_record_id": "c-total",
                "last_used_at": "2026-11-18T19:02:00Z",
                "cumulative_cost": {"total": 3.5}
            }),
        );
        write_json(
            tmp.path(),
            "c5.json",
            &json!({
                "acpx_record_id": "c-none",
                "last_used_at": "2026-11-18T19:01:00Z"
            }),
        );

        let list = list_sessions(tmp.path());
        assert_eq!(list.len(), 5);
        assert_eq!(list[0].cost_usd, Some(0.42));
        assert_eq!(list[1].cost_usd, Some(0.29));
        assert_eq!(list[2].cost_usd, Some(1.75));
        assert_eq!(list[3].cost_usd, Some(3.5));
        assert_eq!(list[4].cost_usd, None);
    }

    #[test]
    fn test_agent_label_variants() {
        assert_eq!(
            agent_label(
                "env -u CLAUDECODE CLAUDE_CODE_EXECUTABLE=/x/claude /Users/me/.local/bin/claude-agent-acp"
            ),
            "claude"
        );
        assert_eq!(agent_label("codex-acp --foo"), "codex");
        assert_eq!(agent_label("/opt/x/grok-pi-acp"), "grok-pi");
        assert_eq!(agent_label("/usr/local/bin/claude-code-acp --bar"), "claude");
        assert_eq!(agent_label(""), "?");
        assert_eq!(agent_label("   "), "?");
        assert_eq!(agent_label("env -u FOO BAR=1"), "?");
    }

    #[test]
    fn test_skips_invalid_and_non_json_files() {
        let Ok(tmp) = tempfile::tempdir() else {
            panic!("tempdir failed");
        };
        write_json(
            tmp.path(),
            "valid.json",
            &json!({
                "acpx_record_id": "valid-1",
                "created_at": "not-a-timestamp",
                "last_used_at": "also-bad"
            }),
        );
        // Non-json file
        let _ = fs::write(
            tmp.path().join("valid-1.stream.ndjson"),
            "{\"acpx_record_id\": \"should-ignore\"}\n",
        );
        // Broken json file
        let _ = fs::write(tmp.path().join("broken.json"), "{not valid json");
        // Missing acpx_record_id
        write_json(
            tmp.path(),
            "no_id.json",
            &json!({
                "name": "missing-id"
            }),
        );
        // Subdirectory named *.json or containing json files
        let subdir = tmp.path().join("nested.json");
        let _ = fs::create_dir_all(&subdir);
        write_json(
            &subdir,
            "inner.json",
            &json!({
                "acpx_record_id": "inner-should-skip"
            }),
        );

        let list = list_sessions(tmp.path());
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].record_id, "valid-1");
        assert_eq!(list[0].created_ms, 0);
        assert_eq!(list[0].last_used_ms, 0);
    }

    #[test]
    fn test_missing_base_directory_returns_empty() {
        let Ok(tmp) = tempfile::tempdir() else {
            panic!("tempdir failed");
        };
        let nonexistent = tmp.path().join("does-not-exist");
        let list = list_sessions(&nonexistent);
        assert!(list.is_empty());
    }
}
```