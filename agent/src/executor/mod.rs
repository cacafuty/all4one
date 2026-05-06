use crate::grpc_client;
use all4one_common::{JobId, JobResources, JobStatus, NodeId, Runtime};
use chrono::Utc;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{broadcast, RwLock};

use crate::api_rest::JobRecord;

#[derive(Debug, Clone)]
pub struct JobCompletionCallback {
    pub origin_endpoint: String,
    pub source_node_id: NodeId,
}

pub fn spawn_job(
    job_id: JobId,
    runtime: Runtime,
    source: String,
    command: Vec<String>,
    resources: JobResources,
    jobs: Arc<RwLock<HashMap<JobId, JobRecord>>>,
    output_channels: Arc<RwLock<HashMap<JobId, broadcast::Sender<String>>>>,
    completion_callback: Option<JobCompletionCallback>,
) {
    tokio::spawn(async move {
        println!(
            "INFO Job start id={} runtime={:?} source={} command_len={}",
            job_id,
            runtime,
            source,
            command.len(),
        );

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
            chans
                .entry(job_id)
                .or_insert_with(|| {
                    let (tx, _rx) = broadcast::channel(256);
                    tx
                })
                .clone()
        };

        let mut cmd = build_command(runtime, &source, &command, &resources);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let child = cmd.spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(err) => {
                eprintln!("ERROR Job spawn failed id={} error={}", job_id, err);
                let _ = sender.send(format!("[executor:error] spawn failed: {err}"));
                let mut jobs_guard = jobs.write().await;
                if let Some(job) = jobs_guard.get_mut(&job_id) {
                    job.status = JobStatus::Failed;
                    job.error = Some(format!("spawn failed: {err}"));
                    job.updated_at = Utc::now();
                }
                if let Some(callback) = &completion_callback {
                    let _ = grpc_client::report_job_status(
                        &callback.origin_endpoint,
                        job_id,
                        JobStatus::Failed,
                        None,
                        Some(&format!("spawn failed: {err}")),
                        callback.source_node_id,
                    )
                    .await;
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
                            println!("INFO Job completed id={} exit_code=0", job_id);
                            job.status = JobStatus::Completed;
                            job.exit_code = Some(0);
                        } else {
                            eprintln!("ERROR Job failed id={} exit_code={}", job_id, code);
                            job.status = JobStatus::Failed;
                            job.exit_code = Some(code);
                            job.error = Some(format!("process exited with code {code}"));
                        }
                        job.updated_at = Utc::now();
                    }
                }
                if let Some(callback) = &completion_callback {
                    let status = if code == 0 {
                        JobStatus::Completed
                    } else {
                        JobStatus::Failed
                    };
                    let error = if code == 0 {
                        None
                    } else {
                        Some(format!("process exited with code {code}"))
                    };
                    let _ = grpc_client::report_job_status(
                        &callback.origin_endpoint,
                        job_id,
                        status,
                        Some(code),
                        error.as_deref(),
                        callback.source_node_id,
                    )
                    .await;
                }
            }
            Err(err) => {
                eprintln!("ERROR Job wait failed id={} error={}", job_id, err);
                let mut jobs_guard = jobs.write().await;
                if let Some(job) = jobs_guard.get_mut(&job_id) {
                    job.status = JobStatus::Failed;
                    job.error = Some(format!("wait failed: {err}"));
                    job.updated_at = Utc::now();
                }
                if let Some(callback) = &completion_callback {
                    let _ = grpc_client::report_job_status(
                        &callback.origin_endpoint,
                        job_id,
                        JobStatus::Failed,
                        None,
                        Some(&format!("wait failed: {err}")),
                        callback.source_node_id,
                    )
                    .await;
                }
            }
        }
    });
}

fn build_command(
    runtime: Runtime,
    source: &str,
    args: &[String],
    resources: &JobResources,
) -> Command {
    match runtime {
        Runtime::Executable => {
            let mut c = Command::new(source);
            c.args(args);
            apply_non_docker_memory_limit(&mut c, resources);
            c
        }
        Runtime::Python => {
            let mut c = Command::new("python3");
            if args.is_empty() {
                c.arg("-c").arg(source);
            } else {
                c.args(args);
            }
            apply_non_docker_memory_limit(&mut c, resources);
            c
        }
        Runtime::Jar => {
            let mut c = Command::new("java");
            c.arg("-jar").arg(source).args(args);
            apply_non_docker_memory_limit(&mut c, resources);
            c
        }
        Runtime::Wasm => {
            let mut c = Command::new("wasmtime");
            c.arg(source).args(args);
            apply_non_docker_memory_limit(&mut c, resources);
            c
        }
        Runtime::Docker => {
            let mut c = Command::new("docker");
            let memory_arg = format!("{}m", resources.memory_mb);
            c.arg("run")
                .arg("--rm")
                .arg("--memory")
                .arg(&memory_arg)
                .arg("--memory-swap")
                .arg(&memory_arg)
                .arg("--cpus")
                .arg(resources.cpu_cores.to_string())
                .arg(source)
                .args(args);
            c
        }
    }
}

fn apply_non_docker_memory_limit(command: &mut Command, resources: &JobResources) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        let limit_bytes = (resources.memory_mb as u64)
            .saturating_mul(1024)
            .saturating_mul(1024);

        unsafe {
            command.pre_exec(move || {
                let limit = libc::rlimit {
                    rlim_cur: limit_bytes as libc::rlim_t,
                    rlim_max: limit_bytes as libc::rlim_t,
                };

                if libc::setrlimit(libc::RLIMIT_AS, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }

                Ok(())
            });
        }
    }

    #[cfg(windows)]
    {
        println!(
            "WARN Memory hard enforcement for non-docker jobs on Windows is not wired yet (requires Job Objects)"
        );
        let _ = command;
        let _ = resources;
    }
}
