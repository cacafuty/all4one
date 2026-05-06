# Architecture Overview

## Fundamental premise

All4One turns heterogeneous existing hardware into a unified compute and storage
cluster. The core principle is that there is no mandatory orchestrator node:
any node can accept jobs and coordinate work, and the cluster runs with whatever
is available at a given time.

**The system works with any number of nodes, from 1 to N.** A single running
agent is already a fully functional one-node cluster: it accepts jobs, executes
work, and stores data. Adding nodes increases compute capacity, enables stronger
data replication, and improves resilience.

This principle drives every major design decision:

- The agent is a **single Rust binary** with no external runtime.
- Consensus is **embedded** (openraft), not delegated to external etcd.
- Discovery is **mDNS + seeds**, with no central naming server.
- Scheduling is **distributed**: the first node receiving a job places it.

---

## Single component: the agent

The entire system is one binary installed on each device. No separate
coordinator, metadata server, or proxy is required.

```
┌─────────────────────────────────────────────────────────────────────┐
│                         ALL4ONE AGENT                              │
│                                                                     │
│  ┌──────────┐  ┌──────────┐  ┌────────────────────────────────┐   │
│  │  config  │  │   node   │  │         discovery              │   │
│  │ (toml)   │  │ (uuid)   │  │   mdns ◄──────────► seeds      │   │
│  └──────────┘  └──────────┘  └────────────────────────────────┘   │
│                                                                     │
│  ┌──────────────────────────────────────────────────────────────┐  │
│  │                   gossip (SWIM/UDP:7947)                    │  │
│  │         ClusterState: HashMap<NodeId, NodeInfo>            │  │
│  │         tokio::broadcast::Sender<MembershipEvent>          │  │
│  └──────────────────────────────────────────────────────────────┘  │
│                                                                     │
│  ┌──────────────────────────────────────────────────────────────┐  │
│  │                  raft (openraft, Phase 2+)                 │  │
│  │   BlockMap | JobRegistry | ClusterConfig | CRL             │  │
│  └──────────────────────────────────────────────────────────────┘  │
│                                                                     │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────────┐ │
│  │  scheduler   │  │   executor   │  │  storage (Phase 2+)      │ │
│  │  JobQueue    │  │  docker.rs   │  │  chunks + index          │ │
│  │  placement   │  │  jar.rs      │  │  SHA-256 + erasure       │ │
│  │  algorithm   │  │  python.rs   │  │  scrubbing               │ │
│  │              │  │  wasm.rs     │  │                          │ │
│  └──────────────┘  └──────────────┘  └──────────────────────────┘ │
│                                                                     │
│  ┌──────────────────────────────────────────────────────────────┐  │
│  │              lifecycle (Phase 4+, Raft leader only)         │  │
│  │            heat score + tier transitions                    │  │
│  └──────────────────────────────────────────────────────────────┘  │
│                                                                     │
│  ┌─────────────────────────┐  ┌───────────────────────────────┐   │
│  │  api_rest (axum :7946)  │  │  grpc_server (tonic :7947)    │   │
│  │  grpc_client (pool)     │  │  certificates (Phase 2+)      │   │
│  └─────────────────────────┘  └───────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Node tiers

Tiers represent **availability patterns**, not quality classes.
The scheduler uses these patterns when making placement decisions.

- **Tier 0**: 24/7 backbone (servers, NAS, dedicated Raspberry Pi).
  Critical metadata always lives here and at least one data replica remains here.
- **Tier 1**: predictable schedules (office PCs, managed laptops).
  Can participate in quorum and host secondary replicas.
- **Tier 2**: opportunistic nodes (personal laptops, Android).
  No quorum participation; suited for opportunistic compute/storage.

---

## Agent roles

Each node can enable up to three independent roles in `agent.toml`.

| Role      | Config flag             | Responsibility |
|-----------|-------------------------|----------------|
| SCHEDULER | `roles.scheduler=true`  | Accepts jobs via REST, computes placement, dispatches over gRPC |
| EXECUTOR  | `roles.executor=true`   | Runs jobs, manages lifecycle and stdout/stderr |
| STORAGE   | `roles.storage=true`    | Stores local chunks, serves reads, participates in replication/erasure |

A typical Tier 0 node enables all three roles.

---

## End-to-end job flow

```
Client (curl / SDK / boto3)
         │
         │  POST /v1/jobs  (YAML/JSON)
         ▼
  ┌─────────────┐
  │  api_rest   │ validates spec, creates JobId, forwards to scheduler
  └──────┬──────┘
         │
         ▼
  ┌─────────────┐
  │  scheduler  │ snapshot ClusterState
  │             │ filter by capabilities/resources/tier/window
  │             │ score candidates and choose target node
  └──────┬──────┘
         │
    ┌────┴─────────────────────────────┐
    │ chosen_node == self?             │
    Yes                                No
    │                                  │
    ▼                                  ▼
 executor.launch()          grpc_client.launch_job(chosen_node, ...)
    │                                  │
    ▼                                  ▼
 docker/jar/python/          remote node -> executor.launch()
 wasm/executable
    │
    ▼
 JobEvent stream (Started -> OutputLine* -> Completed|Failed)
    │
    ▼
 gossip propagates state across the cluster
