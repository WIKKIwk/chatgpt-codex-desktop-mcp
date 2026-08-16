use std::{
    ffi::{OsStr, OsString},
    io,
    path::PathBuf,
    process::Stdio,
    time::Duration,
};

use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    time::timeout,
};

use crate::config::AccessMode;

const OUTPUT_TRUNCATION_MARKER: &str = "\n[output truncated]\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
}

#[derive(Debug, Clone)]
pub struct ProcessInput {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
}

pub fn assert_process_allowed(
    command: &str,
    args: &[String],
    access_mode: AccessMode,
) -> Result<(), String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err("Empty process command.".to_owned());
    }

    let executable = trimmed
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(trimmed)
        .to_ascii_lowercase();
    if blocked_process_commands().contains(&executable.as_str()) {
        return Err(
            "Process command launches a shell or blocked system tool. Use structured tools instead."
                .to_owned(),
        );
    }

    let joined = std::iter::once(trimmed)
        .chain(args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    if blocked_policy_pattern(&joined) {
        return Err("Process arguments match a blocked policy pattern.".to_owned());
    }

    let allowed = match access_mode {
        AccessMode::Full => true,
        AccessMode::Coding => coding_process_allowed(&executable, args),
        AccessMode::Review => review_process_allowed(&executable, args),
    };
    if allowed {
        Ok(())
    } else {
        Err(format!(
            "Process command is not in the {}-mode allowlist.",
            access_mode_name(access_mode)
        ))
    }
}

pub async fn run_process(input: ProcessInput) -> ProcessResult {
    let mut command = Command::new(&input.command);
    command
        .args(&input.args)
        .current_dir(&input.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env_clear()
        .envs(scrub_env());

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return ProcessResult {
                stdout: String::new(),
                stderr: error.to_string(),
                exit_code: None,
                timed_out: false,
            };
        }
    };

    let Some(stdout) = child.stdout.take() else {
        return missing_output_result(&mut child, "stdout").await;
    };
    let Some(stderr) = child.stderr.take() else {
        return missing_output_result(&mut child, "stderr").await;
    };

    let stdout_task = tokio::spawn(read_capped(stdout, input.max_output_bytes));
    let stderr_task = tokio::spawn(read_capped(stderr, input.max_output_bytes));
    let wait_result = timeout(Duration::from_millis(input.timeout_ms), child.wait()).await;

    let (exit_code, timed_out, mut stderr_extra) = match wait_result {
        Ok(Ok(status)) => (status.code(), false, None),
        Ok(Err(error)) => (None, false, Some(error.to_string())),
        Err(_) => {
            let _ = child.kill().await;
            let exit_code = child.wait().await.ok().and_then(|status| status.code());
            (exit_code, true, None)
        }
    };

    let stdout = join_output(stdout_task, input.max_output_bytes, "stdout").await;
    let mut stderr = join_output(stderr_task, input.max_output_bytes, "stderr").await;
    if let Some(error) = stderr_extra.take() {
        stderr = append_capped(stderr, &error, input.max_output_bytes);
    }

    ProcessResult {
        stdout,
        stderr,
        exit_code,
        timed_out,
    }
}

pub fn format_process_result(result: &ProcessResult) -> String {
    let mut lines = vec![
        format!(
            "exit_code: {}",
            result
                .exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "null".to_owned())
        ),
        format!("timed_out: {}", result.timed_out),
    ];
    if !result.stdout.is_empty() {
        lines.push(format!("stdout:\n{}", result.stdout));
    }
    if !result.stderr.is_empty() {
        lines.push(format!("stderr:\n{}", result.stderr));
    }
    lines.join("\n")
}

pub fn scrub_env() -> Vec<(OsString, OsString)> {
    std::env::vars_os()
        .filter(|(key, _)| !is_sensitive_env_name(key))
        .collect()
}

pub(crate) async fn read_capped<R>(mut reader: R, max_bytes: usize) -> io::Result<String>
where
    R: AsyncRead + Unpin,
{
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        if output.len() < max_bytes {
            let kept = read.min(max_bytes - output.len());
            output.extend_from_slice(&buffer[..kept]);
            truncated |= kept < read;
        } else {
            truncated = true;
        }
    }
    let mut text = String::from_utf8_lossy(&output).into_owned();
    if truncated {
        text.push_str(OUTPUT_TRUNCATION_MARKER);
    }
    Ok(text)
}

