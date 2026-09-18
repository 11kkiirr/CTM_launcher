//! Child-process spawning and asynchronous `stdout`/`stderr` piping.

use std::path::Path;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use crate::error::{CoreError, Result};

/// Which stream a log line originated from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    Stdout,
    Stderr,
}

/// A single captured log line.
#[derive(Debug, Clone)]
pub struct LogLine {
    pub stream: StreamKind,
    pub text: String,
}

/// Owns the running child process.
#[derive(Debug)]
pub struct ProcessHandle {
    child: Child,
    pid: Option<u32>,
}

impl ProcessHandle {
    pub fn id(&self) -> Option<u32> {
        self.pid
    }

    /// Wait for the process to exit, returning its status.
    pub async fn wait(&mut self) -> Result<std::process::ExitStatus> {
        Ok(self.child.wait().await?)
    }

    /// Non-blocking check for process completion.
    pub fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>> {
        Ok(self.child.try_wait()?)
    }

    /// Terminate the process if it is still running.
    pub fn kill(&mut self) -> Result<()> {
        self.child
            .start_kill()
            .map_err(|e| CoreError::Launch(format!("failed to kill process: {e}")))
    }
}

/// Receives piped log lines from a spawned process.
#[derive(Debug)]
pub struct LogReceiver {
    rx: mpsc::UnboundedReceiver<LogLine>,
}

impl LogReceiver {
    /// Await the next line, or `None` once both streams close.
    pub async fn recv(&mut self) -> Option<LogLine> {
        self.rx.recv().await
    }

    /// Non-blocking variant for polling in the UI loop.
    pub fn try_recv(&mut self) -> Option<LogLine> {
        self.rx.try_recv().ok()
    }
}

/// Spawn `command` in `cwd`, piping stdout/stderr into an unbounded channel.
///
/// Returns a handle for lifecycle control and a receiver for log lines.
pub async fn spawn(command: &[String], cwd: &Path) -> Result<(ProcessHandle, LogReceiver)> {
    let (program, args) = command
        .split_first()
        .ok_or_else(|| CoreError::Launch("empty launch command".into()))?;

    let mut child = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| CoreError::Launch(format!("failed to spawn {program}: {e}")))?;

    let pid = child.id();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| CoreError::Launch("failed to capture stdout".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| CoreError::Launch("failed to capture stderr".into()))?;

    let (tx, rx) = mpsc::unbounded_channel();

    let tx_out = tx.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(text)) = lines.next_line().await {
            if tx_out
                .send(LogLine {
                    stream: StreamKind::Stdout,
                    text,
                })
                .is_err()
            {
                break;
            }
        }
    });

    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(text)) = lines.next_line().await {
            if tx
                .send(LogLine {
                    stream: StreamKind::Stderr,
                    text,
                })
                .is_err()
            {
                break;
            }
        }
    });

    Ok((ProcessHandle { child, pid }, LogReceiver { rx }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn captures_stdout_and_stderr() {
        let cmd = if cfg!(windows) {
            vec![
                "cmd".to_string(),
                "/C".to_string(),
                "echo hello & echo oops 1>&2".to_string(),
            ]
        } else {
            vec![
                "sh".to_string(),
                "-c".to_string(),
                "echo hello; echo oops 1>&2".to_string(),
            ]
        };

        let (mut handle, mut logs) = spawn(&cmd, Path::new(".")).await.unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        while let Some(line) = logs.recv().await {
            match line.stream {
                StreamKind::Stdout => stdout.push(line.text),
                StreamKind::Stderr => stderr.push(line.text),
            }
        }
        let status = handle.wait().await.unwrap();
        assert!(status.success());
        assert_eq!(stdout, vec!["hello"]);
        assert_eq!(stderr, vec!["oops"]);
    }

    #[tokio::test]
    async fn spawn_rejects_empty_command() {
        assert!(spawn(&[], Path::new(".")).await.is_err());
    }
}
