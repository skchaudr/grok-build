//! Migrate stock Grok user state (`~/.grok`) into grok-pi home (`~/.grok-pi`).
//!
//! Copies only pager-relevant allowlisted entries. Pi sessions stay under
//! `~/.pi` and are never part of this migration. Stock install trees
//! (`bin/`, `downloads/`, `marketplace-cache/`, …) are intentionally skipped.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::home::{MIGRATE_MARKER, display_home, effective_grok_home, legacy_grok_home};

/// Files and directories under legacy home that are useful for grok-pi.
///
/// Keep this list product-focused: UI prefs, policy, skills/hooks, not
/// Grok cloud install caches or Grok-native session trees.
const DEFAULT_ENTRIES: &[&str] = &[
    "pager.toml",
    "config.toml",
    "trusted_folders.toml",
    "slash-mru.json",
    "tip_cursor.json",
    "skills",
    "hooks",
    "projects",
];

/// Optional Grok cloud auth (off by default — Pi uses its own auth store).
const AUTH_ENTRY: &str = "auth.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    Copied,
    SkippedExists,
    SkippedMissing,
    WouldCopy,
    Overwritten,
}

#[derive(Debug, Clone)]
pub(super) struct EntryResult {
    pub name: String,
    pub action: Action,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub(super) struct MigrateReport {
    pub from: PathBuf,
    pub to: PathBuf,
    pub dry_run: bool,
    pub force: bool,
    pub entries: Vec<EntryResult>,
}

impl MigrateReport {
    pub fn copied_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| {
                matches!(
                    e.action,
                    Action::Copied | Action::Overwritten | Action::WouldCopy
                )
            })
            .count()
    }

    pub fn skipped_exists_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.action == Action::SkippedExists)
            .count()
    }

    pub fn format_human(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "migrate-home: {} → {}\n",
            display_home(&self.from),
            display_home(&self.to)
        ));
        if self.dry_run {
            out.push_str("mode: dry-run (no files written)\n");
        } else if self.force {
            out.push_str("mode: force (overwrite existing)\n");
        } else {
            out.push_str("mode: safe (skip existing)\n");
        }
        for e in &self.entries {
            let tag = match e.action {
                Action::Copied => "copied",
                Action::Overwritten => "overwritten",
                Action::SkippedExists => "skip-exists",
                Action::SkippedMissing => "skip-missing",
                Action::WouldCopy => "would-copy",
            };
            out.push_str(&format!("  [{tag}] {}{}\n", e.name, e.detail));
        }
        out.push_str(&format!(
            "summary: {} transferable, {} already present\n",
            self.copied_count(),
            self.skipped_exists_count()
        ));
        out
    }
}

#[derive(Debug, Clone)]
pub(super) struct MigrateOptions {
    pub from: PathBuf,
    pub to: PathBuf,
    pub dry_run: bool,
    pub force: bool,
    pub include_auth: bool,
    /// Write/read the once-marker under `to`.
    pub write_marker: bool,
}

impl Default for MigrateOptions {
    fn default() -> Self {
        Self {
            from: legacy_grok_home(),
            to: effective_grok_home(),
            dry_run: false,
            force: false,
            include_auth: false,
            write_marker: true,
        }
    }
}

fn entry_list(include_auth: bool) -> Vec<&'static str> {
    let mut v = DEFAULT_ENTRIES.to_vec();
    if include_auth {
        v.push(AUTH_ENTRY);
    }
    v
}

/// Run the migration. Creates `to` if needed. Never deletes source files.
pub(super) fn migrate(opts: &MigrateOptions) -> io::Result<MigrateReport> {
    if !opts.from.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "legacy home not found: {} (nothing to migrate)",
                display_home(&opts.from)
            ),
        ));
    }
    if same_path(&opts.from, &opts.to) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "source and destination are the same path; refusing to migrate",
        ));
    }

    if !opts.dry_run {
        fs::create_dir_all(&opts.to)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&opts.to, fs::Permissions::from_mode(0o700));
        }
    }

    let mut entries = Vec::new();
    for name in entry_list(opts.include_auth) {
        let src = opts.from.join(name);
        let dst = opts.to.join(name);
        if !src.exists() {
            entries.push(EntryResult {
                name: name.to_string(),
                action: Action::SkippedMissing,
                detail: String::new(),
            });
            continue;
        }

        if dst.exists() && !opts.force {
            entries.push(EntryResult {
                name: name.to_string(),
                action: Action::SkippedExists,
                detail: String::new(),
            });
            continue;
        }

        if opts.dry_run {
            entries.push(EntryResult {
                name: name.to_string(),
                action: Action::WouldCopy,
                detail: String::new(),
            });
            continue;
        }

        let existed = dst.exists();
        copy_entry(&src, &dst, opts.force)?;
        entries.push(EntryResult {
            name: name.to_string(),
            action: if existed {
                Action::Overwritten
            } else {
                Action::Copied
            },
            detail: String::new(),
        });
    }

    if opts.write_marker && !opts.dry_run {
        write_marker(&opts.to, &opts.from, &entries)?;
    }

    Ok(MigrateReport {
        from: opts.from.clone(),
        to: opts.to.clone(),
        dry_run: opts.dry_run,
        force: opts.force,
        entries,
    })
}