pub(crate) fn append_capped(current: String, next: &str, max_bytes: usize) -> String {
    let combined = format!("{current}{next}");
    cap_text(combined, max_bytes)
}

pub(crate) fn cap_text(mut text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }
    text.truncate(valid_utf8_boundary(&text, max_bytes));
    text.push_str(OUTPUT_TRUNCATION_MARKER);
    text
}

async fn missing_output_result(child: &mut tokio::process::Child, stream: &str) -> ProcessResult {
    let _ = child.kill().await;
    let exit_code = child.wait().await.ok().and_then(|status| status.code());
    ProcessResult {
        stdout: String::new(),
        stderr: format!("process {stream} stream was unavailable"),
        exit_code,
        timed_out: false,
    }
}

async fn join_output(
    task: tokio::task::JoinHandle<io::Result<String>>,
    max_bytes: usize,
    stream: &str,
) -> String {
    let output = match task.await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => format!("process {stream} read failed: {error}"),
        Err(error) => format!("process {stream} reader failed: {error}"),
    };
    cap_text(output, max_bytes)
}

fn coding_process_allowed(executable: &str, args: &[String]) -> bool {
    let first = args.first().map(|value| value.to_ascii_lowercase());
    if executable == "git" || executable == "git.exe" {
        return first.as_deref().is_some_and(git_read_subcommand);
    }
    if package_manager(executable) {
        let Some(first) = first.as_deref() else {
            return false;
        };
        if first != "test" && first != "run" {
            return false;
        }
        let script = if first == "run" {
            args.get(1).map(|value| value.to_ascii_lowercase())
        } else {
            Some("test".to_owned())
        };
        return script.as_deref().is_some_and(safe_project_script);
    }
    if matches!(executable, "cargo" | "cargo.exe") {
        return first
            .as_deref()
            .is_some_and(|value| matches!(value, "build" | "check" | "clippy" | "fmt" | "test"));
    }
    if matches!(executable, "flutter" | "flutter.bat") {
        return first
            .as_deref()
            .is_some_and(|value| matches!(value, "analyze" | "build" | "test"));
    }
    if matches!(executable, "dart" | "dart.exe") {
        return first
            .as_deref()
            .is_some_and(|value| matches!(value, "analyze" | "format" | "test"));
    }
    review_process_allowed(executable, args)
}

fn review_process_allowed(executable: &str, args: &[String]) -> bool {
    let first = args.first().map(|value| value.to_ascii_lowercase());
    if executable == "git" || executable == "git.exe" {
        return first.as_deref().is_some_and(git_read_subcommand);
    }
    if package_manager(executable) {
        return first
            .as_deref()
            .is_some_and(|value| value == "test" || value == "run");
    }
    if matches!(
        executable,
        "node" | "node.exe" | "python" | "python.exe" | "python3" | "python3.exe" | "py" | "py.exe"
    ) {
        return args.len() == 1 && matches!(args[0].as_str(), "--version" | "-v" | "-V");
    }
    matches!(
        executable,
        "pytest" | "pytest.exe" | "rg" | "rg.exe" | "grep" | "grep.exe" | "where" | "where.exe"
    )
}

fn safe_project_script(script: &str) -> bool {
    if [
        "deploy", "publish", "release", "migrat", "seed", "install", "upload", "ship", "prod",
    ]
    .iter()
    .any(|word| script.contains(word))
    {
        return false;
    }
    script.split(':').any(|part| {
        matches!(
            part,
            "test" | "check" | "typecheck" | "lint" | "build" | "format" | "fmt"
        )
    })
}

fn git_read_subcommand(value: &str) -> bool {
    matches!(
        value,
        "status" | "diff" | "log" | "show" | "branch" | "rev-parse" | "ls-files" | "--version"
    )
}

