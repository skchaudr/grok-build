//! grok-pi user state home (`$GROK_HOME`, default `~/.grok-pi`).
//!
//! Stock Grok CLI uses `~/.grok` (user) and `<repo>/.grok` (project).
//! grok-pi must not share those trees: user home → `~/.grok-pi`, project →
//! `<repo>/.grok-pi` via `$GROK_PROJECT_DIR`. Call
//! [`ensure_default_grok_home`] before any code path that reads
//! `xai_grok_config::grok_home()` / `project_config_dirname()` (OnceLocks).

use std::path::{Path, PathBuf};

/// Directory name under `$HOME` when `GROK_HOME` is unset.
pub(super) const DEFAULT_GROK_PI_DIRNAME: &str = ".grok-pi";

/// Stock Grok CLI home directory name (migration source default).
pub(super) const LEGACY_GROK_DIRNAME: &str = ".grok";

/// Marker written after a successful migrate so auto-migrate runs once.
pub(super) const MIGRATE_MARKER: &str = ".migrated-from-legacy";

/// Canonical default home for grok-pi: `~/.grok-pi`.
pub(super) fn default_grok_pi_home() -> PathBuf {
    #[allow(deprecated)]
    let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("."));
    dunce::canonicalize(&home)
        .unwrap_or(home)
        .join(DEFAULT_GROK_PI_DIRNAME)
}

/// Stock Grok home used as migration source: `$GROK_LEGACY_HOME` or `~/.grok`.
pub(super) fn legacy_grok_home() -> PathBuf {
    if let Ok(v) = std::env::var("GROK_LEGACY_HOME") {
        return PathBuf::from(v);
    }
    #[allow(deprecated)]
    let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("."));
    dunce::canonicalize(&home)
        .unwrap_or(home)
        .join(LEGACY_GROK_DIRNAME)
}

/// If `GROK_HOME` is unset, point it at `~/.grok-pi` and ensure the directory exists.
///
/// No-op when the user (or tests) already set `GROK_HOME`. Must run at the
/// very start of `main` so `grok_home()`'s `OnceLock` sees the right value.
pub(super) fn ensure_default_grok_home() {
    // Project-local tree isolation (workflows/hooks/config under repo root).
    // Must run before any `project_config_dirname()` OnceLock init.
    if std::env::var_os("GROK_PROJECT_DIR").is_none() {
        // SAFETY: single-threaded startup before other threads read env.
        unsafe {
            std::env::set_var("GROK_PROJECT_DIR", ".grok-pi");
        }
    }
    if std::env::var_os("GROK_HOME").is_some() {
        return;
    }
    let path = default_grok_pi_home();
    let _ = std::fs::create_dir_all(&path);
    // SAFETY: single-threaded startup before any other thread reads env.
    unsafe {
        std::env::set_var("GROK_HOME", &path);
    }
}

/// Resolved effective home (env or default). Does not create directories.
pub(super) fn effective_grok_home() -> PathBuf {
    if let Ok(v) = std::env::var("GROK_HOME") {
        return PathBuf::from(v);
    }
    default_grok_pi_home()
}

/// Short display form for help/error strings (`~/.grok-pi` or `$GROK_HOME`).
pub(super) fn display_home(path: &Path) -> String {
    if let Ok(home) = std::env::var("HOME") {
        let home_path = Path::new(&home);
        if let Ok(rest) = path.strip_prefix(home_path) {
            return format!("~/{}", rest.display());
        }
    }
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_home_ends_with_grok_pi() {
        let p = default_grok_pi_home();
        assert!(p.ends_with(DEFAULT_GROK_PI_DIRNAME), "got {}", p.display());
    }

    #[test]
    fn legacy_home_ends_with_grok() {
        let p = legacy_grok_home();
        // When GROK_LEGACY_HOME is unset in this process.
        if std::env::var_os("GROK_LEGACY_HOME").is_none() {
            assert!(p.ends_with(LEGACY_GROK_DIRNAME), "got {}", p.display());
        }
    }
}