/// Status-only scan (always dry-run semantics, no marker write).
pub(super) fn status(from: &Path, to: &Path, include_auth: bool) -> MigrateReport {
    let mut entries = Vec::new();
    for name in entry_list(include_auth) {
        let src = from.join(name);
        let dst = to.join(name);
        if !src.exists() {
            entries.push(EntryResult {
                name: name.to_string(),
                action: Action::SkippedMissing,
                detail: String::new(),
            });
        } else if dst.exists() {
            entries.push(EntryResult {
                name: name.to_string(),
                action: Action::SkippedExists,
                detail: String::new(),
            });
        } else {
            entries.push(EntryResult {
                name: name.to_string(),
                action: Action::WouldCopy,
                detail: String::new(),
            });
        }
    }
    MigrateReport {
        from: from.to_path_buf(),
        to: to.to_path_buf(),
        dry_run: true,
        force: false,
        entries,
    }
}

/// Auto-migrate once when dest is a fresh grok-pi home and legacy has data.
///
/// Safe defaults: skip existing, no auth, write marker. Returns `None` when
/// there is nothing useful to do (already migrated, no source, dest already
/// populated with allowlisted files).
pub(super) fn maybe_auto_migrate() -> io::Result<Option<MigrateReport>> {
    let from = legacy_grok_home();
    let to = effective_grok_home();

    if !from.is_dir() || same_path(&from, &to) {
        return Ok(None);
    }
    if to.join(MIGRATE_MARKER).exists() {
        return Ok(None);
    }
    // If dest already has any allowlisted entry, assume user set it up — only
    // write marker so we stop probing.
    let already = entry_list(false)
        .into_iter()
        .any(|name| to.join(name).exists());
    if already {
        let _ = write_marker(
            &to,
            &from,
            &[EntryResult {
                name: "(auto-skip)".into(),
                action: Action::SkippedExists,
                detail: "destination already has grok-pi state".into(),
            }],
        );
        return Ok(None);
    }

    let has_source = entry_list(false)
        .into_iter()
        .any(|name| from.join(name).exists());
    if !has_source {
        return Ok(None);
    }

    let report = migrate(&MigrateOptions {
        from,
        to,
        dry_run: false,
        force: false,
        include_auth: false,
        write_marker: true,
    })?;
    Ok(Some(report))
}

fn write_marker(to: &Path, from: &Path, entries: &[EntryResult]) -> io::Result<()> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut body = format!(
        "migrated_at_unix={ts}\nfrom={}\nto={}\n",
        from.display(),
        to.display()
    );
    for e in entries {
        body.push_str(&format!("{:?}\t{}\t{}\n", e.action, e.name, e.detail));
    }
    let path = to.join(MIGRATE_MARKER);
    fs::write(path, body)
}

fn copy_entry(src: &Path, dst: &Path, force: bool) -> io::Result<()> {
    let meta = fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        // Refuse to copy symlinks into the home tree (avoid escape / surprise).
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("refusing to migrate symlink: {}", src.display()),
        ));
    }
    if meta.is_dir() {
        copy_dir_merge(src, dst, force)
    } else {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, dst)?;
        Ok(())
    }
}

