use crate::api_rest::{apply_terminal_job_update, enqueue_job, enqueue_remote_job, AppState};
use crate::grpc_client::{from_proto, job_status_from_string, proto};
use all4one_common::{JobId, JobStatus};
use tonic::{Request, Response, Status};
use uuid::Uuid;

pub mod raft_service;
use raft_service::{proto_raft::raft_service_server::RaftServiceServer, RaftServiceImpl};

#[derive(Clone)]
struct ServiceImpl {
    state: AppState,
}

#[tonic::async_trait]
impl proto::agent_service_server::AgentService for ServiceImpl {
    async fn submit_job(
        &self,
        request: Request<proto::SubmitJobRequest>,
    ) -> Result<Response<proto::SubmitJobResponse>, Status> {
        let request = request.into_inner();
        let req = from_proto(request.clone());
        println!(
            "INFO gRPC submit received job_id={} runtime={:?} source={} command_len={}",
            request.job_id,
            req.runtime,
            req.source,
            req.command.len(),
        );
        let submitted = if request.job_id.trim().is_empty() {
            enqueue_job(self.state.clone(), req)
                .await
                .map_err(Status::invalid_argument)?
        } else {
            let job_id = Uuid::parse_str(&request.job_id)
                .map(JobId)
                .map_err(|_| Status::invalid_argument("invalid delegated job id"))?;
            let origin_endpoint = if request.origin_endpoint.trim().is_empty() {
                None
            } else {
                Some(request.origin_endpoint)
            };

            enqueue_remote_job(self.state.clone(), job_id, req, origin_endpoint)
                .await
                .map_err(Status::invalid_argument)?
        };

        println!(
            "INFO gRPC submit queued job_id={} assigned_to={}",
            submitted.job_id, submitted.assigned_to,
        );

        Ok(Response::new(proto::SubmitJobResponse {
            job_id: submitted.job_id.to_string(),
            status: "queued".to_string(),
            assigned_to: submitted.assigned_to.to_string(),
            created_at: submitted.created_at.to_rfc3339(),
        }))
    }

    async fn report_job_status(
        &self,
        request: Request<proto::ReportJobStatusRequest>,
    ) -> Result<Response<proto::ReportJobStatusResponse>, Status> {
        let payload = request.into_inner();
        let job_id = Uuid::parse_str(&payload.job_id)
            .map(JobId)
            .map_err(|_| Status::invalid_argument("invalid job id in status report"))?;
        let status = job_status_from_string(&payload.status);
        let exit_code = match status {
            JobStatus::Completed => Some(payload.exit_code),
            JobStatus::Failed => {
                if payload.exit_code == 0 {
                    None
                } else {
                    Some(payload.exit_code)
                }
            }
            _ => None,
        };
        let error = if payload.error.trim().is_empty() {
            None
        } else {
            Some(payload.error)
        };

        println!(
            "INFO gRPC status report received job_id={} status={:?} source_node_id={}",
            job_id, status, payload.source_node_id,
        );

        apply_terminal_job_update(self.state.clone(), job_id, status, exit_code, error)
            .await
            .map_err(Status::not_found)?;

        Ok(Response::new(proto::ReportJobStatusResponse {}))
    }

