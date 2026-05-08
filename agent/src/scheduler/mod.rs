use all4one_common::{ClusterState, NodeId, NodeInfo, NodeStatus, Runtime};

#[derive(Debug, Clone)]
pub struct SchedulingRequest {
    pub runtime: Runtime,
    pub tier_min: u8,
    pub require_docker: bool,
}

pub fn pick_node(
    _local: &NodeInfo,
    cluster: &ClusterState,
    req: &SchedulingRequest,
) -> Option<NodeId> {
    let mut candidates: Vec<&NodeInfo> = cluster
        .nodes
        .values()
        .filter(|n| n.status == NodeStatus::Online)
        .filter(|n| n.profile.tier >= req.tier_min)
        .collect();

    if candidates.is_empty() {
        return None;
    }

    // Sort deterministically by ID; remove local bias to allow valid remote placement.
    candidates.sort_by_key(|n| n.profile.id.to_string());

    for node in candidates {
        if !runtime_supported(node, req) {
            continue;
        }
        return Some(node.profile.id);
    }

    None
}

fn runtime_supported(node: &NodeInfo, req: &SchedulingRequest) -> bool {
    if req.require_docker && !node.profile.capabilities.docker {
        return false;
    }

    match req.runtime {
        Runtime::Docker => node.profile.capabilities.docker,
        Runtime::Python => node.profile.capabilities.python.is_some(),
        Runtime::Jar => node.profile.capabilities.java.is_some(),
        Runtime::Wasm => node.profile.capabilities.wasm,
        Runtime::Executable => true,
    }
}
