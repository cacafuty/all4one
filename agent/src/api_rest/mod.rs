use crate::config::schema::Config;
use crate::executor;
use crate::grpc_client;
use crate::raft::{ClusterStatus, RaftNode};
use crate::scheduler::{self, SchedulingRequest};
use all4one_common::{
    ClusterState, JobId, JobResources, JobStatus, NodeId, NodeInfo, NodeProfile, NodeStatus,
    Runtime,
};
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, RwLock};
use tokio::time::{sleep, Duration};
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub started_at: Instant,
    pub started_at_utc: DateTime<Utc>,
    pub node_id: NodeId,
    pub profile: NodeProfile,
    pub local_node: NodeInfo,
    pub cluster: Arc<RwLock<ClusterState>>,
    pub last_seen: Arc<RwLock<HashMap<NodeId, tokio::time::Instant>>>,
    pub(crate) jobs: Arc<RwLock<HashMap<JobId, JobRecord>>>,
    pub(crate) output_channels: Arc<RwLock<HashMap<JobId, broadcast::Sender<String>>>>,
    pub(crate) ops_events: broadcast::Sender<OpsEvent>,
    pub raft: Option<RaftNode>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct OpsEvent {
    pub at: DateTime<Utc>,
    pub kind: String,
    pub level: String,
    pub message: String,
    pub node_id: Option<String>,
    pub job_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    node_id: String,
    uptime_seconds: u64,
    cluster_connected: bool,
    quorum_healthy: bool,
}

