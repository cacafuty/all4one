use all4one_common::{ClusterState, NodeId, NodeStatus};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration, Instant};

pub fn spawn_failure_detector(
    self_id: NodeId,
    cluster: Arc<RwLock<ClusterState>>,
    last_seen: Arc<RwLock<HashMap<NodeId, Instant>>>,
) {
    tokio::spawn(async move {
        loop {
            let now = Instant::now();
            let seen_snapshot = { last_seen.read().await.clone() };
            let mut state = cluster.write().await;

            for (id, node) in &mut state.nodes {
                if *id == self_id {
                    node.status = NodeStatus::Online;
                    continue;
                }

                let status = if let Some(last) = seen_snapshot.get(id) {
                    let age = now.saturating_duration_since(*last);
                    if age >= Duration::from_secs(100) {
                        NodeStatus::Offline
                    } else if age >= Duration::from_secs(35) {
                        NodeStatus::Suspected
                    } else {
                        NodeStatus::Online
                    }
                } else {
                    NodeStatus::Suspected
                };

                node.status = status;
            }
            state.version = state.version.saturating_add(1);
            drop(state);

            sleep(Duration::from_secs(5)).await;
        }
    });
}
