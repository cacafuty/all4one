#![allow(dead_code)]
#![allow(clippy::too_many_arguments)]

mod api_rest;
mod certificates;
mod config;
mod discovery;
mod executor;
mod gossip;
mod grpc_client;
mod grpc_server;
mod node;
mod raft;
mod scheduler;
mod storage;

use crate::api_rest::AppState;
use crate::config::load;
use crate::discovery::{mark_self_heartbeat, spawn_seed_discovery, upsert_self};
use crate::gossip::spawn_failure_detector;
use crate::node::{node_id, profile};
use all4one_common::{ClusterState, NodeInfo, NodeStatus};
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, RwLock};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() >= 2 && args[1] == "--version" {
        println!("all4one-agent 0.1.0");
        return;
    }

    if args.len() >= 2 && args[1] == "start" {
        let config_path =
            parse_config_path(&args).unwrap_or_else(|| "/etc/all4one/agent.toml".to_string());
        if let Err(err) = run_agent(&config_path).await {
            eprintln!("ERROR {err}");
            std::process::exit(1);
        }
        return;
    }

    eprintln!("Usage: all4one-agent start --config <path> | --version");
    std::process::exit(1);
}

fn parse_config_path(args: &[String]) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--config" && i + 1 < args.len() {
            return Some(args[i + 1].clone());
        }
        i += 1;
    }
    None
}

async fn run_agent(config_path: &str) -> anyhow::Result<()> {
    println!("INFO Starting All4One agent v0.1.0");
    println!("INFO Config path: {}", config_path);

    let config = load(config_path)?;
    let advertise_host = std::env::var("ALL4ONE_ADVERTISE_HOST")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            if config.network.bind_address == "0.0.0.0" {
                std::env::var("HOSTNAME").ok()
            } else {
                Some(config.network.bind_address.clone())
            }
        })
        .unwrap_or_else(|| config.network.bind_address.clone());
    let advertise_grpc_port = std::env::var("ALL4ONE_ADVERTISE_GRPC_PORT")
        .ok()
        .and_then(|v| v.trim().parse::<u16>().ok())
        .unwrap_or(config.network.grpc_port);
    let advertise_rest_port = std::env::var("ALL4ONE_ADVERTISE_REST_PORT")
        .ok()
        .and_then(|v| v.trim().parse::<u16>().ok())
        .unwrap_or(config.network.rest_port);
    let id = node_id(&config.node.data_dir)?;
    let profile = profile(&config, id);
    let local_node = NodeInfo {
        profile: profile.clone(),
        status: NodeStatus::Online,
        version: env!("CARGO_PKG_VERSION").to_string(),
        grpc_endpoint: format!("{}:{}", advertise_host, advertise_grpc_port),
        rest_endpoint: format!("{}:{}", advertise_host, advertise_rest_port),
    };
    let cluster = Arc::new(RwLock::new(ClusterState::default()));
    let last_seen = Arc::new(RwLock::new(HashMap::new()));

    upsert_self(cluster.clone(), local_node.clone()).await;
    mark_self_heartbeat(last_seen.clone(), id).await;

    spawn_seed_discovery(config.clone(), id, cluster.clone(), last_seen.clone());
    spawn_failure_detector(id, cluster.clone(), last_seen.clone());

    println!("INFO Node ID: {}", id);
    println!(
        "INFO Tier: {} | Roles: scheduler={} executor={} storage={}",
        config.node.tier, config.roles.scheduler, config.roles.executor, config.roles.storage
    );

    if config.security.mode == "dev" {
        println!("WARN MODO DESARROLLO ACTIVO - no usar en produccion");
    }

    let cert_manager = certificates::CertificateManager::new(&config.node.data_dir);
    cert_manager.ensure_dirs()?;

    let is_bootstrap_issuer = config.node.tier == 0 || config.discovery.seeds.is_empty();
    if is_bootstrap_issuer {
        cert_manager.init_ca().await?;
        println!("INFO Certificate issuer ready (cluster bootstrap node)");
    } else if !cert_manager.has_node_credentials() {
        enroll_with_seed_ca(&config, id, &cert_manager).await?;
    } else {
        println!("INFO Existing node certificate bundle found, skipping enrollment");
    }

    let raft_handle = if config.node.quorum_participant {
        let grpc_ep = format!("{}:{}", advertise_host, config.network.grpc_port);
        match raft::init_raft(id, &config.node.data_dir, &grpc_ep, vec![]).await {
            Ok(node) => {
                println!("INFO Raft initialised (quorum participant)");
                Some(node)
            }
            Err(e) => {
                eprintln!("WARN Raft init failed, running without Raft: {e}");
                None
            }
        }
    } else {
        println!("INFO Raft disabled (quorum_participant = false)");
        None
    };

    let state = AppState {
        config,
        started_at: Instant::now(),
        started_at_utc: chrono::Utc::now(),
        node_id: id,
        profile,
        local_node,
        cluster,
        last_seen,
        jobs: Arc::new(RwLock::new(HashMap::new())),
        output_channels: Arc::new(RwLock::new(HashMap::<_, broadcast::Sender<String>>::new())),
        ops_events: broadcast::channel(1024).0,
        raft: raft_handle,
    };

    grpc_server::start_background(state.clone()).await;

    api_rest::serve(state).await
}

async fn enroll_with_seed_ca(
    config: &crate::config::schema::Config,
    node_id: all4one_common::NodeId,
    cert_manager: &crate::certificates::CertificateManager,
) -> anyhow::Result<()> {
    let mut last_err: Option<anyhow::Error> = None;

    for _attempt in 0..20 {
        let join_secret = if config.security.mode == "dev" {
            let secret = config.security.shared_secret.trim();
            if secret.is_empty() {
                None
            } else {
                Some(secret)
            }
        } else {
            None
        };

        for seed in &config.discovery.seeds {
            match crate::grpc_client::request_join(seed, node_id.0, join_secret).await {
                Ok(bundle) => {
                    cert_manager.save_node_cert(
                        &bundle.node_cert_pem,
                        &bundle.node_key_pem,
                        &bundle.ca_cert_pem,
                    )?;
                    println!(
                        "INFO Enrollment successful via seed={} cert_expiry_unix={}",
                        seed, bundle.expires_at_unix
                    );
                    return Ok(());
                }
                Err(e) => {
                    last_err = Some(anyhow::anyhow!("seed={} join failed: {}", seed, e));
                }
            }
        }
        sleep(Duration::from_secs(2)).await;
    }

    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("No seed endpoints configured for enrollment")))
}
