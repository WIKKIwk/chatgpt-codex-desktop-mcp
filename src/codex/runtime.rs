use std::{env, process::Stdio, sync::Arc};

use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::Command,
    sync::{Mutex, oneshot},
};

use crate::process::scrub_env;

use super::{
    model::BridgeState,
    protocol::{handle_message, mark_sessions_failed, reject_pending},
};

pub(crate) async fn spawn_server(
    command: &str,
    state: Arc<Mutex<BridgeState>>,
    max_output_bytes: usize,
) -> Result<(), String> {
    let mut child = Command::new(command);
    child
        .args(["app-server", "--listen", "stdio://"])
        .current_dir(env::current_dir().map_err(|error| error.to_string())?)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env_clear()
        .envs(scrub_env())
        .env("CODEX_ANALYTICS_ENABLED", "false");
    let mut child = child.spawn().map_err(|error| error.to_string())?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Codex app-server stdin was unavailable.".to_owned())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Codex app-server stdout was unavailable.".to_owned())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Codex app-server stderr was unavailable.".to_owned())?;
    let (kill_tx, kill_rx) = oneshot::channel();
    {
        let mut shared = state.lock().await;
        shared.stdin = Some(stdin);
        shared.kill_tx = Some(kill_tx);
    }

    let reader_state = state.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(message) = serde_json::from_str(line) else {
                continue;
            };
            let mut shared = reader_state.lock().await;
            handle_message(&mut shared, message, max_output_bytes).await;
        }
        let mut shared = reader_state.lock().await;
        reject_pending(&mut shared, "Codex app-server stdout closed.");
        mark_sessions_failed(&mut shared, "Codex app-server stdout closed.");
    });

    tokio::spawn(async move {
        let mut stderr = BufReader::new(stderr);
        let mut buffer = [0_u8; 8 * 1024];
        loop {
            match stderr.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    });

    let monitor_state = state;
    tokio::spawn(async move {
        tokio::select! {
            _ = child.wait() => {}
            _ = kill_rx => {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
        }
        let mut shared = monitor_state.lock().await;
        shared.stdin = None;
        shared.kill_tx = None;
        reject_pending(&mut shared, "Codex app-server exited.");
        mark_sessions_failed(&mut shared, "Codex app-server exited.");
    });
    Ok(())
}