#[derive(Debug, Serialize)]
struct NodesResponse {
    nodes: Vec<NodeInfo>,
    total: usize,
    online: usize,
    offline: usize,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct JobRecord {
    pub(crate) job_id: JobId,
    pub(crate) status: JobStatus,
    pub(crate) assigned_to: NodeId,
    pub(crate) runtime: Runtime,
    pub(crate) source: String,
    pub(crate) command: Vec<String>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) exit_code: Option<i32>,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SubmitCapabilities {
    #[serde(default)]
    pub docker: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SubmitConstraints {
    #[serde(default)]
    pub tier_min: u8,
    #[serde(default)]
    pub requires_capabilities: SubmitCapabilities,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubmitJobRequest {
    pub runtime: Runtime,
    pub source: String,
    #[serde(default)]
    pub command: Vec<String>,
    pub resources: JobResources,
    #[serde(default)]
    pub constraints: SubmitConstraints,
}

#[derive(Debug, Serialize)]
pub struct SubmitJobResponse {
    pub job_id: JobId,
    pub status: JobStatus,
    pub assigned_to: NodeId,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct ListJobsResponse {
    jobs: Vec<JobRecord>,
    total: usize,
}

#[derive(Debug, Deserialize)]
struct ListJobsQuery {
    status: Option<String>,
    node_id: Option<String>,
    limit: Option<usize>,
}

/// Storage health diagnostics
#[derive(Debug, Serialize)]
struct StorageHealth {
    data_dir: String,
    accessible: bool,
    available_space_mb: Option<u64>,
    object_count: Option<u64>,
    error: Option<String>,
}

/// Distributed memory / state consistency check
#[derive(Debug, Serialize)]
struct DistributedStateHealth {
    raft_enabled: bool,
    raft_leader: Option<String>,
    raft_term: Option<u64>,
    consensus_nodes: usize,
    cluster_synchronized: bool,
    last_heartbeat_ms_ago: Option<u64>,
}

/// Enhanced cluster status with diagnostics
#[derive(Debug, Serialize)]
struct EnhancedClusterStatus {
    node_id: String,
    node_tier: u8,
    node_roles: NodeRolesInfo,
    uptime_seconds: u64,
    cluster_info: ClusterInfo,
    storage_health: Option<StorageHealth>,
    distributed_state: DistributedStateHealth,
}

#[derive(Debug, Serialize)]
struct NodeRolesInfo {
    scheduler: bool,
    executor: bool,
    storage: bool,
}

#[derive(Debug, Serialize)]
struct ClusterInfo {
    total_nodes: usize,
    online_nodes: usize,
    offline_nodes: usize,
    quorum_participant: bool,
    quorum_healthy: bool,
}

pub async fn serve(state: AppState) -> anyhow::Result<()> {
    start_ops_watchers(state.clone());
    let app = build_router(state.clone());
    let addr = format!(
        "{}:{}",
        state.config.network.bind_address, state.config.network.rest_port
    );
    let listener = TcpListener::bind(&addr).await?;
    println!("INFO REST API listening on {addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

pub fn build_router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/v1/nodes", get(get_nodes))
        .route("/v1/nodes/:id", get(get_node))
        .route("/v1/jobs", post(post_job).get(list_jobs))
        .route("/v1/jobs/:id", get(get_job).delete(delete_job))
        .route("/v1/jobs/:id/output/stream", get(stream_output))
        .route("/v1/ops/events", get(stream_ops_events))
        .route("/v1/cluster/status", get(cluster_status))
        .route("/v1/cluster/diagnostics", get(cluster_diagnostics))
        // Storage: external client (S3-like) interface
        .route(
            "/v1/storage/:bucket",
            get(list_bucket_objects).post(create_bucket),
        )
        .route(
            "/v1/storage/:bucket/*key",
            get(get_object_handler)
                .put(put_object_handler)
                .delete(delete_object_handler),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            shared_secret_middleware,
        ));

    let internal = Router::new()
        .route("/v1/internal/node", get(get_internal_node))
        .route("/v1/internal/nodes", get(get_internal_nodes));

    Router::new()
        .route("/", get(dashboard_page))
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .merge(internal)
        .merge(protected)
        .with_state(state)
        .layer(middleware::from_fn(request_id_middleware))
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let cluster = state.cluster.read().await;
    Json(HealthResponse {
        status: "ok",
        node_id: state.node_id.to_string(),
        uptime_seconds: state.started_at.elapsed().as_secs(),
        cluster_connected: !cluster.nodes.is_empty(),
        quorum_healthy: state.config.node.quorum_participant,
    })
}

async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    let cluster = state.cluster.read().await;
    let online = cluster
        .nodes
        .values()
        .filter(|n| n.status == NodeStatus::Online)
        .count();
    let jobs = state.jobs.read().await;
    (
        StatusCode::OK,
        format!(
            "all4one_uptime_seconds {}\nall4one_nodes_online {}\nall4one_jobs_total {}\n",
            state.started_at.elapsed().as_secs(),
            online,
            jobs.len()
        ),
    )
}

async fn cluster_status(State(state): State<AppState>) -> Json<ClusterStatus> {
    let (raft_leader, quorum_healthy, raft_term, last_log_index) =
        if let Some(ref raft) = state.raft {
            let m = raft.current_metrics();
            let leader = m.current_leader.map(|id| id.to_string());
            let healthy = m.current_leader.is_some();
            let term = m.current_term;
            let last_idx = m.last_log_index;
            (leader, healthy, term, last_idx)
        } else {
            (None, false, 0, None)
        };

    Json(ClusterStatus {
        node_id: state.node_id.to_string(),
        quorum_participant: state.config.node.quorum_participant,
        raft_leader,
        quorum_healthy,
        raft_term,
        last_log_index,
    })
}

async fn cluster_diagnostics(State(state): State<AppState>) -> Json<EnhancedClusterStatus> {
    let cluster = state.cluster.read().await;

    let (raft_leader, raft_term, cluster_synchronized) = if let Some(ref raft) = state.raft {
        let m = raft.current_metrics();
        let leader: Option<String> = m.current_leader.map(|id| id.to_string());
        let term = m.current_term;
        let synchronized = leader.is_some();
        (leader, Some(term), synchronized)
    } else {
        (None, None, false)
    };

    let online = cluster
        .nodes
        .values()
        .filter(|n| n.status == NodeStatus::Online)
        .count();
    let offline = cluster.nodes.len() - online;

    let storage_health = if state.config.roles.storage {
        Some(check_storage_health(&state.config.node.data_dir).await)
    } else {
        None
    };

    Json(EnhancedClusterStatus {
        node_id: state.node_id.to_string(),
        node_tier: state.config.node.tier,
        node_roles: NodeRolesInfo {
            scheduler: state.config.roles.scheduler,
            executor: state.config.roles.executor,
            storage: state.config.roles.storage,
        },
        uptime_seconds: state.started_at.elapsed().as_secs(),
        cluster_info: ClusterInfo {
            total_nodes: cluster.nodes.len(),
            online_nodes: online,
            offline_nodes: offline,
            quorum_participant: state.config.node.quorum_participant,
            quorum_healthy: raft_leader.is_some(),
        },
        storage_health,
        distributed_state: DistributedStateHealth {
            raft_enabled: state.raft.is_some(),
            raft_leader,
            raft_term,
            consensus_nodes: cluster.nodes.len(),
            cluster_synchronized,
            last_heartbeat_ms_ago: None, // OpenRaft metrics doesn't provide this directly
        },
    })
}

async fn check_storage_health(data_dir: &str) -> StorageHealth {
    use std::path::Path;

    let path = Path::new(data_dir);
    let accessible = path.exists() && path.is_dir();

    let available_space = if accessible {
        // Try to get disk space info
        match std::fs::metadata(data_dir) {
            Ok(_) => {
                // Estimate available space (this is a placeholder)
                Some(1000) // Should use proper disk space API
            }
            Err(_) => None,
        }
    } else {
        None
    };

    let object_count = if accessible {
        match std::fs::read_dir(path) {
            Ok(entries) => Some(entries.count() as u64),
            Err(_) => None,
        }
    } else {
        None
    };

    StorageHealth {
        data_dir: data_dir.to_string(),
        accessible,
        available_space_mb: available_space,
        object_count,
        error: if !accessible {
            Some("Data directory not accessible".to_string())
        } else {
            None
        },
    }
}

// ── Storage response types ────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct StorageErrorResponse {
    error: String,
}

#[derive(Debug, Deserialize)]
struct ListObjectsQuery {
    prefix: Option<String>,
    max_keys: Option<usize>,
}

// ── Storage handlers ─────────────────────────────────────────────────────────

/// POST /v1/storage/:bucket  — create a bucket (idempotent)
async fn create_bucket(
    State(state): State<AppState>,
    Path(bucket): Path<String>,
) -> impl IntoResponse {
    if !state.config.roles.storage {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(StorageErrorResponse {
                error: "This node does not have the storage role".to_string(),
            }),
        )
            .into_response();
    }
    let data_dir = &state.config.node.data_dir;
    match crate::storage::index::create_bucket(std::path::Path::new(data_dir), &bucket).await {
        Ok(_) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "bucket": bucket, "created": true })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(StorageErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// GET /v1/storage/:bucket  — list objects in bucket
async fn list_bucket_objects(
    State(state): State<AppState>,
    Path(bucket): Path<String>,
    Query(params): Query<ListObjectsQuery>,
) -> impl IntoResponse {
    if !state.config.roles.storage {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(StorageErrorResponse {
                error: "This node does not have the storage role".to_string(),
            }),
        )
            .into_response();
    }
    let data_dir = &state.config.node.data_dir;
    match crate::storage::list_objects(data_dir, &bucket, params.prefix.as_deref(), params.max_keys)
        .await
    {
        Ok(objects) => Json(
            serde_json::json!({ "bucket": bucket, "objects": objects, "count": objects.len() }),
        )
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(StorageErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// PUT /v1/storage/:bucket/*key  — upload an object
/// Header: X-All4One-Policy = hot | warm | cold | archive  (default: warm)
async fn put_object_handler(
    State(state): State<AppState>,
    Path((bucket, key)): Path<(String, String)>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    if !state.config.roles.storage {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(StorageErrorResponse {
                error: "This node does not have the storage role".to_string(),
            }),
        )
            .into_response();
    }

    let policy_str = headers
        .get("x-all4one-policy")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("warm");
    let policy = match policy_str {
        "hot" => crate::storage::StoragePolicy::Hot,
        "cold" => crate::storage::StoragePolicy::Cold,
        "archive" => crate::storage::StoragePolicy::Archive,
        _ => crate::storage::StoragePolicy::Warm,
    };

    let data_dir = &state.config.node.data_dir;

    // Ensure bucket exists
    let _ = crate::storage::index::create_bucket(std::path::Path::new(data_dir), &bucket).await;

    let meta = match crate::storage::put_object(data_dir, &bucket, &key, &body, policy).await {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(StorageErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response();
        }
    };

    // Fan-out shards to peer storage nodes asynchronously (fire-and-forget)
    {
        let cluster = state.cluster.read().await;
        let peers: Vec<String> = cluster
            .nodes
            .values()
            .filter(|n| {
                n.profile.capabilities.storage_node
                    && n.status == all4one_common::NodeStatus::Online
                    && n.profile.id != state.node_id
            })
            .map(|n| n.grpc_endpoint.clone())
            .collect();
        drop(cluster);

        if !peers.is_empty() {
            let data_dir_clone = data_dir.clone();
            let bucket_clone = bucket.clone();
            let key_clone = key.clone();
            let meta_clone = meta.clone();
            let policy_str = meta_clone.policy.clone();
            let etag = meta_clone.etag.clone();
            let size = meta_clone.size_bytes;

            tokio::spawn(async move {
                match crate::storage::read_shards(&data_dir_clone, &bucket_clone, &key_clone).await
                {
                    Ok(shards) => {
                        for peer in &peers {
                            for (idx, hash, data) in &shards {
                                if let Err(e) = crate::grpc_client::transfer_chunk(
                                    peer,
                                    &bucket_clone,
                                    &key_clone,
                                    *idx,
                                    data.clone(),
                                    hash.clone(),
                                    &policy_str,
                                    size,
                                    &etag,
                                )
                                .await
                                {
                                    println!(
                                        "WARN shard transfer failed peer={} shard={} err={}",
                                        peer, idx, e
                                    );
                                }
                            }
                        }
                    }
                    Err(e) => {
                        println!("WARN read_shards failed for distribution: {}", e);
                    }
                }
            });
        }
    }

    (StatusCode::CREATED, Json(meta)).into_response()
}

/// GET /v1/storage/:bucket/*key  — download an object
async fn get_object_handler(
    State(state): State<AppState>,
    Path((bucket, key)): Path<(String, String)>,
) -> impl IntoResponse {
    if !state.config.roles.storage {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(StorageErrorResponse {
                error: "This node does not have the storage role".to_string(),
            }),
        )
            .into_response();
    }

    let data_dir = &state.config.node.data_dir;
    match crate::storage::get_object(data_dir, &bucket, &key).await {
        Ok(data) => {
            let etag =
                crate::storage::index::get_object(std::path::Path::new(data_dir), &bucket, &key)
                    .await
                    .map(|m| m.etag)
                    .unwrap_or_default();

            axum::http::Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/octet-stream")
                .header("ETag", etag)
                .body(Body::from(data))
                .unwrap()
                .into_response()
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") || msg.contains("Object not found") {
                (
                    StatusCode::NOT_FOUND,
                    Json(StorageErrorResponse { error: msg }),
                )
                    .into_response()
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(StorageErrorResponse { error: msg }),
                )
                    .into_response()
            }
        }
    }
}

