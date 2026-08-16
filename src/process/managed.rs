use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tokio::{
    process::{Child, Command},
    sync::oneshot,
    time::sleep,
};
use uuid::Uuid;

use super::runner::{ProcessInput, append_capped, read_capped, scrub_env};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedProcessSnapshot {
    pub id: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub running: bool,
    pub started_at: u64,
    pub finished_at: Option<u64>,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
}

#[derive(Debug)]
struct ProcessState {
    id: String,
    command: String,
    args: Vec<String>,
    cwd: String,
    running: bool,
    started_at: u64,
    finished_at: Option<u64>,
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
    timed_out: bool,
    max_output_bytes: usize,
}

#[derive(Debug)]
struct ManagedProcessEntry {
    state: Arc<Mutex<ProcessState>>,
    stop: Option<oneshot::Sender<()>>,
}

#[derive(Debug, Default)]
pub struct ManagedProcessStore {
    processes: HashMap<String, ManagedProcessEntry>,
}

impl ManagedProcessStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&mut self, input: ProcessInput) -> ManagedProcessSnapshot {
        let id = format!("proc_{}", Uuid::new_v4());
        let state = Arc::new(Mutex::new(ProcessState {
            id: id.clone(),
            command: input.command.clone(),
            args: input.args.clone(),
            cwd: input.cwd.display().to_string(),
            running: true,
            started_at: now_millis(),
            finished_at: None,
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            timed_out: false,
            max_output_bytes: input.max_output_bytes,
        }));

        let mut command = Command::new(&input.command);
        command
            .args(&input.args)
            .current_dir(&input.cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .env_clear()
            .envs(scrub_env());

        let entry = match command.spawn() {
            Ok(mut child) => {
                let stdout = child.stdout.take();
                let stderr = child.stderr.take();
                let (stop_sender, stop_receiver) = oneshot::channel();
                let task_state = Arc::clone(&state);
                tokio::spawn(watch_process(
                    task_state,
                    child,
                    stdout,
                    stderr,
                    stop_receiver,
                    input.timeout_ms,
                ));
                ManagedProcessEntry {
                    state,
                    stop: Some(stop_sender),
                }
            }
            Err(error) => {
                finish_with_error(&state, error.to_string());
                ManagedProcessEntry { state, stop: None }
            }
        };

        self.processes.insert(id.clone(), entry);
        self.processes
            .get(&id)
            .map(snapshot)
            .expect("managed process was inserted")
    }

    pub fn read(&self, id: &str) -> Result<ManagedProcessSnapshot, String> {
        self.processes
            .get(id)
            .map(snapshot)
            .ok_or_else(|| format!("Unknown process id: {id}"))
    }

    pub fn stop(&mut self, id: &str) -> Result<ManagedProcessSnapshot, String> {
        let entry = self
            .processes
            .get_mut(id)
            .ok_or_else(|| format!("Unknown process id: {id}"))?;
        let is_running = entry
            .state
            .lock()
            .map_err(|_| "process state is unavailable".to_owned())?
            .running;
        if is_running {
            if let Some(stop) = entry.stop.take() {
                let _ = stop.send(());
            }
            let mut state = entry
                .state
                .lock()
                .map_err(|_| "process state is unavailable".to_owned())?;
            state.running = false;
            state.finished_at = Some(now_millis());
        }
        Ok(snapshot(entry))
    }
}

async fn watch_process(
    state: Arc<Mutex<ProcessState>>,
    mut child: Child,
    stdout: Option<tokio::process::ChildStdout>,
    stderr: Option<tokio::process::ChildStderr>,
    mut stop_receiver: oneshot::Receiver<()>,
    timeout_ms: u64,
) {
    let Some(stdout) = stdout else {
        finish_with_error(&state, "process stdout stream was unavailable".to_owned());
        return;
    };
    let Some(stderr) = stderr else {
        finish_with_error(&state, "process stderr stream was unavailable".to_owned());
        return;
    };

    let max_bytes = state
        .lock()
        .ok()
        .map(|value| value.max_output_bytes)
        .unwrap_or(0);
    let stdout_task = tokio::spawn(read_capped(stdout, max_bytes));
    let stderr_task = tokio::spawn(read_capped(stderr, max_bytes));
    let timeout_task = sleep(Duration::from_millis(timeout_ms));
    tokio::pin!(timeout_task);

    let (exit_code, timed_out) = tokio::select! {
        status = child.wait() => (status.ok().and_then(|value| value.code()), false),
        _ = &mut stop_receiver => {
            let _ = child.kill().await;
            let code = child.wait().await.ok().and_then(|value| value.code());
            (code, false)
        }
        _ = &mut timeout_task => {
            let _ = child.kill().await;
            let code = child.wait().await.ok().and_then(|value| value.code());
            (code, true)
        }
    };

    let stdout = capture_result(stdout_task, max_bytes).await;
    let stderr = capture_result(stderr_task, max_bytes).await;
    if let Ok(mut value) = state.lock() {
        value.stdout = stdout;
        value.stderr = stderr;
        value.timed_out |= timed_out;
        if value.running {
            value.running = false;
            value.finished_at = Some(now_millis());
            value.exit_code = exit_code;
        }
    }
}

async fn capture_result(
    task: tokio::task::JoinHandle<std::io::Result<String>>,
    max_bytes: usize,
) -> String {
    match task.await {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => append_capped(String::new(), &error.to_string(), max_bytes),
        Err(error) => append_capped(String::new(), &error.to_string(), max_bytes),
    }
}

fn finish_with_error(state: &Arc<Mutex<ProcessState>>, error: String) {
    if let Ok(mut state) = state.lock() {
        state.running = false;
        state.finished_at = Some(now_millis());
        state.stderr = append_capped(String::new(), &error, state.max_output_bytes);
    }
}

fn snapshot(entry: &ManagedProcessEntry) -> ManagedProcessSnapshot {
    let state = entry.state.lock().expect("process state lock");
    ManagedProcessSnapshot {
        id: state.id.clone(),
        command: state.command.clone(),
        args: state.args.clone(),
        cwd: state.cwd.clone(),
        running: state.running,
        started_at: state.started_at,
        finished_at: state.finished_at,
        stdout: state.stdout.clone(),
        stderr: state.stderr.clone(),
        exit_code: state.exit_code,
        timed_out: state.timed_out,
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn format_managed_process(snapshot: &ManagedProcessSnapshot) -> String {
    let mut lines = vec![
        format!("process_id: {}", snapshot.id),
        format!("running: {}", snapshot.running),
        format!(
            "exit_code: {}",
            snapshot
                .exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "null".to_owned())
        ),
        format!("timed_out: {}", snapshot.timed_out),
    ];
    if !snapshot.stdout.is_empty() {
        lines.push(format!("stdout:\n{}", snapshot.stdout));
    }
    if !snapshot.stderr.is_empty() {
        lines.push(format!("stderr:\n{}", snapshot.stderr));
    }
    lines.join("\n")
}
