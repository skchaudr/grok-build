use super::*;
use std::process::Command;

fn isolated(test: impl FnOnce(&Path)) {
    let thread = std::thread::current();
    let name = thread.name().expect("test name");
    if std::env::var("GROK_CWD_TEST_CHILD").as_deref() == Ok(name) {
        test(&std::env::current_dir().unwrap());
        return;
    }
    let root = std::env::temp_dir().join(format!(
        "grok-cwd-{}-{}",
        std::process::id(),
        name.replace(':', "_")
    ));
    std::fs::create_dir(&root).unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("GROK_CWD_TEST_CHILD", name)
        .current_dir(&root)
        .status()
        .unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    assert!(status.success(), "isolated cwd test failed: {name}");
}

#[test]
#[cfg(unix)]
fn external_symlink_keeps_lexical_spelling_without_chdir() {
    isolated(|launch| {
        let real = launch.join("real");
        let alias = launch.join("alias");
        std::fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        apply(Some(&alias), true).unwrap();
        assert_eq!(session_cwd(Some(&alias), true).unwrap(), alias);
        assert_eq!(std::env::current_dir().unwrap(), launch);
    });
}

#[test]
fn external_missing_absolute_directory_is_not_checked_locally() {
    isolated(|launch| {
        let remote = launch.join("remote-only");
        assert!(!remote.exists());
        apply(Some(&remote), true).unwrap();
        assert_eq!(session_cwd(Some(&remote), true).unwrap(), remote);
        assert_eq!(std::env::current_dir().unwrap(), launch);
        assert!(!remote.exists());
    });
}

#[test]
fn external_relative_directory_is_rejected_without_chdir() {
    isolated(|launch| {
        std::fs::create_dir("relative").unwrap();
        let error = apply(Some(Path::new("relative")), true).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("absolute"));
        assert_eq!(std::env::current_dir().unwrap(), launch);
    });
}

#[test]
#[cfg(unix)]
fn native_symlink_still_changes_process_cwd_and_resolves_target() {
    isolated(|launch| {
        let real = launch.join("real");
        let alias = launch.join("alias");
        std::fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        apply(Some(&alias), false).unwrap();
        assert_eq!(session_cwd(Some(&alias), false).unwrap(), real);
        assert_eq!(std::env::current_dir().unwrap(), real);
    });
}

#[test]
fn native_missing_directory_still_fails() {
    isolated(|launch| {
        let missing = launch.join("missing");
        assert!(apply(Some(&missing), false).is_err());
        assert_eq!(std::env::current_dir().unwrap(), launch);
    });
}

#[test]
fn native_relative_directory_still_changes_process_cwd() {
    isolated(|launch| {
        let relative = Path::new("relative");
        std::fs::create_dir(relative).unwrap();
        apply(Some(relative), false).unwrap();
        assert_eq!(
            session_cwd(Some(relative), false).unwrap(),
            launch.join(relative)
        );
    });
}

#[test]
fn no_explicit_cwd_still_uses_process_directory() {
    isolated(|launch| {
        for external in [false, true] {
            apply(None, external).unwrap();
            assert_eq!(session_cwd(None, external).unwrap(), launch);
        }
    });
}