/// DELETE /v1/storage/:bucket/*key  — delete an object
async fn delete_object_handler(
    State(state): State<AppState>,
    Path((bucket, key)): Path<(String, String)>,
) -> impl IntoResponse {
    if !state.config.roles.storage {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(StorageErrorResponse {
                error: "This node does not have the storage role".to_string(),
            }),
        )
            .into_response();
    }

    let data_dir = &state.config.node.data_dir;
    match crate::storage::delete_object(data_dir, &bucket, &key).await {
        Ok(_) => (StatusCode::NO_CONTENT).into_response(),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") || msg.contains("Object not found") {
                (
                    StatusCode::NOT_FOUND,
                    Json(StorageErrorResponse { error: msg }),
                )
                    .into_response()
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(StorageErrorResponse { error: msg }),
                )
                    .into_response()
            }
        }
    }
}

async fn dashboard_page() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("Content-Type", "text/html; charset=utf-8")],
        include_str!("./dashboard.html"),
    )
}

fn publish_ops_event(
    state: &AppState,
    kind: &str,
    level: &str,
    message: String,
    node_id: Option<NodeId>,
    job_id: Option<JobId>,
) {
    let _ = state.ops_events.send(OpsEvent {
        at: Utc::now(),
        kind: kind.to_string(),
        level: level.to_string(),
        message,
        node_id: node_id.map(|id| id.to_string()),
        job_id: job_id.map(|id| id.to_string()),
    });
}

