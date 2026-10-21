//! Open the selected block's file (Read/Edit tool calls) in an editor at the relevant line.
//!
//! Editor: `$GROK_OPEN_EDITOR`, then `$VISUAL`, then `$EDITOR`, then `zed`.
//! GUI editors (zed, code, cursor, …) are spawned detached. Terminal editors open in a new Herdr pane when running
//! inside Herdr. Outside Herdr, a terminal editor falls back to the in-place `$VISUAL`/`$EDITOR` flow with no line
//! jump; `$GROK_OPEN_EDITOR` is not used on that path.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::scrollback::block::RenderBlock;
use crate::scrollback::blocks::ToolCallBlock;

const DEFAULT_EDITOR: &str = "zed";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EditorTarget {
    pub path: PathBuf,
    /// 1-based line to jump to.
    pub line: Option<usize>,
}

/// File and line for a block, if it names a file. Relative paths resolve against `cwd`.
pub(crate) fn target_for_block(block: &RenderBlock, cwd: &Path) -> Option<EditorTarget> {
    let (path, line) = match block {
        RenderBlock::ToolCall(ToolCallBlock::Read(b)) => {
            (b.path.as_str(), b.line_range.map(|r| r.start.max(1)))
        }
        RenderBlock::ToolCall(ToolCallBlock::Edit(b)) => (
            b.path.as_str(),
            b.hunks
                .iter()
                .flatten()
                .find(|l| l.tag != similar::ChangeTag::Equal)
                .map(|l| l.ln.max(1)),
        ),
        _ => return None,
    };
    if path.is_empty() {
        return None;
    }
    // Block `path` can be a tool-title fallback rather than a real file; only open existing files.
    let full = match path.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME")
            .map_or_else(|| cwd.join(path), |home| PathBuf::from(home).join(rest)),
        None => cwd.join(path),
    };
    if !full.is_file() {
        return None;
    }
    Some(EditorTarget { path: full, line })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditorKind {
    /// `zed path:line`, `subl path:line`
    GuiColon,
    /// `code -g path:line`
    GuiGoto,
    /// `idea --line N path`
    GuiIdea,
    /// `open path`
    GuiOpen,
    /// `hx path:line`, `micro path:line`
    TerminalColon,
    /// `nvim +N path` (vim, vi, nano, emacs, kak, …)
    TerminalPlus,
}

impl EditorKind {
    pub(crate) fn is_gui(self) -> bool {
        matches!(
            self,
            Self::GuiColon | Self::GuiGoto | Self::GuiIdea | Self::GuiOpen
        )
    }
}

pub(crate) fn classify(program: &str) -> EditorKind {
    let name = Path::new(program)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(program);
    match name {
        "zed" | "zed-preview" | "zeditor" | "subl" => EditorKind::GuiColon,
        "code" | "code-insiders" | "cursor" | "windsurf" | "codium" => EditorKind::GuiGoto,
        "idea" | "rustrover" | "pycharm" | "webstorm" | "goland" => EditorKind::GuiIdea,
        "open" => EditorKind::GuiOpen,
        "hx" | "helix" | "micro" => EditorKind::TerminalColon,
        _ => EditorKind::TerminalPlus,
    }
}

/// Editor command words from the environment (first non-empty of the chain), else [`DEFAULT_EDITOR`].
pub(crate) fn resolve_editor(
    grok_open_editor: Option<&str>,
    visual: Option<&str>,
    editor: Option<&str>,
) -> Option<Vec<String>> {
    let command = [grok_open_editor, visual, editor]
        .into_iter()
        .flatten()
        .find(|v| !v.trim().is_empty())
        .unwrap_or(DEFAULT_EDITOR);
    shlex::split(command).filter(|parts| parts.first().is_some_and(|p| !p.is_empty()))
}

fn editor_from_env() -> Option<Vec<String>> {
    let get = |k: &str| std::env::var(k).ok();
    resolve_editor(
        get("GROK_OPEN_EDITOR").as_deref(),
        get("VISUAL").as_deref(),
        get("EDITOR").as_deref(),
    )
}