fn copy_dir_merge(src: &Path, dst: &Path, force: bool) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_merge(&from, &to, force)?;
        } else if ty.is_symlink() {
            // Skip symlinks inside trees rather than failing the whole migrate.
            continue;
        } else if to.exists() && !force {
            continue;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn same_path(a: &Path, b: &Path) -> bool {
    let ca = dunce::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let cb = dunce::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    ca == cb
}

/// CLI entry: print report, return process-style error for main.
pub(super) fn run_cli(
    from: Option<PathBuf>,
    to: Option<PathBuf>,
    dry_run: bool,
    force: bool,
    include_auth: bool,
    status_only: bool,
) -> anyhow::Result<()> {
    let opts = MigrateOptions {
        from: from.unwrap_or_else(legacy_grok_home),
        to: to.unwrap_or_else(effective_grok_home),
        dry_run: dry_run || status_only,
        force: force && !status_only,
        include_auth,
        write_marker: !status_only && !dry_run,
    };

    if status_only {
        let report = status(&opts.from, &opts.to, opts.include_auth);
        print!("{}", report.format_human());
        let marker = opts.to.join(MIGRATE_MARKER);
        if marker.exists() {
            println!("marker: present ({})", display_home(&marker));
        } else {
            println!("marker: absent");
        }
        return Ok(());
    }

    let report = migrate(&opts).map_err(|e| anyhow::anyhow!("{e}"))?;
    print!("{}", report.format_human());
    if !opts.dry_run {
        println!("marker: {}", display_home(&opts.to.join(MIGRATE_MARKER)));
        println!("note: Pi sessions remain under ~/.pi (not part of this migrate).");
        println!(
            "note: source {} was left intact (copy, not move).",
            display_home(&opts.from)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(path: &Path, body: &str) {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p).unwrap();
        }
        fs::write(path, body).unwrap();
    }

    #[test]
    fn copies_allowlisted_files_and_skips_unknown() {
        let tmp = TempDir::new().unwrap();
        let from = tmp.path().join("legacy");
        let to = tmp.path().join("pi");
        write(&from.join("pager.toml"), "x=1\n");
        write(&from.join("config.toml"), "[ui]\n");
        write(&from.join("skills/foo/SKILL.md"), "hi\n");
        write(&from.join("downloads/big.bin"), "nope\n");
        write(&from.join("auth.json"), "{}\n");

        let report = migrate(&MigrateOptions {
            from: from.clone(),
            to: to.clone(),
            dry_run: false,
            force: false,
            include_auth: false,
            write_marker: true,
        })
        .unwrap();

        assert!(to.join("pager.toml").exists());
        assert!(to.join("config.toml").exists());
        assert!(to.join("skills/foo/SKILL.md").exists());
        assert!(!to.join("downloads/big.bin").exists());
        assert!(!to.join("auth.json").exists());
        assert!(to.join(MIGRATE_MARKER).exists());
        assert!(report.copied_count() >= 3);
    }

    #[test]
    fn skip_existing_unless_force() {
        let tmp = TempDir::new().unwrap();
        let from = tmp.path().join("legacy");
        let to = tmp.path().join("pi");
        write(&from.join("pager.toml"), "from\n");
        write(&to.join("pager.toml"), "keep\n");

        let r1 = migrate(&MigrateOptions {
            from: from.clone(),
            to: to.clone(),
            dry_run: false,
            force: false,
            include_auth: false,
            write_marker: false,
        })
        .unwrap();
        assert_eq!(fs::read_to_string(to.join("pager.toml")).unwrap(), "keep\n");
        assert!(
            r1.entries
                .iter()
                .any(|e| e.name == "pager.toml" && e.action == Action::SkippedExists)
        );

        let r2 = migrate(&MigrateOptions {
            from,
            to: to.clone(),
            dry_run: false,
            force: true,
            include_auth: false,
            write_marker: false,
        })
        .unwrap();
        assert_eq!(fs::read_to_string(to.join("pager.toml")).unwrap(), "from\n");
        assert!(
            r2.entries
                .iter()
                .any(|e| e.name == "pager.toml" && e.action == Action::Overwritten)
        );
    }

    #[test]
    fn dry_run_writes_nothing() {
        let tmp = TempDir::new().unwrap();
        let from = tmp.path().join("legacy");
        let to = tmp.path().join("pi");
        write(&from.join("pager.toml"), "x\n");

        let report = migrate(&MigrateOptions {
            from,
            to: to.clone(),
            dry_run: true,
            force: false,
            include_auth: false,
            write_marker: true,
        })
        .unwrap();
        assert!(!to.exists() || !to.join("pager.toml").exists());
        assert!(!to.join(MIGRATE_MARKER).exists());
        assert!(report.entries.iter().any(|e| e.action == Action::WouldCopy));
    }

    #[test]
    fn refuses_same_path() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("same");
        fs::create_dir_all(&home).unwrap();
        write(&home.join("pager.toml"), "x\n");
        let err = migrate(&MigrateOptions {
            from: home.clone(),
            to: home,
            dry_run: false,
            force: false,
            include_auth: false,
            write_marker: false,
        })
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn include_auth_copies_auth_json() {
        let tmp = TempDir::new().unwrap();
        let from = tmp.path().join("legacy");
        let to = tmp.path().join("pi");
        write(&from.join("auth.json"), "{\"t\":1}\n");

        migrate(&MigrateOptions {
            from,
            to: to.clone(),
            dry_run: false,
            force: false,
            include_auth: true,
            write_marker: false,
        })
        .unwrap();
        assert_eq!(
            fs::read_to_string(to.join("auth.json")).unwrap(),
            "{\"t\":1}\n"
        );
    }

    #[test]
    fn auto_migrate_once() {
        let tmp = TempDir::new().unwrap();
        let from = tmp.path().join("legacy");
        let to = tmp.path().join("pi");
        write(&from.join("pager.toml"), "auto\n");

        // Simulate env isolation without mutating process GROK_HOME once-lock:
        // call migrate path used by maybe_auto_migrate with explicit opts.
        let report = migrate(&MigrateOptions {
            from: from.clone(),
            to: to.clone(),
            dry_run: false,
            force: false,
            include_auth: false,
            write_marker: true,
        })
        .unwrap();
        assert!(to.join("pager.toml").exists());
        assert!(to.join(MIGRATE_MARKER).exists());
        assert!(report.copied_count() >= 1);

        // Second run with skip: existing file skipped.
        let report2 = migrate(&MigrateOptions {
            from,
            to: to.clone(),
            dry_run: false,
            force: false,
            include_auth: false,
            write_marker: true,
        })
        .unwrap();
        assert!(report2.skipped_exists_count() >= 1);
    }
}
