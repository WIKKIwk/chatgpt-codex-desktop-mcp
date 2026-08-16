use super::{ProcessInput, assert_process_allowed, run_process};
use crate::config::AccessMode;

#[test]
fn policy_matches_reference_command_boundaries() {
    let git_status = vec!["status".to_owned()];
    assert!(assert_process_allowed("git", &git_status, AccessMode::Coding).is_ok());
    let git_commit = vec!["commit".to_owned()];
    assert!(assert_process_allowed("git", &git_commit, AccessMode::Coding).is_err());
    assert!(assert_process_allowed("rm", &["-rf".to_owned()], AccessMode::Full).is_err());
    assert!(assert_process_allowed("echo", &[], AccessMode::Full).is_ok());
    assert!(assert_process_allowed("node", &["--version".to_owned()], AccessMode::Review).is_ok());
}

#[tokio::test]
async fn foreground_process_returns_status_and_output() {
    let result = run_process(ProcessInput {
        command: "git".to_owned(),
        args: vec!["--version".to_owned()],
        cwd: std::env::current_dir().expect("current directory"),
        timeout_ms: 5_000,
        max_output_bytes: 2_000,
    })
    .await;
    assert_eq!(result.exit_code, Some(0));
    assert!(result.stdout.to_ascii_lowercase().contains("git"));
    assert!(!result.timed_out);
}
