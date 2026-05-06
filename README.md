# All4One

P2P distributed infrastructure platform that turns existing heterogeneous hardware
(laptops, office PCs, servers, Raspberry Pi, Android phones) into a unified compute
and storage cluster. Processes running on it are unaware of the distributed environment.

---

## Core principle

All4One works with **any number of nodes: from 1 to N**. A single agent on your
laptop is already a fully functional cluster; adding more nodes increases capacity,
resilience, and hardware diversity — but is never a requirement.

There is no mandatory orchestration node. Any node can receive jobs and coordinate.
The cluster works with whatever is available at any given moment.

```
┌─────────────┐   ┌─────────────┐   ┌─────────────┐
│   Laptop    │   │ Raspberry   │   │ Office PC   │
│  (Tier 1)   │◄──│   Pi Tier 0 │──►│  (Tier 1)   │
│  scheduler  │   │  scheduler  │   │  executor   │
│  executor   │   │  executor   │   │  storage    │
└─────────────┘   │  storage    │   └─────────────┘
                  │  raft/quorum│
                  └─────────────┘
         Any node accepts jobs and coordinates
```

---

## Architecture at a glance

```
                        AGENT (single Rust binary)
                 ┌──────────────────────────────────────┐
                 │  config  │  node  │  discovery        │
                 │──────────────────────────────────────│
                 │  gossip (SWIM UDP:7947)               │
                 │──────────────────────────────────────│
                 │  raft (openraft, Phase 2+)            │
                 │──────────────────────────────────────│
                 │  scheduler  │  executor  │  storage   │
                 │──────────────────────────────────────│
                 │  api_rest (:7946) │ grpc (:7947)      │
                 │──────────────────────────────────────│
                 │  certificates (Phase 2+)              │
                 │──────────────────────────────────────│
                 │  lifecycle (Phase 4+, Raft leader only)│
                 └──────────────────────────────────────┘
```

Each agent runs up to three roles simultaneously, configured independently:

- **SCHEDULER** — receives jobs and decides placement
- **EXECUTOR** — runs assigned jobs
- **STORAGE** — stores chunks

---

## Node tiers

| Tier | Description                                        | Quorum |
|------|----------------------------------------------------|--------|
| 0    | 24/7. Servers, NAS, dedicated Raspberry Pi.        | Yes    |
| 1    | Predictable availability on a known schedule.      | Yes    |
| 2    | Opportunistic. Personal laptops, mobile devices.   | No     |

Critical metadata and at least one replica of every object always reside on Tier 0.

---

## Quick start (Phase 1)

> A single node is enough to get started. You can add more at any time.
> In Phase 1, `shared_secret` protects REST endpoints as a simple access-control option.
> In Phase 2+, enrollment is always CA-issued via `Join`; join authorization supports both CA-based bootstrap trust (recommended) and an optional `shared_secret` gate.

### Single node (minimum viable setup)

```bash
# Installation
curl -sSL https://releases.all4one.io/install.sh | bash

# Minimal configuration
cat > /etc/all4one/agent.toml << 'EOF'
[node]
tier = 0
availability = "always"
quorum_participant = true
data_dir = "/var/lib/all4one"

[roles]
scheduler = true
executor = true
storage = true

[network]
bind_address = "0.0.0.0"
grpc_port = 7947
rest_port = 7946

[security]
mode = "dev"
shared_secret = "change-me-before-production"

[discovery]
mdns = true
seeds = []
EOF

# Start
all4one-agent start
```

### Additional node (optional — adds capacity and resilience)

```bash
cat > /etc/all4one/agent.toml << 'EOF'
[node]
tier = 1
availability = "cron:0 9-18 * * 1-5"
quorum_participant = true
data_dir = "/var/lib/all4one"

[roles]
scheduler = false
executor = true
storage = false

[network]
bind_address = "0.0.0.0"
grpc_port = 7947
rest_port = 7946

[security]
mode = "dev"
shared_secret = "change-me-before-production"

[discovery]
mdns = true
seeds = ["192.168.1.100:7947"]
EOF

all4one-agent start
```

### First job

```bash
cat > hello.yaml << 'EOF'
runtime: docker
source: "python:3.11-slim"
command: ["python", "-c", "print('Hello from All4One!')"]
resources:
  cpu_cores: 1
  memory_mb: 256
EOF

curl -X POST http://192.168.1.100:7946/v1/jobs \
  -H "Content-Type: application/yaml" \
  -H "X-All4One-Secret: change-me-before-production" \
  --data-binary @hello.yaml
```