/// Full argv for opening `target` with `editor`.
pub(crate) fn build_argv(editor: &[String], target: &EditorTarget) -> Vec<String> {
    let mut argv = editor.to_vec();
    let path = target.path.display().to_string();
    let kind = editor
        .first()
        .map_or(EditorKind::TerminalPlus, |p| classify(p));
    match (kind, target.line) {
        (EditorKind::GuiColon | EditorKind::TerminalColon, Some(line)) => {
            argv.push(format!("{path}:{line}"));
        }
        (EditorKind::GuiGoto, Some(line)) => {
            argv.push("-g".to_owned());
            argv.push(format!("{path}:{line}"));
        }
        (EditorKind::GuiIdea, Some(line)) => {
            argv.push("--line".to_owned());
            argv.push(line.to_string());
            argv.push(path);
        }
        (EditorKind::TerminalPlus, Some(line)) => {
            argv.push(format!("+{line}"));
            argv.push(path);
        }
        (_, _) => argv.push(path),
    }
    argv
}

fn in_herdr() -> bool {
    std::env::var_os("HERDR_PANE_ID").is_some()
}

/// What the caller should do after [`open`].
#[derive(Debug)]
pub(crate) enum OpenOutcome {
    /// Launched; show this toast.
    Launched(String),
    /// Terminal editor outside Herdr: suspend the TUI and run `$EDITOR` on this path.
    SuspendFallback(PathBuf),
    /// Could not launch; show this toast.
    Failed(String),
}

pub(crate) fn open(target: &EditorTarget, cwd: &Path) -> OpenOutcome {
    let Some(editor) = editor_from_env() else {
        return OpenOutcome::Failed(
            "could not parse $GROK_OPEN_EDITOR / $VISUAL / $EDITOR".to_owned(),
        );
    };
    let Some(program) = editor.first().cloned() else {
        return OpenOutcome::Failed("empty editor command".to_owned());
    };
    let argv = build_argv(&editor, target);
    let label = describe(target, &program);
    if classify(&program).is_gui() {
        return match spawn_detached(&argv) {
            Ok(()) => OpenOutcome::Launched(format!("opened {label}")),
            Err(e) => OpenOutcome::Failed(format!("{program}: {e}")),
        };
    }
    if in_herdr() {
        let cwd = cwd.to_path_buf();
        // Two herdr round-trips; keep them off the UI thread.
        std::thread::spawn(move || {
            if let Err(error) = open_in_herdr_pane(&argv, &cwd) {
                tracing::warn!(%error, "open in editor: herdr pane launch failed");
            }
        });
        return OpenOutcome::Launched(format!("opening {label} in a Herdr pane"));
    }
    OpenOutcome::SuspendFallback(target.path.clone())
}

fn describe(target: &EditorTarget, program: &str) -> String {
    let name = target.path.file_name().map_or_else(
        || target.path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let editor = Path::new(program)
        .file_name()
        .map_or(program.into(), |n| n.to_string_lossy());
    match target.line {
        Some(line) => format!("{name}:{line} in {editor}"),
        None => format!("{name} in {editor}"),
    }
}

#[allow(clippy::disallowed_methods)] // GUI editor deliberately outlives the TUI; reaped on a thread
fn spawn_detached(argv: &[String]) -> std::io::Result<()> {
    let (program, args) = argv.split_first().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "empty editor command")
    })?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn open_in_herdr_pane(argv: &[String], cwd: &Path) -> anyhow::Result<()> {
    let herdr = std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".to_owned());
    let split = Command::new(&herdr)
        .args([
            "pane",
            "split",
            "--current",
            "--direction",
            "right",
            "--focus",
            "--cwd",
        ])
        .arg(cwd)
        .stdin(Stdio::null())
        .output()?;
    anyhow::ensure!(
        split.status.success(),
        "herdr pane split: {}",
        String::from_utf8_lossy(&split.stderr).trim()
    );
    let pane = herdr_split_pane_id(&split.stdout)?;
    let command = shlex::try_join(argv.iter().map(String::as_str))?;
    let run = Command::new(&herdr)
        .args(["pane", "run", pane.as_str(), command.as_str()])
        .stdin(Stdio::null())
        .output()?;
    anyhow::ensure!(
        run.status.success(),
        "herdr pane run: {}",
        String::from_utf8_lossy(&run.stderr).trim()
    );
    Ok(())
}