pub fn start_ops_watchers(state: AppState) {
    tokio::spawn(async move {
        let mut last_status: HashMap<NodeId, NodeStatus> = {
            let snapshot = state.cluster.read().await;
            snapshot
                .nodes
                .iter()
                .map(|(id, info)| (*id, info.status.clone()))
                .collect()
        };

        loop {
            sleep(Duration::from_secs(2)).await;

            let snapshot = state.cluster.read().await;
            let current: HashMap<NodeId, NodeStatus> = snapshot
                .nodes
                .iter()
                .map(|(id, info)| (*id, info.status.clone()))
                .collect();

            for (node_id, status) in &current {
                match last_status.get(node_id) {
                    Some(prev) if prev != status => {
                        publish_ops_event(
                            &state,
                            "node.status_changed",
                            if *status == NodeStatus::Online {
                                "info"
                            } else {
                                "warn"
                            },
                            format!("Node {} changed status {:?} -> {:?}", node_id, prev, status),
                            Some(*node_id),
                            None,
                        );
                    }
                    None => {
                        publish_ops_event(
                            &state,
                            "node.discovered",
                            "info",
                            format!("Node {} discovered with status {:?}", node_id, status),
                            Some(*node_id),
                            None,
                        );
                    }
                    _ => {}
                }
            }

            for node_id in last_status.keys() {
                if !current.contains_key(node_id) {
                    publish_ops_event(
                        &state,
                        "node.removed",
                        "warn",
                        format!("Node {} removed from cluster view", node_id),
                        Some(*node_id),
                        None,
                    );
                }
            }

            last_status = current;
        }
    });
}

