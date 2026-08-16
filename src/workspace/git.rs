use std::path::Path;

use crate::process::{ProcessInput, ProcessResult, cap_text, run_process};

#[derive(Debug, Clone)]
pub struct GitDiffOptions {
    pub staged: bool,
    pub path: Option<String>,
    pub stat_only: bool,
    pub max_bytes: usize,
}

pub async fn git_status(root: &Path, max_bytes: usize) -> ProcessResult {
    run_process(ProcessInput {
        command: "git".to_owned(),
        args: vec!["status".to_owned(), "--short".to_owned()],
        cwd: root.to_path_buf(),
        timeout_ms: 30_000,
        max_output_bytes: max_bytes,
    })
    .await
}

pub async fn git_diff(root: &Path, options: GitDiffOptions) -> ProcessResult {
    let mut base_args = vec!["diff".to_owned()];
    if options.staged {
        base_args.push("--cached".to_owned());
    }
    let path_args = options
        .path
        .as_ref()
        .map(|path| vec!["--".to_owned(), path.clone()])
        .unwrap_or_default();

    let mut stat_args = base_args.clone();
    stat_args.push("--stat".to_owned());
    stat_args.extend(path_args.clone());
    let stat_result = run_process(ProcessInput {
        command: "git".to_owned(),
        args: stat_args,
        cwd: root.to_path_buf(),
        timeout_ms: 30_000,
        max_output_bytes: options.max_bytes,
    })
    .await;
    if stat_result.exit_code != Some(0) || options.stat_only {
        return stat_result;
    }

    let mut diff_args = base_args;
    diff_args.extend(path_args);
    let diff_result = run_process(ProcessInput {
        command: "git".to_owned(),
        args: diff_args,
        cwd: root.to_path_buf(),
        timeout_ms: 30_000,
        max_output_bytes: options.max_bytes,
    })
    .await;
    combine_process_results(&[stat_result, diff_result], options.max_bytes)
}

fn combine_process_results(results: &[ProcessResult], max_bytes: usize) -> ProcessResult {
    let stdout = results
        .iter()
        .map(|result| result.stdout.trim_end())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let stderr = results
        .iter()
        .map(|result| result.stderr.trim_end())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    ProcessResult {
        stdout: cap_text(stdout, max_bytes),
        stderr: cap_text(stderr, max_bytes),
        exit_code: results
            .iter()
            .find_map(|result| (result.exit_code != Some(0)).then_some(result.exit_code))
            .flatten()
            .or_else(|| results.last().and_then(|result| result.exit_code)),
        timed_out: results.iter().any(|result| result.timed_out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command};
    use tempfile::tempdir;

    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .expect("git command");
        assert!(
            output.status.success(),
            "git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test]
    async fn git_status_and_diff_are_formatted_and_capped() {
        let temp = tempdir().expect("temporary directory");
        git(temp.path(), &["init", "--quiet"]);
        git(temp.path(), &["config", "user.email", "test@example.com"]);
        git(temp.path(), &["config", "user.name", "Rust Test"]);
        fs::write(temp.path().join("sample.txt"), "before\n").expect("sample file");
        git(temp.path(), &["add", "sample.txt"]);
        git(temp.path(), &["commit", "--quiet", "-m", "initial"]);
        fs::write(temp.path().join("sample.txt"), "after\n").expect("updated file");

        let status = git_status(temp.path(), 1_000).await;
        assert!(status.stdout.contains("sample.txt"));

        let diff = git_diff(
            temp.path(),
            GitDiffOptions {
                staged: false,
                path: Some("sample.txt".to_owned()),
                stat_only: false,
                max_bytes: 1_000,
            },
        )
        .await;
        assert!(diff.stdout.contains("sample.txt"));
        assert!(diff.stdout.contains("+after"));
    }
}
