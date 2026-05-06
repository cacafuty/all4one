use crate::config::schema::Config;
use all4one_common::{ClusterState, NodeId, NodeInfo, NodeStatus};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration, Instant};

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
                let endpoint = format!("http://{}/v1/internal/node", seed_to_rest(seed));
                let response = client.get(&endpoint).send().await;
                let Ok(response) = response else {
                    continue;
                };
                let Ok(node) = response.json::<NodeInfo>().await else {
                    continue;
                };
                if node.profile.id == self_id {
                    continue;
                }

                {
                    let mut state = cluster.write().await;
                    let mut discovered = node.clone();
                    discovered.status = NodeStatus::Online;
                    state.nodes.insert(discovered.profile.id, discovered);
                    state.version = state.version.saturating_add(1);
                }
                {
                    let mut seen = last_seen.write().await;
                    seen.insert(node.profile.id, Instant::now());
                }
            }
            sleep(Duration::from_secs(5)).await;
        }
    });
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