async fn get_internal_node(State(state): State<AppState>) -> Json<NodeInfo> {
    Json(state.local_node.clone())
}

/// Minimal peer info returned by the unauthenticated /v1/internal/nodes endpoint.
/// Only exposes what a joining node needs for discovery — no capabilities or resources.
#[derive(Debug, Serialize)]
struct PeerInfo {
    id: String,
    tier: u8,
    grpc_endpoint: String,
    rest_endpoint: String,
    status: NodeStatus,
}

#[derive(Debug, Serialize)]
struct PeerListResponse {
    peers: Vec<PeerInfo>,
}

async fn get_internal_nodes(State(state): State<AppState>) -> Json<PeerListResponse> {
    let st = state.cluster.read().await;

    println!(
        "DEBUG [/v1/internal/nodes] Cluster has {} nodes",
        st.nodes.len()
    );

    let mut peers: Vec<PeerInfo> = st
        .nodes
        .values()
        .map(|n| {
            println!(
                "DEBUG [/v1/internal/nodes] Node: id={} tier={} status={:?}",
                n.profile.id, n.profile.tier, n.status
            );
            PeerInfo {
                id: n.profile.id.to_string(),
                tier: n.profile.tier,
                grpc_endpoint: n.grpc_endpoint.clone(),
                rest_endpoint: n.rest_endpoint.clone(),
                status: n.status.clone(),
            }
        })
        .collect();
    peers.sort_by_key(|p| p.id.clone());
    Json(PeerListResponse { peers })
}

async fn get_nodes(State(state): State<AppState>) -> Json<NodesResponse> {
    let st = state.cluster.read().await;
    let mut nodes: Vec<NodeInfo> = st.nodes.values().cloned().collect();
    nodes.sort_by_key(|n| n.profile.id.to_string());
    let online = nodes
        .iter()
        .filter(|n| n.status == NodeStatus::Online)
        .count();
    let offline = nodes
        .iter()
        .filter(|n| n.status == NodeStatus::Offline)
        .count();

    Json(NodesResponse {
        total: nodes.len(),
        online,
        offline,
        nodes,
    })
}

async fn get_node(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let parsed = match Uuid::parse_str(&id) {
        Ok(v) => NodeId(v),
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "invalid node id"})),
            )
                .into_response()
        }
    };

    let st = state.cluster.read().await;
    if let Some(node) = st.nodes.get(&parsed) {
        return (StatusCode::OK, Json(node)).into_response();
    }

    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": "node not found"})),
    )
        .into_response()
}

