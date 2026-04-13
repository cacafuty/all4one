use crate::config::schema::Config;
use crate::executor;
use crate::grpc_client;
use crate::scheduler::{self, SchedulingRequest};
use all4one_common::{
    ClusterState, JobId, JobResources, JobStatus, NodeId, NodeInfo, NodeProfile, NodeStatus, Runtime,
};
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{Request, StatusCode};
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

pub async fn serve(state: AppState) -> anyhow::Result<()> {
    let app = build_router(state.clone());
    let addr = format!("{}:{}", state.config.network.bind_address, state.config.network.rest_port);
    let listener = TcpListener::bind(&addr).await?;
    println!("INFO REST API listening on {addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

pub fn build_router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/v1/nodes", get(get_nodes))
        .route("/v1/jobs", post(post_job).get(list_jobs))
        .route("/v1/jobs/:id", get(get_job).delete(delete_job))
        .route("/v1/jobs/:id/output/stream", get(stream_output))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            shared_secret_middleware,
        ));

    let internal = Router::new().route("/v1/internal/node", get(get_internal_node));

    Router::new()
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

async fn get_internal_node(State(state): State<AppState>) -> Json<NodeInfo> {
    Json(state.local_node.clone())
}

async fn get_nodes(State(state): State<AppState>) -> Json<NodesResponse> {
    let st = state.cluster.read().await;
    let mut nodes: Vec<NodeInfo> = st.nodes.values().cloned().collect();
    nodes.sort_by_key(|n| n.profile.id.to_string());
    let online = nodes.iter().filter(|n| n.status == NodeStatus::Online).count();
    let offline = nodes.iter().filter(|n| n.status == NodeStatus::Offline).count();

    Json(NodesResponse {
        total: nodes.len(),
        online,
        offline,
        nodes,
    })
}

async fn post_job(State(state): State<AppState>, body: String) -> impl IntoResponse {
    let request: SubmitJobRequest = match serde_yaml::from_str(&body).or_else(|_| serde_json::from_str(&body)) {
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

    (
        StatusCode::ACCEPTED,
        Json(response),
    )
        .into_response()
}

pub async fn enqueue_job(state: AppState, request: SubmitJobRequest) -> Result<SubmitJobResponse, String> {
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

    if let Some(node_id) = assigned {
        let _ = dispatch_to_assigned(
            state.clone(),
            node_id,
            job_id,
            request.clone(),
        )
        .await;
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

async fn dispatch_to_assigned(
    state: AppState,
    assigned: NodeId,
    job_id: JobId,
    request: SubmitJobRequest,
) -> bool {
    if assigned == state.node_id {
        executor::spawn_job(
            job_id,
            request.runtime,
            request.source,
            request.command,
            state.jobs.clone(),
            state.output_channels.clone(),
        );
        return true;
    }

    let target = {
        let st = state.cluster.read().await;
        st.nodes.get(&assigned).map(|n| n.grpc_endpoint.clone())
    };

    if let Some(endpoint) = target {
        if grpc_client::submit_remote(&endpoint, &request).await.is_ok() {
            let mut jobs = state.jobs.write().await;
            if let Some(job) = jobs.get_mut(&job_id) {
                if job.status == JobStatus::Queued {
                    job.status = JobStatus::Running;
                    job.assigned_to = assigned;
                    job.updated_at = Utc::now();
                }
            }
            return true;
        }
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
            let Some(next_node) = scheduler::pick_node(&state.local_node, &cluster_snapshot, &scheduling) else {
                continue;
            };

            let dispatched = dispatch_to_assigned(state.clone(), next_node, job_id, request.clone()).await;
            if dispatched {
                return;
            }
        }
    });
}

async fn list_jobs(State(state): State<AppState>) -> Json<ListJobsResponse> {
    let jobs = state.jobs.read().await;
    let mut list: Vec<JobRecord> = jobs.values().cloned().collect();
    list.sort_by_key(|j| j.created_at);
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

    if matches!(existing.status, JobStatus::Completed | JobStatus::Failed | JobStatus::Cancelled) {
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
    (StatusCode::OK, Json(updated)).into_response()
}

async fn stream_output(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let parsed = match Uuid::parse_str(&id) {
        Ok(v) => JobId(v),
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                "invalid job id",
            )
                .into_response();
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

async fn shared_secret_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if state.config.security.mode != "dev" {
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

    if header != configured {
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