fn package_manager(executable: &str) -> bool {
    matches!(
        executable,
        "npm" | "npm.cmd" | "pnpm" | "pnpm.cmd" | "yarn" | "yarn.cmd"
    )
}

fn blocked_process_commands() -> &'static [&'static str] {
    &[
        "bash",
        "bash.exe",
        "cmd",
        "cmd.exe",
        "format",
        "format.com",
        "powershell",
        "powershell.exe",
        "pwsh",
        "pwsh.exe",
        "reboot",
        "reboot.exe",
        "reg",
        "reg.exe",
        "sc",
        "sc.exe",
        "sh",
        "sh.exe",
        "shutdown",
        "shutdown.exe",
    ]
}

fn blocked_policy_pattern(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let has_word = |needle: &str| contains_word(&lower, needle);
    if has_rm_recursive_force(&lower)
        || has_del_s_or_q(&lower)
        || has_sequence(&lower, "reg", "delete")
        || has_sequence(&lower, "net", "user")
        || has_sequence(&lower, "sc", "delete")
        || has_sequence(&lower, "sed", "-i")
        || has_sequence(&lower, "perl", "-i")
        || has_download_pipe(&lower)
        || has_redirection(&lower)
        || [
            "format",
            "shutdown",
            "reboot",
            "iex",
            "invoke-expression",
            "tee",
        ]
        .iter()
        .any(|needle| has_word(needle))
    {
        return true;
    }
    false
}

fn has_rm_recursive_force(value: &str) -> bool {
    let mut tokens = value.split_whitespace();
    while let Some(token) = tokens.next() {
        if token != "rm" {
            continue;
        }
        if let Some(flags) = tokens.next() {
            let flags = flags.strip_prefix('-').unwrap_or("");
            if flags.contains('r') && flags.contains('f') {
                return true;
            }
        }
    }
    false
}

fn has_del_s_or_q(value: &str) -> bool {
    let mut tokens = value.split_whitespace();
    while let Some(token) = tokens.next() {
        if token == "del"
            && tokens
                .next()
                .is_some_and(|next| next == "/s" || next == "/q")
        {
            return true;
        }
    }
    false
}

fn has_sequence(value: &str, first: &str, second: &str) -> bool {
    let tokens = value.split_whitespace().collect::<Vec<_>>();
    tokens
        .windows(2)
        .any(|pair| pair[0] == first && pair[1] == second)
}

fn has_download_pipe(value: &str) -> bool {
    let download = ["curl", "wget", "irm", "iwr", "invoke-webrequest"];
    let shell = [
        "sh",
        "bash",
        "pwsh",
        "powershell",
        "iex",
        "invoke-expression",
    ];
    if !download.iter().any(|word| contains_word(value, word)) {
        return false;
    }
    value
        .split('|')
        .skip(1)
        .filter_map(|part| part.split_whitespace().next())
        .any(|command| shell.contains(&command))
}

fn has_redirection(value: &str) -> bool {
    value.match_indices('>').any(|(index, _)| {
        value[index + 1..]
            .chars()
            .find(|character| !character.is_whitespace())
            .is_some_and(|next| next != '&')
    })
}

fn contains_word(value: &str, needle: &str) -> bool {
    if needle.contains('-') {
        return value.split_whitespace().any(|word| {
            word.trim_matches(|character: char| {
                !character.is_ascii_alphanumeric() && character != '-' && character != '_'
            }) == needle
        });
    }
    value
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .any(|word| word == needle)
}

fn is_sensitive_env_name(key: &OsStr) -> bool {
    let name = key.to_string_lossy().to_ascii_lowercase();
    name.contains("token")
        || name.contains("secret")
        || name.contains("password")
        || name.contains("api_key")
        || name.contains("api-key")
        || name.contains("apikey")
}

fn valid_utf8_boundary(value: &str, max_bytes: usize) -> usize {
    value
        .char_indices()
        .take_while(|(index, _)| *index <= max_bytes)
        .map(|(index, _)| index)
        .last()
        .unwrap_or(0)
}

fn access_mode_name(value: AccessMode) -> &'static str {
    match value {
        AccessMode::Review => "review",
        AccessMode::Coding => "coding",
        AccessMode::Full => "full",
    }
}