async fn post_job(State(state): State<AppState>, body: String) -> impl IntoResponse {
    let request: SubmitJobRequest =
        match serde_yaml::from_str(&body).or_else(|_| serde_json::from_str(&body)) {
            Ok(v) => v,
            Err(err) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": format!("invalid job payload: {err}")
                    })),
                )
                    .into_response()
            }
        };

    let response = match enqueue_job(state, request).await {
        Ok(resp) => resp,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": err})),
            )
                .into_response()
        }
    };

    (StatusCode::ACCEPTED, Json(response)).into_response()
}

pub async fn enqueue_job(
    state: AppState,
    request: SubmitJobRequest,
) -> Result<SubmitJobResponse, String> {
    if request.resources.cpu_cores == 0 || request.resources.memory_mb == 0 {
        return Err("resources.cpu_cores and resources.memory_mb must be > 0".to_string());
    }

    let cluster_snapshot = state.cluster.read().await.clone();
    let scheduling = SchedulingRequest {
        runtime: request.runtime.clone(),
        tier_min: request.constraints.tier_min,
        require_docker: request.constraints.requires_capabilities.docker,
    };
    let assigned = scheduler::pick_node(&state.local_node, &cluster_snapshot, &scheduling);

    let now = Utc::now();
    let job_id = JobId::new();
    let record = JobRecord {
        job_id,
        status: JobStatus::Queued,
        assigned_to: assigned.unwrap_or(state.node_id),
        runtime: request.runtime.clone(),
        source: request.source.clone(),
        command: request.command.clone(),
        created_at: now,
        updated_at: now,
        exit_code: None,
        error: None,
    };

    {
        let mut jobs = state.jobs.write().await;
        jobs.insert(job_id, record);
    }

    println!(
        "INFO Job queued id={} runtime={:?} tier_min={} require_docker={} assigned_to={}",
        job_id,
        request.runtime,
        request.constraints.tier_min,
        request.constraints.requires_capabilities.docker,
        assigned.unwrap_or(state.node_id),
    );

    if let Some(node_id) = assigned {
        let _ = dispatch_to_assigned(state.clone(), node_id, job_id, request.clone()).await;
    } else {
        spawn_retry_dispatch(state.clone(), job_id, request.clone(), scheduling);
    }

    Ok(SubmitJobResponse {
        job_id,
        status: JobStatus::Queued,
        assigned_to: assigned.unwrap_or(state.node_id),
        created_at: now,
    })
}

pub async fn enqueue_remote_job(
    state: AppState,
    job_id: JobId,
    request: SubmitJobRequest,
    origin_endpoint: Option<String>,
) -> Result<SubmitJobResponse, String> {
    if request.resources.cpu_cores == 0 || request.resources.memory_mb == 0 {
        return Err("resources.cpu_cores and resources.memory_mb must be > 0".to_string());
    }

    let now = Utc::now();
    let record = JobRecord {
        job_id,
        status: JobStatus::Queued,
        assigned_to: state.node_id,
        runtime: request.runtime.clone(),
        source: request.source.clone(),
        command: request.command.clone(),
        created_at: now,
        updated_at: now,
        exit_code: None,
        error: None,
    };

    {
        let mut jobs = state.jobs.write().await;
        jobs.insert(job_id, record);
    }

    println!(
        "INFO Remote job queued id={} runtime={:?} origin_endpoint={}",
        job_id,
        request.runtime,
        origin_endpoint.as_deref().unwrap_or("<none>"),
    );

    let callback = origin_endpoint.map(|endpoint| executor::JobCompletionCallback {
        origin_endpoint: endpoint,
        source_node_id: state.node_id,
    });

    executor::spawn_job(
        job_id,
        request.runtime,
        request.source,
        request.command,
        request.resources,
        state.jobs.clone(),
        state.output_channels.clone(),
        callback,
    );

    Ok(SubmitJobResponse {
        job_id,
        status: JobStatus::Queued,
        assigned_to: state.node_id,
        created_at: now,
    })
}

