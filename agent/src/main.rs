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
mod shared_volume;
mod storage;

use crate::api_rest::AppState;
use crate::config::load;
use crate::config::schema::Config;
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
        println!("all4one-agent {}", env!("CARGO_PKG_VERSION"));
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
    println!("INFO Starting All4One agent v{}", env!("CARGO_PKG_VERSION"));
    println!("INFO Config path: {}", config_path);

    let config = load(config_path)?;
    let advertise_host = std::env::var("ALL4ONE_ADVERTISE_HOST")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            if !config.network.advertise_host.is_empty() {
                Some(config.network.advertise_host.clone())
            } else if config.network.bind_address == "0.0.0.0" {
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

    if config.security.mode == "shared-secret" {
        println!("WARN MODO DESARROLLO ACTIVO - no usar en produccion");
    }

    let cert_manager = certificates::CertificateManager::new(&config.node.data_dir);
    cert_manager.ensure_dirs()?;

    let is_bootstrap_issuer = config.node.tier == 0 || config.discovery.seeds.is_empty();
    if is_bootstrap_issuer {
        cert_manager.init_ca().await?;
        println!("INFO Certificate issuer ready (cluster bootstrap node)");
    } else if !cert_manager.has_node_credentials() {
        enroll_with_seed_ca(
            &config,
            id,
            advertise_host.clone(),
            advertise_grpc_port,
            advertise_rest_port,
            &cert_manager,
        )
        .await?;
    } else {
        println!("INFO Existing node certificate bundle found, skipping enrollment");
    }

    if !is_bootstrap_issuer && !config.discovery.seeds.is_empty() {
        spawn_seed_presence_refresh(
            config.clone(),
            id,
            advertise_host.clone(),
            advertise_grpc_port,
            advertise_rest_port,
        );
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
    shared_volume::spawn_shared_volume_listener(
        state.config.clone(),
        state.local_node.rest_endpoint.clone(),
    );

    api_rest::serve(state).await
}

async fn enroll_with_seed_ca(
    config: &crate::config::schema::Config,
    node_id: all4one_common::NodeId,
    advertise_host: String,
    advertise_grpc_port: u16,
    advertise_rest_port: u16,
    cert_manager: &crate::certificates::CertificateManager,
) -> anyhow::Result<()> {
    let mut last_err: Option<anyhow::Error> = None;

    let ca_cert_path = if config.security.ca_cert_path.is_empty() {
        None
    } else {
        Some(config.security.ca_cert_path.as_str())
    };

    for _attempt in 0..20 {
        let join_secret = if config.security.mode == "shared-secret" {
            let secret = config.security.shared_secret.trim();
            if secret.is_empty() {
                None
            } else {
                Some(secret)
            }
        } else {
            None
        };

        for seed in crate::discovery::resolved_seeds(config) {
            match crate::grpc_client::request_join_with_ca(
                &seed,
                node_id.0,
                join_secret,
                ca_cert_path,
                config.node.tier,
                &advertise_host,
                advertise_grpc_port,
                advertise_rest_port,
            )
            .await
            {
                Ok(bundle) => {
                    if bundle.node_cert_pem.trim().is_empty()
                        || bundle.node_key_pem.trim().is_empty()
                        || bundle.ca_cert_pem.trim().is_empty()
                    {
                        last_err = Some(anyhow::anyhow!(
                            "seed={} does not issue certificates; try a CA issuer seed",
                            seed
                        ));
                        continue;
                    }
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

fn spawn_seed_presence_refresh(
    config: Arc<Config>,
    node_id: all4one_common::NodeId,
    advertise_host: String,
    advertise_grpc_port: u16,
    advertise_rest_port: u16,
) {
    tokio::spawn(async move {
        let mut failure_streak: u32 = 0;

        loop {
            let join_secret = if config.security.mode == "shared-secret" {
                let secret = config.security.shared_secret.trim();
                if secret.is_empty() {
                    None
                } else {
                    Some(secret)
                }
            } else {
                None
            };

            let mut any_success = false;
            let mut last_err: Option<String> = None;
            for seed in crate::discovery::resolved_seeds(config.as_ref()) {
                match crate::grpc_client::announce_presence(
                    &seed,
                    node_id.0,
                    join_secret,
                    config.node.tier,
                    &advertise_host,
                    advertise_grpc_port,
                    advertise_rest_port,
                )
                .await
                {
                    Ok(_) => {
                        any_success = true;
                    }
                    Err(e) => {
                        last_err = Some(format!("seed={} join refresh failed: {}", seed, e));
                    }
                }
            }

            if any_success {
                if failure_streak > 0 {
                    println!("INFO Presence refresh restored after transient failures");
                }
                failure_streak = 0;
            } else {
                failure_streak = failure_streak.saturating_add(1);
                if failure_streak.is_multiple_of(6) {
                    if let Some(err) = &last_err {
                        eprintln!("WARN {}", err);
                    }
                }
            }

            sleep(Duration::from_secs(10)).await;
        }
    });
}