    async fn join(
        &self,
        request: Request<proto::JoinRequest>,
    ) -> Result<Response<proto::JoinResponse>, Status> {
        let payload = request.into_inner();
        let node_id = Uuid::parse_str(&payload.node_id)
            .map_err(|_| Status::invalid_argument("invalid node_id in join request"))?;

        // Dual-mode enrollment support:
        // - CA mode (default): no join_secret required.
        // - Dev shared-secret mode: caller provides join_secret matching local config.
        let provided_join_secret = payload.join_secret.trim();
        if !provided_join_secret.is_empty() {
            if self.state.config.security.mode != "shared-secret" {
                return Err(Status::unauthenticated(
                    "join_secret enrollment is only allowed in dev mode",
                ));
            }

            let configured_secret = self.state.config.security.shared_secret.trim();
            if configured_secret.is_empty() || provided_join_secret != configured_secret {
                return Err(Status::unauthenticated("invalid join_secret"));
            }
        }

        // Nodes without CA keys still accept join as a presence registration endpoint.
        // This enables distributed entry points where any reachable node can ingest peers.
        let cert_manager =
            crate::certificates::CertificateManager::new(&self.state.config.node.data_dir);
        let can_issue_certs = cert_manager.has_ca_key();

        println!("INFO gRPC Join request from node_id={} granted", node_id,);

        // Add joining node to cluster state with its profile information
        let joining_node = all4one_common::NodeInfo {
            profile: all4one_common::NodeProfile {
                id: all4one_common::NodeId(node_id),
                tier: payload.tier as u8,
                availability: String::new(),
                quorum_participant: false,
                resources: all4one_common::NodeResources {
                    cpu_cores: 0,
                    memory_mb: 0,
                    disk_mb: None,
                },
                capabilities: all4one_common::NodeCapabilities {
                    docker: false,
                    python: None,
                    java: None,
                    wasm: false,
                    gpu_enabled: false,
                    operating_system: String::new(),
                    storage_node: false,
                },
            },
            status: all4one_common::NodeStatus::Online,
            version: env!("CARGO_PKG_VERSION").to_string(),
            grpc_endpoint: payload.grpc_endpoint.clone(),
            rest_endpoint: payload.rest_endpoint.clone(),
        };

        // Upsert the joining node to the cluster state
        println!(
            "DEBUG [JOIN] Before insert: cluster has {} nodes",
            self.state.cluster.read().await.nodes.len()
        );

        {
            let mut cluster = self.state.cluster.write().await;
            cluster
                .nodes
                .insert(joining_node.profile.id, joining_node.clone());
            cluster.version = cluster.version.saturating_add(1);

            println!(
                "DEBUG [JOIN] After insert: cluster has {} nodes",
                cluster.nodes.len()
            );
            println!(
                "DEBUG [JOIN] Cluster now contains: {}",
                cluster
                    .nodes
                    .values()
                    .map(|n| format!("{} (tier={})", n.profile.id, n.profile.tier))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }

        // Mark heartbeat for the joining node to prevent immediate "suspected" status
        {
            let mut last_seen = self.state.last_seen.write().await;
            last_seen.insert(joining_node.profile.id, tokio::time::Instant::now());
            println!(
                "DEBUG [JOIN] Marked heartbeat for node {}",
                joining_node.profile.id
            );
        }

        println!(
            "INFO Cluster state updated: node {} (tier={}) added to cluster at {} / {}",
            node_id, payload.tier, payload.grpc_endpoint, payload.rest_endpoint
        );
        crate::discovery::remember_known_seed(
            &self.state.config.node.data_dir,
            &payload.grpc_endpoint,
        );

        if !can_issue_certs {
            return Ok(Response::new(proto::JoinResponse {
                node_cert_pem: String::new(),
                node_key_pem: String::new(),
                ca_cert_pem: String::new(),
                expires_at_unix: 0,
            }));
        }

        cert_manager
            .init_ca()
            .await
            .map_err(|e| Status::internal(format!("Failed to initialize CA: {}", e)))?;

        let (node_cert_pem, node_key_pem) = cert_manager
            .generate_node_cert(node_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to generate cert: {}", e)))?;

        let ca_cert_pem = cert_manager
            .load_ca_pem()
            .map_err(|e| Status::internal(format!("Failed to load CA cert: {}", e)))?;

        Ok(Response::new(proto::JoinResponse {
            node_cert_pem,
            node_key_pem,
            ca_cert_pem,
            expires_at_unix: chrono::Utc::now().timestamp() + (90 * 24 * 3600),
        }))
    }

    async fn transfer_chunk(
        &self,
        request: Request<proto::TransferChunkRequest>,
    ) -> Result<Response<proto::TransferChunkResponse>, Status> {
        if !self.state.config.roles.storage {
            return Ok(Response::new(proto::TransferChunkResponse {
                accepted: false,
                error: "This node does not have the storage role".to_string(),
            }));
        }

        let payload = request.into_inner();
        let chunks_dir = std::path::Path::new(&self.state.config.node.data_dir)
            .join("chunks")
            .join(&payload.bucket);

        if let Err(e) = std::fs::create_dir_all(&chunks_dir) {
            return Ok(Response::new(proto::TransferChunkResponse {
                accepted: false,
                error: format!("Cannot create chunk dir: {e}"),
            }));
        }

        let safe_key = payload.key.replace('/', "-");
        let object_id = format!("{}-{}", payload.bucket, safe_key);
        let shard_id = format!("{}-shard-{}", object_id, payload.shard_index);
        let shard_path = chunks_dir.join(&shard_id);

        if let Err(e) = std::fs::write(&shard_path, &payload.data) {
            return Ok(Response::new(proto::TransferChunkResponse {
                accepted: false,
                error: format!("Write failed: {e}"),
            }));
        }

        let meta_content = format!(
            "shard={},hash={},original_size={}",
            payload.shard_index, payload.hash, payload.object_size
        );
        let meta_path = format!("{}.meta", shard_path.display());
        let _ = std::fs::write(meta_path, meta_content);

        // Update the sled metadata index so that GET requests on this node
        // can find the object after all shards have been received.
        let _ = crate::storage::index::create_bucket(
            std::path::Path::new(&self.state.config.node.data_dir),
            &payload.bucket,
        )
        .await;
        let recv_policy = match payload.policy.as_str() {
            "hot" => crate::storage::StoragePolicy::Hot,
            "cold" => crate::storage::StoragePolicy::Cold,
            "archive" => crate::storage::StoragePolicy::Archive,
            _ => crate::storage::StoragePolicy::Warm,
        };
        let _ = crate::storage::index::put_object(
            std::path::Path::new(&self.state.config.node.data_dir),
            &payload.bucket,
            &payload.key,
            &payload.etag,
            payload.object_size as usize,
            &recv_policy,
        )
        .await;

        println!(
            "INFO gRPC TransferChunk accepted bucket={} key={} shard={}",
            payload.bucket, payload.key, payload.shard_index
        );

        Ok(Response::new(proto::TransferChunkResponse {
            accepted: true,
            error: String::new(),
        }))
    }

    async fn fetch_chunk(
        &self,
        request: Request<proto::FetchChunkRequest>,
    ) -> Result<Response<proto::FetchChunkResponse>, Status> {
        let payload = request.into_inner();
        let chunks_dir = std::path::Path::new(&self.state.config.node.data_dir)
            .join("chunks")
            .join(&payload.bucket);

        let safe_key = payload.key.replace('/', "-");
        let object_id = format!("{}-{}", payload.bucket, safe_key);
        let shard_id = format!("{}-shard-{}", object_id, payload.shard_index);
        let shard_path = chunks_dir.join(&shard_id);

        if !shard_path.exists() {
            return Ok(Response::new(proto::FetchChunkResponse {
                data: Vec::new(),
                hash: String::new(),
                found: false,
            }));
        }

        match std::fs::read(&shard_path) {
            Ok(data) => {
                let hash = crate::storage::chunks::sha256(&data);
                println!(
                    "INFO gRPC FetchChunk served bucket={} key={} shard={} bytes={}",
                    payload.bucket,
                    payload.key,
                    payload.shard_index,
                    data.len()
                );
                Ok(Response::new(proto::FetchChunkResponse {
                    data,
                    hash,
                    found: true,
                }))
            }
            Err(e) => Err(Status::internal(format!("Failed to read shard: {e}"))),
        }
    }
}

pub async fn start_background(state: AppState) {
    let addr = format!(
        "{}:{}",
        state.config.network.bind_address, state.config.network.grpc_port
    )
    .parse();

    let Ok(addr) = addr else {
        eprintln!("ERROR invalid gRPC bind address");
        return;
    };

    let svc = ServiceImpl { state };
    let raft_svc = RaftServiceImpl {
        state: svc.state.clone(),
    };
    tokio::spawn(async move {
        let result = tonic::transport::Server::builder()
            .add_service(proto::agent_service_server::AgentServiceServer::new(svc))
            .add_service(RaftServiceServer::new(raft_svc))
            .serve(addr)
            .await;

        if let Err(err) = result {
            eprintln!("ERROR gRPC server stopped: {err}");
        }
    });
}