pub async fn apply_terminal_job_update(
    state: AppState,
    job_id: JobId,
    status: JobStatus,
    exit_code: Option<i32>,
    error: Option<String>,
) -> Result<(), String> {
    let mut jobs = state.jobs.write().await;
    let Some(job) = jobs.get_mut(&job_id) else {
        return Err(format!("job {job_id} not found on origin node"));
    };

    job.status = status.clone();
    job.exit_code = exit_code;
    job.error = error;
    job.updated_at = Utc::now();

    println!(
        "INFO Job terminal status updated from remote id={} status={:?} assigned_to={} exit_code={:?}",
        job_id,
        status,
        job.assigned_to,
        job.exit_code,
    );

    let level = if status == JobStatus::Failed {
        "error"
    } else {
        "info"
    };
    publish_ops_event(
        &state,
        "job.terminal",
        level,
        format!("Job {} updated to terminal status {:?}", job_id, status),
        Some(job.assigned_to),
        Some(job_id),
    );

    Ok(())
}

async fn dispatch_to_assigned(
    state: AppState,
    assigned: NodeId,
    job_id: JobId,
    request: SubmitJobRequest,
) -> bool {
    if assigned == state.node_id {
        println!(
            "INFO Job dispatch local id={} runtime={:?} node_id={}",
            job_id, request.runtime, state.node_id,
        );
        executor::spawn_job(
            job_id,
            request.runtime,
            request.source,
            request.command,
            request.resources,
            state.jobs.clone(),
            state.output_channels.clone(),
            None,
        );
        publish_ops_event(
            &state,
            "job.started",
            "info",
            format!("Job {} started on local node {}", job_id, state.node_id),
            Some(state.node_id),
            Some(job_id),
        );
        return true;
    }

    let target = {
        let st = state.cluster.read().await;
        st.nodes.get(&assigned).map(|n| n.grpc_endpoint.clone())
    };

    if let Some(endpoint) = target {
        println!(
            "INFO Job dispatch remote id={} runtime={:?} target_node={} endpoint={}",
            job_id, request.runtime, assigned, endpoint,
        );

        if grpc_client::submit_remote(&endpoint, job_id, &state.local_node.grpc_endpoint, &request)
            .await
            .is_ok()
        {
            let mut jobs = state.jobs.write().await;
            if let Some(job) = jobs.get_mut(&job_id) {
                if job.status == JobStatus::Queued {
                    job.status = JobStatus::Running;
                    job.assigned_to = assigned;
                    job.updated_at = Utc::now();
                }
            }
            println!(
                "INFO Job dispatch remote accepted id={} target_node={}",
                job_id, assigned,
            );
            publish_ops_event(
                &state,
                "job.dispatched",
                "info",
                format!("Job {} dispatched to node {}", job_id, assigned),
                Some(assigned),
                Some(job_id),
            );
            return true;
        }

        println!(
            "WARN Job dispatch remote failed id={} target_node={} endpoint={}",
            job_id, assigned, endpoint,
        );
        publish_ops_event(
            &state,
            "job.dispatch_failed",
            "warn",
            format!("Job {} dispatch to node {} failed", job_id, assigned),
            Some(assigned),
            Some(job_id),
        );
    }

    false
}

fn spawn_retry_dispatch(
    state: AppState,
    job_id: JobId,
    request: SubmitJobRequest,
    scheduling: SchedulingRequest,
) {
    tokio::spawn(async move {
        for _ in 0..60 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;

            {
                let jobs = state.jobs.read().await;
                let Some(job) = jobs.get(&job_id) else {
                    return;
                };
                if job.status != JobStatus::Queued {
                    return;
                }
            }

            let cluster_snapshot = state.cluster.read().await.clone();
            let Some(next_node) =
                scheduler::pick_node(&state.local_node, &cluster_snapshot, &scheduling)
            else {
                continue;
            };

            println!(
                "INFO Job retry dispatch id={} next_node={} runtime={:?}",
                job_id, next_node, request.runtime,
            );

            let dispatched =
                dispatch_to_assigned(state.clone(), next_node, job_id, request.clone()).await;
            if dispatched {
                return;
            }
        }
    });
}