---

## Implementation phases

| Phase | Name                              | Brief description                             |
|-------|-----------------------------------|-----------------------------------------------|
| 1     | Run something, somewhere          | Gossip + scheduler + executor. No storage.    |
| 2     | Data lives in the cluster         | Raft + storage + mTLS + internal PKI.         |
| 3     | Operational UI and visibility     | Real-time cluster state, jobs, storage, events. |
| 4     | Data manages itself               | Lifecycle engine + automatic tiering.         |
| 5     | Full process transparency         | FUSE + LD_PRELOAD + SDK + S3 API.             |
| 6     | Platforms and maturity            | Android + GPU + multi-tenant + hardening.     |

---

## Documentation

### Architecture
- [Overview](docs/architecture/overview.md)
- [Agent modules](docs/architecture/agent.md)
- [Network protocols](docs/architecture/networking.md)
- [Scheduler and placement](docs/architecture/scheduler.md)
- [Distributed storage](docs/architecture/storage.md)
- [Lifecycle engine](docs/architecture/lifecycle.md)
- [AI and inference](docs/architecture/ai-inference.md)

### API
- [Job specification](docs/api/job-spec.md)
- [Full REST API](docs/api/rest-api.md)
- [agent.toml reference](docs/api/config-reference.md)

### Phases
- [Phase 1](docs/phases/phase-1.md) · [Phase 2](docs/phases/phase-2.md) · [Phase 3](docs/phases/phase-3.md) · [Phase 4](docs/phases/phase-4.md) · [Phase 5](docs/phases/phase-5.md) · [Phase 6](docs/phases/phase-6.md)

### Guides
- [Node setup](docs/guides/node-setup.md)
- [Getting started](docs/guides/getting-started.md)

### Architecture decisions
- [ADR-001 Rust for the agent](docs/decisions/001-rust-agent.md)
- [ADR-002 No central node](docs/decisions/002-no-central-node.md)
- [ADR-003 No MinIO](docs/decisions/003-no-minio.md)
- [ADR-004 llama.cpp RPC](docs/decisions/004-llama-cpp-rpc.md)
- [ADR-005 Internal gRPC](docs/decisions/005-grpc-internal.md)
- [ADR-006 PKI + mTLS](docs/decisions/006-pki-mtls.md)

---

## Technology stack (license summary)

| Crate / library  | License      | Usage                       |
|------------------|--------------|-----------------------------|
| tokio            | MIT          | async runtime               |
| axum             | MIT          | API REST                    |
| tonic            | MIT          | gRPC server and client      |
| prost            | Apache 2.0   | Protocol Buffers            |
| openraft         | Apache 2.0   | embedded Raft consensus     |
| chitchat         | Apache 2.0   | SWIM gossip                 |
| mdns-sd          | MIT          | mDNS discovery              |
| reed-solomon     | Apache 2.0   | erasure coding              |
| zstd             | MIT          | compression                 |
| rcgen            | MIT          | X.509 certificates          |
| rustls           | Apache 2.0   | TLS                         |
| sled             | MIT          | chunk index                 |
| fuser            | MIT          | FUSE Linux/macOS            |
| WinFsp           | LGPL         | FUSE Windows (dynamic link) |
| wasmtime         | Apache 2.0   | WASM runtime                |
| llama.cpp        | MIT          | AI inference and RPC        |
| bincode          | MIT          | SWIM serialization          |

> **MinIO discarded**: AGPL v3 incompatible with proprietary redistribution.
> See [ADR-003](docs/decisions/003-no-minio.md).

---

## Supported platforms

| Platform        | Executor | Storage | Quorum | FUSE |
|-----------------|----------|---------|--------|------|
| Linux x86_64    | ✓        | ✓       | ✓      | Native |
| Linux ARM64     | ✓        | ✓       | ✓      | Native |
| macOS ARM64     | ✓        | ✓       | ✓      | macFUSE |
| macOS x86_64    | ✓        | ✓       | ✓      | macFUSE |
| Windows x86_64  | ✓        | ✓       | ✓      | WinFsp |
| Android ARM64   | —        | ✓       | —      | — |

iOS: dropped from v1 due to Apple sandbox restrictions.
