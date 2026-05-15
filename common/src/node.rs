use crate::types::NodeId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    Online,
    Suspected,
    Offline,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeResources {
    pub cpu_cores: u32,
    pub memory_mb: u32,
    pub disk_mb: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeCapabilities {
    pub docker: bool,
    pub python: Option<String>,
    pub java: Option<String>,
    pub wasm: bool,
    pub gpu_enabled: bool,
    #[serde(default)]
    pub operating_system: String,
    #[serde(default)]
    pub storage_node: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeProfile {
    pub id: NodeId,
    pub tier: u8,
    pub availability: String,
    pub quorum_participant: bool,
    pub resources: NodeResources,
    pub capabilities: NodeCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeInfo {
    pub profile: NodeProfile,
    pub status: NodeStatus,
    pub version: String,
    pub grpc_endpoint: String,
    pub rest_endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ClusterState {
    pub nodes: HashMap<NodeId, NodeInfo>,
    pub version: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cluster_state_roundtrip_json() {
        let mut nodes = HashMap::new();
        let id = NodeId::new();
        nodes.insert(
            id,
            NodeInfo {
                profile: NodeProfile {
                    id,
                    tier: 0,
                    availability: "always".to_string(),
                    quorum_participant: true,
                    resources: NodeResources {
                        cpu_cores: 4,
                        memory_mb: 4096,
                        disk_mb: Some(32768),
                    },
                    capabilities: NodeCapabilities {
                        docker: true,
                        python: Some("/usr/bin/python3".to_string()),
                        java: None,
                        wasm: true,
                        gpu_enabled: false,
                        operating_system: "linux".to_string(),
                        storage_node: false,
                    },
                },
                status: NodeStatus::Online,
                version: "0.1.0".to_string(),
                grpc_endpoint: "127.0.0.1:7947".to_string(),
                rest_endpoint: "127.0.0.1:7946".to_string(),
            },
        );

        let state = ClusterState { nodes, version: 1 };
        let json = serde_json::to_string(&state).expect("serialize cluster state");
        let decoded: ClusterState = serde_json::from_str(&json).expect("deserialize cluster state");
        assert_eq!(state, decoded);
    }
}