async fn list_jobs(
    State(state): State<AppState>,
    Query(query): Query<ListJobsQuery>,
) -> Json<ListJobsResponse> {
    let jobs = state.jobs.read().await;
    let mut list: Vec<JobRecord> = jobs.values().cloned().collect();
    if let Some(status) = query.status {
        let expected = status.to_lowercase();
        list.retain(|j| format!("{:?}", j.status).to_lowercase() == expected);
    }

    if let Some(node_id) = query.node_id {
        list.retain(|j| j.assigned_to.to_string() == node_id);
    }

    list.sort_by_key(|j| j.created_at);

    if let Some(limit) = query.limit {
        if list.len() > limit {
            let keep_from = list.len() - limit;
            list = list.split_off(keep_from);
        }
    }

    Json(ListJobsResponse {
        total: list.len(),
        jobs: list,
    })
}

async fn get_job(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let parsed = match uuid::Uuid::parse_str(&id) {
        Ok(v) => JobId(v),
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "invalid job id"})),
            )
                .into_response()
        }
    };

    let jobs = state.jobs.read().await;
    if let Some(job) = jobs.get(&parsed) {
        return (StatusCode::OK, Json(job)).into_response();
    }

    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": "job not found"})),
    )
        .into_response()
}

async fn delete_job(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let parsed = match uuid::Uuid::parse_str(&id) {
        Ok(v) => JobId(v),
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "invalid job id"})),
            )
                .into_response()
        }
    };

    let mut jobs = state.jobs.write().await;
    let Some(existing) = jobs.get(&parsed) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "job not found"})),
        )
            .into_response();
    };

    if matches!(
        existing.status,
        JobStatus::Completed | JobStatus::Failed | JobStatus::Cancelled
    ) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "job already finished"})),
        )
            .into_response();
    }

    let mut updated = existing.clone();
    updated.status = JobStatus::Cancelled;
    updated.updated_at = Utc::now();
    jobs.insert(parsed, updated.clone());
    publish_ops_event(
        &state,
        "job.cancelled",
        "warn",
        format!("Job {} was cancelled", parsed),
        Some(updated.assigned_to),
        Some(parsed),
    );
    (StatusCode::OK, Json(updated)).into_response()
}

async fn stream_output(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let parsed = match Uuid::parse_str(&id) {
        Ok(v) => JobId(v),
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "invalid job id").into_response();
        }
    };

    let sender = {
        let mut channels = state.output_channels.write().await;
        channels
            .entry(parsed)
            .or_insert_with(|| {
                let (tx, _rx) = broadcast::channel(256);
                tx
            })
            .clone()
    };

    let stream = BroadcastStream::new(sender.subscribe()).filter_map(|msg| async move {
        match msg {
            Ok(line) => Some(Ok::<Event, Infallible>(Event::default().data(line))),
            Err(_) => None,
        }
    });

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

async fn stream_ops_events(State(state): State<AppState>) -> impl IntoResponse {
    let stream = BroadcastStream::new(state.ops_events.subscribe()).filter_map(|msg| async move {
        match msg {
            Ok(event) => {
                let payload = serde_json::to_string(&event).ok()?;
                Some(Ok::<Event, Infallible>(
                    Event::default().event("ops").data(payload),
                ))
            }
            Err(_) => None,
        }
    });

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

async fn shared_secret_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if state.config.security.mode != "shared-secret" {
        return next.run(request).await;
    }

    let path = request.uri().path();
    if path.starts_with("/v1/internal/") {
        return next.run(request).await;
    }

    let configured = state.config.security.shared_secret.trim();
    if configured.is_empty() {
        return next.run(request).await;
    }

    let header = request
        .headers()
        .get("X-All4One-Secret")
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();

    let query_secret = request
        .uri()
        .query()
        .and_then(|q| {
            q.split('&').find_map(|pair| {
                let mut parts = pair.splitn(2, '=');
                let key = parts.next()?;
                let value = parts.next().unwrap_or_default();
                if key == "secret" {
                    Some(value)
                } else {
                    None
                }
            })
        })
        .unwrap_or_default();

    if header != configured && query_secret != configured {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }

    next.run(request).await
}

async fn request_id_middleware(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    let request_id = Uuid::new_v4().to_string();
    if let Ok(value) = request_id.parse() {
        response.headers_mut().insert("X-Request-Id", value);
    }
    response
}
