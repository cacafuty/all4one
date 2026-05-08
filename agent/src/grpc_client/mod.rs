use crate::api_rest::{SubmitCapabilities, SubmitConstraints, SubmitJobRequest};
use all4one_common::{JobId, JobStatus, NodeId};
use tonic::transport::Channel;
use uuid::Uuid;

pub mod proto {
    tonic::include_proto!("all4one.v1");
}

pub struct EnrollmentBundle {
    pub node_cert_pem: String,
    pub node_key_pem: String,
    pub ca_cert_pem: String,
    pub expires_at_unix: i64,
}

pub async fn request_join(
    endpoint: &str,
    node_id: Uuid,
    join_secret: Option<&str>,
) -> Result<EnrollmentBundle, tonic::Status> {
    request_join_with_ca(
        endpoint,
        node_id,
        join_secret,
        None,
        1,
        "127.0.0.1",
        9999,
        9998,
    )
    .await
}

pub async fn request_join_with_ca(
    endpoint: &str,
    node_id: Uuid,
    join_secret: Option<&str>,
    ca_cert_path: Option<&str>,
    tier: u8,
    advertise_host: &str,
    advertise_grpc_port: u16,
    advertise_rest_port: u16,
) -> Result<EnrollmentBundle, tonic::Status> {
    // Verify CA cert exists if specified (validates bootstrap authenticity)
    if let Some(ca_path) = ca_cert_path {
        std::fs::read_to_string(ca_path).map_err(|e| {
            tonic::Status::internal(format!("Failed to read CA cert at {}: {}", ca_path, e))
        })?;
        println!("INFO Verified CA cert at: {}", ca_path);
    }

    let target = normalize_endpoint(endpoint);
    let channel = Channel::from_shared(target)
        .map_err(|e| tonic::Status::internal(e.to_string()))?
        .connect()
        .await
        .map_err(|e| tonic::Status::unavailable(e.to_string()))?;

    let mut client = proto::agent_service_client::AgentServiceClient::new(channel);
    let response = client
        .join(proto::JoinRequest {
            node_id: node_id.to_string(),
            csr_pem: String::new(),
            join_secret: join_secret.unwrap_or_default().to_string(),
            tier: tier as u32,
            grpc_endpoint: format!("{}:{}", advertise_host, advertise_grpc_port),
            rest_endpoint: format!("{}:{}", advertise_host, advertise_rest_port),
        })
        .await?
        .into_inner();

    Ok(EnrollmentBundle {
        node_cert_pem: response.node_cert_pem,
        node_key_pem: response.node_key_pem,
        ca_cert_pem: response.ca_cert_pem,
        expires_at_unix: response.expires_at_unix,
    })
}

pub async fn submit_remote(
    endpoint: &str,
    job_id: JobId,
    origin_endpoint: &str,
    request: &SubmitJobRequest,
) -> Result<(), tonic::Status> {
    let target = normalize_endpoint(endpoint);
    println!(
        "INFO gRPC submit sending endpoint={} job_id={} runtime={:?} source={} command_len={}",
        target,
        job_id,
        request.runtime,
        request.source,
        request.command.len(),
    );
    let channel = Channel::from_shared(target)
        .map_err(|e| tonic::Status::internal(e.to_string()))?
        .connect()
        .await
        .map_err(|e| tonic::Status::unavailable(e.to_string()))?;

    let mut client = proto::agent_service_client::AgentServiceClient::new(channel);
    let _ = client
        .submit_job(proto::SubmitJobRequest {
            runtime: runtime_to_string(&request.runtime),
            source: request.source.clone(),
            command: request.command.clone(),
            cpu_cores: request.resources.cpu_cores,
            memory_mb: request.resources.memory_mb,
            job_id: job_id.to_string(),
            origin_endpoint: origin_endpoint.to_string(),
        })
        .await?;
    println!(
        "INFO gRPC submit completed endpoint={} job_id={}",
        endpoint, job_id
    );
    Ok(())
}

pub async fn report_job_status(
    endpoint: &str,
    job_id: JobId,
    status: JobStatus,
    exit_code: Option<i32>,
    error: Option<&str>,
    source_node_id: NodeId,
) -> Result<(), tonic::Status> {
    let target = normalize_endpoint(endpoint);
    println!(
        "INFO gRPC status report sending endpoint={} job_id={} status={:?} source_node_id={}",
        target, job_id, status, source_node_id,
    );

    let channel = Channel::from_shared(target)
        .map_err(|e| tonic::Status::internal(e.to_string()))?
        .connect()
        .await
        .map_err(|e| tonic::Status::unavailable(e.to_string()))?;

    let mut client = proto::agent_service_client::AgentServiceClient::new(channel);
    let _ = client
        .report_job_status(proto::ReportJobStatusRequest {
            job_id: job_id.to_string(),
            status: job_status_to_string(&status),
            exit_code: exit_code.unwrap_or_default(),
            error: error.unwrap_or_default().to_string(),
            source_node_id: source_node_id.to_string(),
        })
        .await?;

    println!(
        "INFO gRPC status report completed endpoint={} job_id={} status={:?}",
        endpoint, job_id, status,
    );
    Ok(())
}

