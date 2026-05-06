pub mod commands;
pub mod network;
pub mod store;

use crate::raft::commands::{RaftCommand, RaftCommandResponse};
use crate::raft::network::GrpcNetworkFactory;
use crate::raft::store::{SledLogStore, SledStateMachine};
use all4one_common::NodeId;
use openraft::{BasicNode, Config, Raft, RaftMetrics};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Type aliases
// ---------------------------------------------------------------------------

pub type RaftNodeId = NodeId;

openraft::declare_raft_types!(
    pub TypeConfig:
        D   = RaftCommand,
        R   = RaftCommandResponse,
        NodeId = RaftNodeId,
        Node = BasicNode,
        Entry = openraft::Entry<TypeConfig>,
        SnapshotData = std::io::Cursor<Vec<u8>>,
        AsyncRuntime = openraft::TokioRuntime,
);

// ---------------------------------------------------------------------------
// Public handle stored in AppState
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct RaftNode {
    pub raft: Raft<TypeConfig>,
    pub node_id: RaftNodeId,
}

impl RaftNode {
    /// Submit a command to the Raft cluster.
    /// Returns an error if this node is not the leader.
    pub async fn apply_command(
        &self,
        cmd: RaftCommand,
    ) -> anyhow::Result<RaftCommandResponse> {
        self.raft
            .client_write(cmd)
            .await
            .map(|r| r.data)
            .map_err(|e| anyhow::anyhow!("raft write: {}", e))
    }

    /// Snapshot of current Raft metrics (non-blocking).
    pub fn current_metrics(&self) -> RaftMetrics<RaftNodeId, BasicNode> {
        self.raft.metrics().borrow().clone()
    }
}

// Cluster status suitable for the REST response
#[derive(Debug, Serialize, Deserialize)]
pub struct ClusterStatus {
    pub node_id: String,
    pub quorum_participant: bool,
    pub raft_leader: Option<String>,
    pub quorum_healthy: bool,
    pub raft_term: u64,
    pub last_log_index: Option<u64>,
}

// ---------------------------------------------------------------------------
// Initialisation
// ---------------------------------------------------------------------------

/// Creates and starts a `Raft` instance backed by sled storage.
///
/// * `node_id`  — this node's ID
/// * `data_dir` — directory for sled databases
/// * `grpc_endpoint` — advertised gRPC address for this node
/// * `initial_peers` — (node_id, grpc_endpoint) of *other* quorum members
///   known at bootstrap time.  May be empty for single-node clusters.
pub async fn init_raft(
    node_id: RaftNodeId,
    data_dir: &str,
    grpc_endpoint: &str,
    initial_peers: Vec<(RaftNodeId, String)>,
) -> anyhow::Result<RaftNode> {
    let config = Arc::new(
        Config {
            heartbeat_interval: 500,
            election_timeout_min: 1500,
            election_timeout_max: 3000,
            ..Default::default()
        }
        .validate()
        .map_err(|e| anyhow::anyhow!("raft config: {}", e))?,
    );

    let path = Path::new(data_dir);
    let log_store = SledLogStore::open(path)?;
    let state_machine = SledStateMachine::open(path)?;

    let raft = Raft::new(node_id, config, GrpcNetworkFactory, log_store, state_machine)
        .await
        .map_err(|e| anyhow::anyhow!("raft new: {}", e))?;

    // Bootstrap the cluster if this is a fresh node (no vote stored yet).
    // For a single-node cluster we initialise immediately.
    // For multi-node clusters the caller is responsible for calling
    // `raft.initialize(members)` with the full quorum.
    let log_index = raft
        .metrics()
        .borrow()
        .last_log_index
        .clone();

    if log_index.is_none() {
        let mut members: BTreeMap<RaftNodeId, BasicNode> = BTreeMap::new();
        members.insert(
            node_id,
            BasicNode {
                addr: grpc_endpoint.to_string(),
            },
        );
        for (peer_id, endpoint) in &initial_peers {
            members.insert(*peer_id, BasicNode { addr: endpoint.clone() });
        }
        match raft.initialize(members).await {
            Ok(_) => {}
            Err(e) if e.to_string().contains("has already been initialized") => {}
            Err(e) => {
                return Err(anyhow::anyhow!("raft init: {}", e));
            }
        }
    }

    Ok(RaftNode { raft, node_id })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use uuid::Uuid;

    #[test]
    fn cluster_status_json_roundtrip() {
        let status = ClusterStatus {
            node_id: Uuid::new_v4().to_string(),
            quorum_participant: true,
            raft_leader: Some(Uuid::new_v4().to_string()),
            quorum_healthy: true,
            raft_term: 7,
            last_log_index: Some(42),
        };

        let json = serde_json::to_string(&status).expect("serialize cluster status");
        let decoded: ClusterStatus = serde_json::from_str(&json).expect("deserialize cluster status");

        assert_eq!(decoded.node_id, status.node_id);
        assert_eq!(decoded.raft_term, 7);
        assert_eq!(decoded.last_log_index, Some(42));
    }

    #[tokio::test]
    async fn init_raft_single_node_smoke_test() -> anyhow::Result<()> {
        let tmp = TempDir::new()?;
        let node_id = all4one_common::NodeId(Uuid::new_v4());
        let raft = init_raft(node_id, tmp.path().to_str().unwrap(), "127.0.0.1:7947", vec![]).await?;

        let metrics = raft.current_metrics();
        assert_eq!(metrics.id, node_id);
        Ok(())
    }
}
