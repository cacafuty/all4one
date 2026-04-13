use all4one_common::{JobId, JobStatus, Runtime};
use chrono::Utc;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{broadcast, RwLock};

use crate::api_rest::JobRecord;

pub fn spawn_job(
    job_id: JobId,
    runtime: Runtime,
    source: String,
    command: Vec<String>,
    jobs: Arc<RwLock<HashMap<JobId, JobRecord>>>,
    output_channels: Arc<RwLock<HashMap<JobId, broadcast::Sender<String>>>>,
) {
    tokio::spawn(async move {
        {
            let mut jobs_guard = jobs.write().await;
            if let Some(job) = jobs_guard.get_mut(&job_id) {
                job.status = JobStatus::Running;
                job.updated_at = Utc::now();
            } else {
                return;
            }
        }

        let sender = {
            let mut chans = output_channels.write().await;
            chans.entry(job_id)
                .or_insert_with(|| {
                    let (tx, _rx) = broadcast::channel(256);
                    tx
                })
                .clone()
        };

        let mut cmd = build_command(runtime, &source, &command);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let child = cmd.spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(err) => {
                let _ = sender.send(format!("[executor:error] spawn failed: {err}"));
                let mut jobs_guard = jobs.write().await;
                if let Some(job) = jobs_guard.get_mut(&job_id) {
                    job.status = JobStatus::Failed;
                    job.error = Some(format!("spawn failed: {err}"));
                    job.updated_at = Utc::now();
                }
                return;
            }
        };

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let tx_out = sender.clone();
        let out_task = tokio::spawn(async move {
            if let Some(stdout) = stdout {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = tx_out.send(line);
                }
            }
        });

        let tx_err = sender.clone();
        let err_task = tokio::spawn(async move {
            if let Some(stderr) = stderr {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = tx_err.send(format!("[stderr] {line}"));
                }
            }
        });

        let status = child.wait().await;
        let _ = out_task.await;
        let _ = err_task.await;

        match status {
            Ok(exit) => {
                let code = exit.code().unwrap_or(-1);
                let mut jobs_guard = jobs.write().await;
                if let Some(job) = jobs_guard.get_mut(&job_id) {
                    if job.status != JobStatus::Cancelled {
                        if code == 0 {
                            job.status = JobStatus::Completed;
                            job.exit_code = Some(0);
                        } else {
                            job.status = JobStatus::Failed;
                            job.exit_code = Some(code);
                            job.error = Some(format!("process exited with code {code}"));
                        }
                        job.updated_at = Utc::now();
                    }
                }
            }
            Err(err) => {
                let mut jobs_guard = jobs.write().await;
                if let Some(job) = jobs_guard.get_mut(&job_id) {
                    job.status = JobStatus::Failed;
                    job.error = Some(format!("wait failed: {err}"));
                    job.updated_at = Utc::now();
                }
            }
        }
    });
}

fn build_command(runtime: Runtime, source: &str, args: &[String]) -> Command {
    match runtime {
        Runtime::Executable => {
            let mut c = Command::new(source);
            c.args(args);
            c
        }
        Runtime::Python => {
            let mut c = Command::new("python3");
            if args.is_empty() {
                c.arg("-c").arg(source);
            } else {
                c.args(args);
            }
            c
        }
        Runtime::Jar => {
            let mut c = Command::new("java");
            c.arg("-jar").arg(source).args(args);
            c
        }
        Runtime::Wasm => {
            let mut c = Command::new("wasmtime");
            c.arg(source).args(args);
            c
        }
        Runtime::Docker => {
            let mut c = Command::new("docker");
            c.arg("run").arg("--rm").arg(source).args(args);
            c
        }
    }
}
