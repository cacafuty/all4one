use crate::config::schema::Config;
use all4one_common::{ClusterState, NodeId, NodeInfo, NodeStatus};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration, Instant};
use uuid::Uuid;

/// Minimal peer info returned by GET /v1/internal/nodes (unauthenticated).
#[derive(Deserialize)]
struct PeerInfo {
    id: String,
    tier: u8,
    grpc_endpoint: String,
    rest_endpoint: String,
}

#[derive(Deserialize)]
struct PeerListResponse {
    peers: Vec<PeerInfo>,
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
            // Discovery phase 1: Poll configured seeds
            for seed in &config.discovery.seeds {
                let rest_addr = seed_to_rest(seed);

                // Ask this seed for the full peer list it knows about.
                // A single reachable seed is enough to discover the whole cluster.
                let endpoint = format!("http://{rest_addr}/v1/internal/nodes");
                let response = match client.get(&endpoint).send().await {
                    Ok(r) if r.status().is_success() => r,
                    // 404 = older agent without /internal/nodes; connection error = unreachable.
                    // Either way fall back to the single-node identity endpoint.
                    _ => {
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

                let Ok(body) = response.json::<PeerListResponse>().await else {
                    continue;
                };

                let now = Instant::now();
                let mut state = cluster.write().await;
                let mut seen = last_seen.write().await;
                for peer in body.peers {
                    let Ok(uuid) = Uuid::parse_str(&peer.id) else {
                        continue;
                    };
                    let id = NodeId(uuid);
                    if id == self_id {
                        continue;
                    }
                    // Only update endpoint info; preserve existing profile/capabilities if known.
                    state
                        .nodes
                        .entry(id)
                        .and_modify(|n| {
                            n.grpc_endpoint = peer.grpc_endpoint.clone();
                            n.rest_endpoint = peer.rest_endpoint.clone();
                            n.status = NodeStatus::Online;
                        })
                        .or_insert_with(|| NodeInfo {
                            profile: all4one_common::NodeProfile {
                                id,
                                tier: peer.tier,
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
                                    storage_node: false,
                                },
                            },
                            status: NodeStatus::Online,
                            version: String::new(),
                            grpc_endpoint: peer.grpc_endpoint,
                            rest_endpoint: peer.rest_endpoint,
                        });
                    seen.insert(id, now);
                }
                state.version = state.version.saturating_add(1);
            }

            // Discovery phase 2: Reflexively discover through known nodes (bootstrap-free discovery)
            // When a node joins via enrollment without seeds, other nodes discover it by
            // asking nodes they already know about. This creates a self-healing mesh without
            // requiring seeds to be configured.
            let known_nodes: Vec<(NodeId, String)> = {
                let state = cluster.read().await;
                state
                    .nodes
                    .iter()
                    .filter(|(id, _)| *id != &self_id)
                    .map(|(id, info)| (*id, info.rest_endpoint.clone()))
                    .collect()
            };

            for (_node_id, rest_endpoint) in known_nodes {
                if rest_endpoint.is_empty() {
                    continue;
                }
                let endpoint = format!("http://{rest_endpoint}/v1/internal/nodes");
                let response = match client.get(&endpoint).send().await {
                    Ok(r) if r.status().is_success() => r,
                    _ => continue,
                };

                let Ok(body) = response.json::<PeerListResponse>().await else {
                    continue;
                };

                let now = Instant::now();
                let mut state = cluster.write().await;
                let mut seen = last_seen.write().await;
                for peer in body.peers {
                    let Ok(uuid) = Uuid::parse_str(&peer.id) else {
                        continue;
                    };
                    let id = NodeId(uuid);
                    if id == self_id {
                        continue;
                    }
                    state
                        .nodes
                        .entry(id)
                        .and_modify(|n| {
                            n.grpc_endpoint = peer.grpc_endpoint.clone();
                            n.rest_endpoint = peer.rest_endpoint.clone();
                            n.status = NodeStatus::Online;
                        })
                        .or_insert_with(|| NodeInfo {
                            profile: all4one_common::NodeProfile {
                                id,
                                tier: peer.tier,
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
                                    storage_node: false,
                                },
                            },
                            status: NodeStatus::Online,
                            version: String::new(),
                            grpc_endpoint: peer.grpc_endpoint,
                            rest_endpoint: peer.rest_endpoint,
                        });
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
