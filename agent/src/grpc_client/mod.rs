use crate::api_rest::{SubmitJobRequest, SubmitCapabilities, SubmitConstraints};
use tonic::transport::Channel;

pub mod proto {
    tonic::include_proto!("all4one.v1");
}

pub async fn submit_remote(endpoint: &str, request: &SubmitJobRequest) -> Result<(), tonic::Status> {
    let target = normalize_endpoint(endpoint);
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
        })
        .await?;
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
