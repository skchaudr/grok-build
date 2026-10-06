//! Keep external-agent session paths separate from the pager's local process directory.

use std::io;
use std::path::{Path, PathBuf};

fn external_cwd(requested: Option<&Path>, external: bool) -> io::Result<Option<&Path>> {
    let Some(cwd) = requested.filter(|_| external) else {
        return Ok(None);
    };
    if !cwd.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--cwd with --agent-cmd must be an absolute path on the agent host",
        ));
    }
    Ok(Some(cwd))
}

pub(super) fn apply(requested: Option<&Path>, external: bool) -> io::Result<()> {
    if external_cwd(requested, external)?.is_none()
        && let Some(cwd) = requested
    {
        std::env::set_current_dir(cwd)?;
    }
    Ok(())
}

pub(super) fn session_cwd(requested: Option<&Path>, external: bool) -> io::Result<PathBuf> {
    match external_cwd(requested, external)? {
        Some(cwd) => Ok(cwd.to_path_buf()),
        None => std::env::current_dir(),
    }
}

#[cfg(test)]
#[path = "working_directory_tests.rs"]
mod tests;
