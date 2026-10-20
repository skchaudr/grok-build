//! Where each harness keeps its session files, and the bounded walk over them.

use std::path::{Path, PathBuf};

pub const MAX_FILES: usize = 50_000;
pub const MAX_DEPTH: usize = 32;
const IGNORED_DIRS: [&str; 15] = [
    ".git", "node_modules", "dist", "build", ".next", "cache", ".cache", ".pnpm-store", ".turbo",
    ".venv", ".repos", "blobs", "Attachments", "attachments", "target",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Jsonl,
    Zstd,
}

impl Format {
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Jsonl => "jsonl",
            Format::Zstd => "zstd",
        }
    }
}

/// Ledger tab order: All, then these.
pub const SOURCES: [&str; 6] = ["dsh", "claude", "pi", "omp", "codex", "grok"];

pub fn source_label(source: &str) -> &str {
    match source {
        "dsh" => "DeepSeek",
        "claude" => "Claude",
        "pi" => "Pi",
        "omp" => "OMP",
        "codex" => "Codex",
        "grok" => "Grok",
        other => other,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Root {
    pub id: String,
    pub source: String,
    pub path: PathBuf,
    pub format: Format,
    /// Only files with exactly this name count (grok's session dirs hold several jsonl files).
    pub file_name: Option<&'static str>,
}

impl Root {
    fn new(id: &str, source: &str, path: PathBuf, format: Format) -> Self {
        Self { id: id.into(), source: source.into(), path, format, file_name: None }
    }
}

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}

pub fn default_roots(home: &Path, grok_home: &Path) -> Vec<Root> {
    let claude = env_dir("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude"));
    let codex = env_dir("CODEX_HOME").unwrap_or_else(|| home.join(".codex"));
    vec![
        Root::new("dsh-sessions", "dsh", home.join(".dsh/sessions"), Format::Zstd),
        Root::new("claude-projects", "claude", claude.join("projects"), Format::Jsonl),
        Root::new("pi-sessions", "pi", home.join(".pi/agent/sessions"), Format::Jsonl),
        Root::new("omp-sessions", "omp", home.join(".omp/agent/sessions"), Format::Jsonl),
        Root::new("codex-archive", "codex", codex.join("archived_sessions"), Format::Jsonl),
        Root::new("codex-sessions", "codex", codex.join("sessions"), Format::Jsonl),
        Root {
            file_name: Some("chat_history.jsonl"),
            ..Root::new("grok-sessions", "grok", grok_home.join("sessions"), Format::Jsonl)
        },
    ]
}

/// Roots for the current user, from `$HOME` and the resolved grok home.
pub fn user_roots() -> Vec<Root> {
    let home = xai_dirs::home_dir().unwrap_or_default();
    default_roots(&home, &xai_dirs::grok_home())
}

#[derive(Debug, Default)]
pub struct Discovery {
    pub files: Vec<PathBuf>,
    pub accessible: bool,
    pub limited: bool,
    pub complete: bool,
}

fn matches(name: &str, root: &Root) -> bool {
    match root.file_name {
        Some(want) => name == want,
        None => {
            let lower = name.to_lowercase();
            lower.ends_with(".jsonl") || lower.ends_with(".jsonl.zst") || lower.ends_with(".jsonl.zstd")
        }
    }
}

pub fn discover(root: &Root, max_files: usize) -> Discovery {
    let mut d = Discovery { complete: true, ..Default::default() };
    let Ok(meta) = std::fs::metadata(&root.path) else {
        d.complete = false;
        return d;
    };
    d.accessible = true;
    if meta.is_file() {
        d.files.push(root.path.clone());
        return d;
    }
    let mut stack = vec![(root.path.clone(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            d.limited = true;
            d.complete = false;
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            d.complete = false;
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if ft.is_dir() {
                if !IGNORED_DIRS.contains(&name.as_ref()) {
                    stack.push((entry.path(), depth + 1));
                }
            } else if ft.is_file() && matches(&name, root) {
                if d.files.len() >= max_files {
                    d.limited = true;
                    d.complete = false;
                    d.files.sort();
                    return d;
                }
                d.files.push(entry.path());
            }
        }
    }
    d.files.sort();
    d
}