/// `herdr pane split` prints the new pane as `.result.pane.pane_id`.
fn herdr_split_pane_id(stdout: &[u8]) -> anyhow::Result<String> {
    let value: serde_json::Value = serde_json::from_slice(stdout)?;
    value
        .pointer("/result/pane/pane_id")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("herdr pane split: no .result.pane.pane_id"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(line: Option<usize>) -> EditorTarget {
        EditorTarget {
            path: PathBuf::from("/repo/src/main.rs"),
            line,
        }
    }

    fn words(s: &[&str]) -> Vec<String> {
        s.iter().map(|w| (*w).to_owned()).collect()
    }

    #[test]
    fn resolve_editor_prefers_grok_then_visual_then_editor_then_zed() {
        assert_eq!(
            resolve_editor(Some("nvim"), Some("code -w"), Some("vi")),
            Some(words(&["nvim"]))
        );
        assert_eq!(
            resolve_editor(Some(" "), Some("code -w"), Some("vi")),
            Some(words(&["code", "-w"]))
        );
        assert_eq!(resolve_editor(None, None, Some("vi")), Some(words(&["vi"])));
        assert_eq!(resolve_editor(None, None, None), Some(words(&["zed"])));
        assert_eq!(resolve_editor(Some("\"unterminated"), None, None), None);
    }

    #[test]
    fn build_argv_per_editor_family() {
        let t = target(Some(42));
        assert_eq!(
            build_argv(&words(&["zed"]), &t),
            words(&["zed", "/repo/src/main.rs:42"])
        );
        assert_eq!(
            build_argv(&words(&["/usr/local/bin/cursor"]), &t),
            words(&["/usr/local/bin/cursor", "-g", "/repo/src/main.rs:42"])
        );
        assert_eq!(
            build_argv(&words(&["nvim"]), &t),
            words(&["nvim", "+42", "/repo/src/main.rs"])
        );
        assert_eq!(
            build_argv(&words(&["hx"]), &t),
            words(&["hx", "/repo/src/main.rs:42"])
        );
        assert_eq!(
            build_argv(&words(&["idea"]), &t),
            words(&["idea", "--line", "42", "/repo/src/main.rs"])
        );
        assert_eq!(
            build_argv(&words(&["nvim"]), &target(None)),
            words(&["nvim", "/repo/src/main.rs"])
        );
    }

    #[test]
    fn classify_separates_gui_and_terminal() {
        assert!(classify("zed").is_gui());
        assert!(classify("/Applications/Zed.app/Contents/MacOS/cli/zed").is_gui());
        assert!(classify("code").is_gui());
        assert!(!classify("nvim").is_gui());
        assert!(!classify("vi").is_gui());
    }

    #[test]
    fn herdr_split_pane_id_reads_result_pane() {
        let out = br#"{"result":{"pane":{"pane_id":"w36:p9","tab":"w36:t5"}}}"#;
        assert_eq!(herdr_split_pane_id(out).unwrap(), "w36:p9");
        assert!(herdr_split_pane_id(br#"{"result":{}}"#).is_err());
    }

    #[test]
    fn describe_names_file_line_and_editor() {
        assert_eq!(
            describe(&target(Some(7)), "/opt/bin/nvim"),
            "main.rs:7 in nvim"
        );
        assert_eq!(describe(&target(None), "zed"), "main.rs in zed");
    }
}
