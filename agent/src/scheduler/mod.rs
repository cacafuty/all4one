use all4one_common::{ClusterState, NodeId, NodeInfo, NodeStatus, Runtime};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct SchedulingRequest {
    pub runtime: Runtime,
    pub tier_min: u8,
    pub require_docker: bool,
    pub source: String,
    pub command: Vec<String>,
}

pub fn pick_node(
    _local: &NodeInfo,
    cluster: &ClusterState,
    req: &SchedulingRequest,
    running_jobs: &HashMap<NodeId, usize>,
    excluded_nodes: &HashSet<NodeId>,
) -> Option<NodeId> {
    let mut candidates: Vec<&NodeInfo> = cluster
        .nodes
        .values()
        .filter(|n| n.status == NodeStatus::Online)
        .filter(|n| n.profile.tier >= req.tier_min)
        .filter(|n| !excluded_nodes.contains(&n.profile.id))
        .filter(|n| runtime_supported(n, req))
        .collect();

    if candidates.is_empty() {
        return None;
    }

    // Prefer the node with fewer currently running jobs.
    // Tie-break by deterministic NodeId ordering for stable placement.
    candidates.sort_by_key(|n| {
        (
            running_jobs.get(&n.profile.id).copied().unwrap_or(0),
            n.profile.id.to_string(),
        )
    });

    candidates.first().map(|n| n.profile.id)
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
        Runtime::Executable => executable_supported(node, &req.source, &req.command),
    }
}

fn executable_supported(node: &NodeInfo, source: &str, command: &[String]) -> bool {
    let required_os = required_os_for_executable(source, command);
    let node_os = node.profile.capabilities.operating_system.to_lowercase();

    match required_os {
        None => true,
        // If peer OS metadata is missing, allow scheduling as a fallback.
        // This keeps mixed-cluster dispatch working while discovery converges.
        Some(os) => node_os.is_empty() || node_os == os,
    }
}

fn required_os_for_executable(source: &str, command: &[String]) -> Option<&'static str> {
    let source = source.trim().to_lowercase();
    if is_windows_executable_target(&source) {
        return Some("windows");
    }
    if is_unix_shell_target(&source) {
        return Some("linux");
    }

    let first_arg = command
        .first()
        .map(|s| s.trim().to_lowercase())
        .unwrap_or_default();

    if is_windows_executable_target(&first_arg) {
        return Some("windows");
    }
    if is_unix_shell_target(&first_arg) {
        return Some("linux");
    }

    None
}

fn is_windows_executable_target(value: &str) -> bool {
    value.ends_with(".exe")
        || value.ends_with(".bat")
        || value.ends_with(".cmd")
        || matches!(
            value,
            "cmd" | "cmd.exe" | "powershell" | "powershell.exe" | "pwsh" | "pwsh.exe"
        )
}

fn is_unix_shell_target(value: &str) -> bool {
    value.ends_with(".sh") || matches!(value, "sh" | "bash")
}
