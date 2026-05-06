use crate::config::schema::Config;
use all4one_common::{ClusterState, NodeId, NodeInfo, NodeStatus};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration, Instant};

/// Response shape for GET /v1/nodes (we only need the node list).
#[derive(Deserialize)]
struct NodesResponse {
    nodes: Vec<NodeInfo>,
}

pub fn spawn_seed_discovery(
    config: Arc<Config>,
    self_id: NodeId,
    cluster: Arc<RwLock<ClusterState>>,
    last_seen: Arc<RwLock<HashMap<NodeId, Instant>>>,
) {
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        loop {
            for seed in &config.discovery.seeds {
                let rest_addr = seed_to_rest(seed);

                // Ask this seed for the full peer list it knows about.
                // A single reachable seed is enough to discover the whole cluster.
                let endpoint = format!("http://{rest_addr}/v1/nodes");
                let response = match client.get(&endpoint).send().await {
                    Ok(r) => r,
                    Err(_) => {
                        // Seed unreachable — fall back to its own identity only.
                        let fallback = format!("http://{rest_addr}/v1/internal/node");
                        match client.get(&fallback).send().await {
                            Ok(r) => {
                                if let Ok(node) = r.json::<NodeInfo>().await {
                                    if node.profile.id != self_id {
                                        upsert_peer(&cluster, &last_seen, node).await;
                                    }
                                }
                                continue;
                            }
                            Err(_) => continue,
                        }
                    }
                };

                let Ok(body) = response.json::<NodesResponse>().await else {
                    continue;
                };

                let now = Instant::now();
                let mut state = cluster.write().await;
                let mut seen = last_seen.write().await;
                for node in body.nodes {
                    if node.profile.id == self_id {
                        continue;
                    }
                    let id = node.profile.id;
                    let mut peer = node;
                    peer.status = NodeStatus::Online;
                    state.nodes.insert(id, peer);
                    seen.insert(id, now);
                }
                state.version = state.version.saturating_add(1);
            }
            sleep(Duration::from_secs(5)).await;
        }
    });
}

async fn upsert_peer(
    cluster: &Arc<RwLock<ClusterState>>,
    last_seen: &Arc<RwLock<HashMap<NodeId, Instant>>>,
    node: NodeInfo,
) {
    let id = node.profile.id;
    let now = Instant::now();
    let mut state = cluster.write().await;
    let mut peer = node;
    peer.status = NodeStatus::Online;
    state.nodes.insert(id, peer);
    state.version = state.version.saturating_add(1);
    drop(state);
    last_seen.write().await.insert(id, now);
}

fn seed_to_rest(seed: &str) -> String {
    if let Some((host, port_raw)) = seed.rsplit_once(':') {
        if let Ok(port) = port_raw.parse::<u16>() {
            if port > 0 {
                let rest_port = port.saturating_sub(1);
                return format!("{host}:{rest_port}");
            }
        }
    }
    seed.to_string()
}

pub async fn upsert_self(state: Arc<RwLock<ClusterState>>, local: NodeInfo) {
    let mut st = state.write().await;
    st.nodes.insert(local.profile.id, local);
    st.version = st.version.saturating_add(1);
}

pub async fn mark_self_heartbeat(
    last_seen: Arc<RwLock<HashMap<NodeId, Instant>>>,
    node_id: NodeId,
) {
    let mut seen = last_seen.write().await;
    seen.insert(node_id, Instant::now());
}