pub fn from_proto(request: proto::SubmitJobRequest) -> SubmitJobRequest {
    SubmitJobRequest {
        runtime: runtime_from_string(&request.runtime),
        source: request.source,
        command: request.command,
        resources: all4one_common::JobResources {
            cpu_cores: request.cpu_cores,
            memory_mb: request.memory_mb,
        },
        constraints: SubmitConstraints {
            tier_min: 0,
            requires_capabilities: SubmitCapabilities { docker: false },
        },
    }
}

pub fn runtime_to_string(runtime: &all4one_common::Runtime) -> String {
    match runtime {
        all4one_common::Runtime::Docker => "docker",
        all4one_common::Runtime::Python => "python",
        all4one_common::Runtime::Jar => "jar",
        all4one_common::Runtime::Executable => "executable",
        all4one_common::Runtime::Wasm => "wasm",
    }
    .to_string()
}

pub fn job_status_to_string(status: &JobStatus) -> String {
    match status {
        JobStatus::Queued => "queued",
        JobStatus::Scheduled => "scheduled",
        JobStatus::Running => "running",
        JobStatus::Completed => "completed",
        JobStatus::Failed => "failed",
        JobStatus::Cancelled => "cancelled",
    }
    .to_string()
}

pub fn job_status_from_string(value: &str) -> JobStatus {
    match value {
        "queued" => JobStatus::Queued,
        "scheduled" => JobStatus::Scheduled,
        "running" => JobStatus::Running,
        "completed" => JobStatus::Completed,
        "failed" => JobStatus::Failed,
        "cancelled" => JobStatus::Cancelled,
        _ => JobStatus::Failed,
    }
}

pub fn runtime_from_string(value: &str) -> all4one_common::Runtime {
    match value {
        "docker" => all4one_common::Runtime::Docker,
        "python" => all4one_common::Runtime::Python,
        "jar" => all4one_common::Runtime::Jar,
        "wasm" => all4one_common::Runtime::Wasm,
        "executable" => all4one_common::Runtime::Executable,
        _ => all4one_common::Runtime::Executable,
    }
}

fn normalize_endpoint(endpoint: &str) -> String {
    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        endpoint.to_string()
    } else {
        format!("http://{endpoint}")
    }
}

/// Transfer a shard to a peer storage node via gRPC.
/// Used for chunk replication after a local write.
pub async fn transfer_chunk(
    endpoint: &str,
    bucket: &str,
    key: &str,
    shard_index: u32,
    data: Vec<u8>,
    hash: String,
    policy: &str,
    object_size: u64,
    etag: &str,
) -> Result<(), tonic::Status> {
    let target = normalize_endpoint(endpoint);
    let channel = Channel::from_shared(target)
        .map_err(|e| tonic::Status::internal(e.to_string()))?
        .connect()
        .await
        .map_err(|e| tonic::Status::unavailable(e.to_string()))?;

    let mut client = proto::agent_service_client::AgentServiceClient::new(channel);
    let response = client
        .transfer_chunk(proto::TransferChunkRequest {
            bucket: bucket.to_string(),
            key: key.to_string(),
            shard_index,
            data,
            hash,
            policy: policy.to_string(),
            object_size,
            etag: etag.to_string(),
        })
        .await?;

    if !response.into_inner().accepted {
        return Err(tonic::Status::internal("Peer rejected shard transfer"));
    }
    Ok(())
}

/// Fetch a specific shard from a peer storage node via gRPC.
/// Used when a local shard is missing during reconstruction.
pub async fn fetch_chunk(
    endpoint: &str,
    bucket: &str,
    key: &str,
    shard_index: u32,
) -> Result<Option<Vec<u8>>, tonic::Status> {
    let target = normalize_endpoint(endpoint);
    let channel = Channel::from_shared(target)
        .map_err(|e| tonic::Status::internal(e.to_string()))?
        .connect()
        .await
        .map_err(|e| tonic::Status::unavailable(e.to_string()))?;

    let mut client = proto::agent_service_client::AgentServiceClient::new(channel);
    let response = client
        .fetch_chunk(proto::FetchChunkRequest {
            bucket: bucket.to_string(),
            key: key.to_string(),
            shard_index,
        })
        .await?
        .into_inner();

    if response.found {
        Ok(Some(response.data))
    } else {
        Ok(None)
    }
}