```

---

## Storage flow (Phase 2+)

```
Client
  │  PUT /v1/storage/bucket/key  (bytes)
  ▼
api_rest -> storage module
  │
  ├── chunking (default 64MB)
  ├── SHA-256 per chunk
  ├── tier-based compression (zstd)
  ├── tier-based erasure coding (Reed-Solomon)
  │
  ▼
chunk placement
  ├── never place all replicas on same tier
  ├── consistent hashing baseline
  └── prefer nodes with larger remaining window
  │
  ▼
grpc_client.transfer_chunk() -> destination nodes
  │
  ▼
Raft.apply(PutChunkMap) -> BlockMap replicated in quorum
  │
  ▼
200 OK { etag, tier, replicas, ... }
```

---

## Discovery and membership

- On startup, each node announces itself via mDNS and probes configured seeds.
- Gossip (SWIM) sends heartbeats and indirect probes for failure detection.
- Membership updates are broadcast through `tokio::broadcast`.
- Scheduler and storage subscribe to membership events for retry/rebalance logic.

---

## Security by phase

- **Phase 1 (dev)**:
  shared secret (`X-All4One-Secret`) and plaintext transport.
- **Phase 2+ (production)**:
  internal Ed25519 CA, mTLS on inter-node gRPC, one-time enrollment tokens,
  Raft-replicated CRL for immediate revocation, and certificate rotation.

---

## Operational limits (key defaults)

| Parameter | Value |
|-----------|-------|
| Node connect timeout | 5 seconds |
| LaunchJob timeout | 10 seconds |
| Chunk transfer timeout (64MB) | 60 seconds |
| SWIM heartbeat | every 10 seconds |
| SUSPECTED after | 30 seconds |
| OFFLINE after | 60 seconds in SUSPECTED |
| Max job output captured | 10 MB |
| Max JobSpec size | 1 MB |
| Max nodes in ClusterState | 500 |
| Default chunk size | 64 MB |

---

## Cluster monitoring and visualization

### Integrated web dashboard

Each node exposes an interactive dashboard at `GET /` with:

- Local node info (id, tier, uptime, roles)
- Cluster health (total, online, offline nodes)
- Distributed memory state (Raft leader and synchronization)
- Storage health (data directory access, object count, available space)
- Cluster node list (online/offline and endpoints)

The dashboard auto-refreshes every 5 seconds.

### Diagnostic endpoints

- `GET /v1/cluster/status` - basic Raft consensus status
- `GET /v1/cluster/diagnostics` - full diagnostics and health checks
- `GET /health` - basic node health
- `GET /metrics` - Prometheus-friendly metrics

### Shared-folder checks

Storage diagnostics verify:

- data directory accessibility
- available free space
- object/chunk counters

Access failures are reported via `/v1/cluster/diagnostics`.

### Distributed-memory checks

Raft diagnostics verify:

- leader presence
- consensus synchronization
- quorum availability

Status is exposed in `distributed_state.cluster_synchronized`.

---

## References

- [Agent modules in detail](agent.md)
- [Network protocols and ports](networking.md)
- [Scheduling algorithm](scheduler.md)
- [Distributed storage](storage.md)
- [Lifecycle engine](lifecycle.md)
- [AI and inference](ai-inference.md)
