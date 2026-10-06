use super::*;
use crate::app::PagerArgs;
use crate::app::effects::{SessionFlags, execute};
use crate::app::session_startup::{
    MaterializeCtx, MaterializedStartup, materialize_startup_for_cwd,
};
use clap::Parser;
use std::path::Path;

async fn assert_external_session_requests(cwd: &Path) {
    for mode in ["--leader", "--no-leader"] {
        for resume in [false, true] {
            let mut argv = vec![
                "grok",
                "--agent-cmd",
                "external-agent",
                mode,
                "--cwd",
                cwd.to_str().unwrap(),
            ];
            if resume {
                argv.extend(["--resume", "remote-session"]);
            }
            let args = PagerArgs::try_parse_from(argv)
                .unwrap()
                .apply_cwd()
                .unwrap();
            let mut app = test_app();
            app.cwd = args.session_cwd().unwrap();
            app.leader_mode = args.leader;
            let materialized = materialize_startup_for_cwd(
                MaterializeCtx::from_pager_args(&args),
                args.session_startup_intent().unwrap(),
                &app.cwd.to_string_lossy(),
            )
            .await
            .unwrap();
            let effects = match materialized {
                MaterializedStartup::NewAuto => dispatch_new_session_inner(&mut app, None),
                MaterializedStartup::Resume {
                    session_id,
                    original_cwd,
                    ..
                } => dispatch(
                    Action::LoadSession(session_id, original_cwd, false),
                    &mut app,
                ),
                other => panic!("unexpected startup: {other:?}"),
            };
            assert_eq!(app.cwd, cwd);
            assert_eq!(app.agents.last().unwrap().1.session.cwd, cwd);
            let effect = effects
                .into_iter()
                .find(|effect| {
                    matches!(
                        effect,
                        Effect::CreateSession { .. } | Effect::LoadSession { .. }
                    )
                })
                .expect("session effect");
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let (progress_tx, _progress_rx) = tokio::sync::mpsc::unbounded_channel();
            let mut tasks = tokio::task::JoinSet::new();
            execute(
                effect,
                &mut tasks,
                &tx,
                &app.cwd,
                &SessionFlags::default(),
                &progress_tx,
            );
            let message = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
                .await
                .expect("session request timeout")
                .expect("session request");
            match message {
                xai_acp_lib::AcpAgentMessage::NewSession(request) => {
                    assert!(!resume);
                    assert_eq!(request.cwd, cwd);
                }
                xai_acp_lib::AcpAgentMessage::LoadSession(request) => {
                    assert!(resume);
                    assert_eq!(request.cwd, cwd);
                    assert_eq!(request.session_id, acp::SessionId::new("remote-session"));
                }
                other => panic!("unexpected request: {other:?}"),
            }
            tasks.shutdown().await;
        }
    }
}

#[tokio::test]
async fn external_session_new_and_load_accept_remote_only_cwd() {
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("remote-only");
    assert!(!remote.exists());
    assert_external_session_requests(&remote).await;
    assert!(!remote.exists());
}

#[tokio::test]
#[cfg(unix)]
async fn external_session_new_and_load_preserve_symlink_cwd() {
    let temp = tempfile::tempdir().unwrap();
    let real = temp.path().join("real");
    let alias = temp.path().join("alias");
    std::fs::create_dir(&real).unwrap();
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    assert_external_session_requests(&alias).await;
}
