//! `/rename` for external Claude sessions: the adapter has no rename method, so append the same
//! `custom-title` line the Claude SDK writes to the session transcript (last one wins on read).
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

/// One transcript line (no trailing newline), field order as the SDK writes it.
pub(super) fn custom_title_line(session_id: &str, title: &str) -> String {
    #[derive(serde::Serialize)]
    struct Line<'a> {
        r#type: &'static str,
        #[serde(rename = "customTitle")]
        custom_title: &'a str,
        #[serde(rename = "sessionId")]
        session_id: &'a str,
    }
    serde_json::to_string(&Line {
        r#type: "custom-title",
        custom_title: title,
        session_id,
    })
    .expect("serialize custom-title line")
}

/// Blocking; run off the UI thread. Finds `<projects>/*/<session_id>.jsonl` (shared with `claude_rewind`).
pub(super) fn find_transcript(session_id: &str) -> Result<PathBuf, String> {
    // The id becomes a file name: accept only UUID-style characters.
    if session_id.is_empty() || !session_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("invalid session id".to_string());
    }
    let projects = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".claude")))
        .ok_or("cannot locate ~/.claude")?
        .join("projects");
    let file = format!("{session_id}.jsonl");
    std::fs::read_dir(&projects)
        .map_err(|e| format!("cannot read {}: {e}", projects.display()))?
        .flatten()
        .map(|d| d.path().join(&file))
        .find(|p| p.is_file())
        .ok_or_else(|| "session transcript not found (it has no messages yet?)".to_string())
}

/// Blocking; run off the UI thread. Appends the title to the session transcript.
pub(super) fn append_custom_title(session_id: &str, title: &str) -> Result<(), String> {
    let path = find_transcript(session_id)?;
    // Append mode: every write lands at EOF, so the read cursor below cannot truncate or overwrite.
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    // A transcript cut off mid-line would swallow our entry into the broken line: start on a fresh one.
    let mut last = [0u8; 1];
    let unterminated =
        f.seek(SeekFrom::End(-1)).is_ok() && f.read_exact(&mut last).is_ok() && last[0] != b'\n';
    let mut line = custom_title_line(session_id, title);
    if unterminated {
        line.insert(0, '\n');
    }
    writeln!(f, "{line}").map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_is_valid_json_with_sdk_shape() {
        let line = custom_title_line("40e362e8-0d31-4d5a-bd05-916ffa9f77a5", "My \"title\"\n\\ ✓");
        assert!(!line.contains('\n'));
        assert!(line.starts_with(r#"{"type":"custom-title","customTitle":""#));
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["type"], "custom-title");
        assert_eq!(v["customTitle"], "My \"title\"\n\\ ✓");
        assert_eq!(v["sessionId"], "40e362e8-0d31-4d5a-bd05-916ffa9f77a5");
    }

    #[test]
    fn rejects_path_like_session_id() {
        assert!(append_custom_title("../x", "t").is_err());
        assert!(append_custom_title("", "t").is_err());
    }
}
