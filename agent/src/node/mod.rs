use crate::config::schema::Config;
use all4one_common::{NodeCapabilities, NodeId, NodeProfile, NodeResources};
use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

pub fn node_id(data_dir: &str) -> Result<NodeId> {
    let data_dir = PathBuf::from(data_dir);
    fs::create_dir_all(&data_dir).with_context(|| format!("cannot create data_dir {}", data_dir.display()))?;

    let node_id_path = data_dir.join("node-id");
    if node_id_path.exists() {
        let raw = fs::read_to_string(&node_id_path)
            .with_context(|| format!("cannot read {}", node_id_path.display()))?;
        let parsed = uuid::Uuid::parse_str(raw.trim()).context("invalid node-id format")?;
        return Ok(NodeId(parsed));
    }

    let id = NodeId::new();
    fs::write(&node_id_path, id.to_string())
        .with_context(|| format!("cannot write {}", node_id_path.display()))?;
    Ok(id)
}

pub fn profile(config: &Config, id: NodeId) -> NodeProfile {
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);
    let memory_mb = detect_memory_mb();

    NodeProfile {
        id,
        tier: config.node.tier,
        availability: config.node.availability.clone(),
        quorum_participant: config.node.quorum_participant,
        resources: NodeResources {
            cpu_cores,
            memory_mb,
            disk_mb: None,
        },
        capabilities: NodeCapabilities {
            docker: config.capabilities.docker,
            python: config.capabilities.python.clone(),
            java: config.capabilities.java.clone(),
            wasm: config.capabilities.wasm,
            gpu_enabled: config.capabilities.gpu_enabled,
            storage_node: config.roles.storage,
        },
    }
}

fn detect_memory_mb() -> u32 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(raw) = fs::read_to_string("/proc/meminfo") {
            // Prefer MemAvailable; fallback to MemTotal if unavailable.
            for key in ["MemAvailable:", "MemTotal:"] {
                if let Some(value) = parse_meminfo_kb(&raw, key) {
                    let mb = value / 1024;
                    if mb > 0 {
                        return mb as u32;
                    }
                }
            }
        }
    }

    // Conservative fallback for non-linux environments.
    1024
}

#[cfg(target_os = "linux")]
fn parse_meminfo_kb(raw: &str, key: &str) -> Option<u64> {
    raw.lines()
        .find(|line| line.starts_with(key))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|n| n.parse::<u64>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use crate::config::schema::{
        CapabilitiesConfig, Config, DiscoveryConfig, ExecutorConfig, LoggingConfig, NetworkConfig,
        NodeConfig, RolesConfig, SecurityConfig,
    };

    fn fake_config(data_dir: &Path) -> Config {
        Config {
            node: NodeConfig {
                tier: 0,
                availability: "always".to_string(),
                quorum_participant: true,
                data_dir: data_dir.display().to_string(),
            },
            roles: RolesConfig {
                scheduler: true,
                executor: true,
                storage: false,
            },
            network: NetworkConfig {
                bind_address: "0.0.0.0".to_string(),
                grpc_port: 7947,
                rest_port: 7946,
                metrics_port: 9090,
            },
            discovery: DiscoveryConfig {
                mdns: true,
                seeds: vec![],
            },
            security: SecurityConfig {
                mode: "dev".to_string(),
                shared_secret: "s".to_string(),
            },
            executor: ExecutorConfig {
                max_concurrent_jobs: 8,
                docker_socket: "/var/run/docker.sock".to_string(),
                cgroups_enabled: true,
                output_max_bytes: 10 * 1024 * 1024,
            },
            capabilities: CapabilitiesConfig {
                docker: true,
                java: None,
                python: Some("/usr/bin/python3".to_string()),
                wasm: true,
                gpu_enabled: false,
            },
            logging: LoggingConfig {
                level: "info".to_string(),
                format: "text".to_string(),
            },
        }
    }

    #[test]
    fn node_id_is_persistent() {
        let temp = tempfile::tempdir().expect("tempdir");
        let first = node_id(temp.path().to_str().expect("path str")).expect("first id");
        let second = node_id(temp.path().to_str().expect("path str")).expect("second id");
        assert_eq!(first, second);
    }

    #[test]
    fn profile_uses_config_values() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config = fake_config(temp.path());
        let id = NodeId::new();
        let p = profile(&config, id);
        assert_eq!(p.id, id);
        assert_eq!(p.tier, 0);
        assert!(p.capabilities.docker);
        assert_eq!(p.availability, "always");
    }
}
