# Docker Compose Lab

This folder provides a local multi-node All4One lab.

## Start

```bash
cd deploy/compose
docker compose up --build -d
```

## Start from GitHub Releases

```bash
cd deploy/compose
VERSION=0.1.5 GH_REPO=cacafuty/all4one docker compose -f docker-compose.release.yml up --build -d
```

## Validate

```bash
docker compose ps
docker compose logs -f agent-a
curl -s -H "X-All4One-Secret: compose-secret" http://localhost:7946/v1/nodes
```

## Fire test examples

Examples are under:

- `deploy/compose/examples/phase1-fire-test/agent-a/`
- `deploy/compose/examples/phase1-fire-test/agent-b/`
- `deploy/compose/examples/phase1-fire-test/agent-c/`

Run:

```powershell
./scripts/phase1-fire-test.ps1
```

## Phase 2 — Distributed Storage

Phase 2 adds `roles.storage = true` to all three agents and verifies that data is shared **only via gRPC** — each agent gets its own isolated Docker volume (`phase2_agent_a_data`, etc.), so there is no shared filesystem between them.

Phase 2 configs use full-mesh static seeds across `agent-a`, `agent-b`, and `agent-c` so each storage node can discover both peers and fan out shards in all directions.

### Start Phase 2 cluster

```bash
cd deploy/compose
docker compose -f docker-compose.phase2.yml up --build -d
```

### Run the 10-scenario storage test suite

```powershell
./scripts/phase2-storage-test.ps1
```

### Build reliability note

The compose Dockerfile pins Rust build parallelism (`CARGO_BUILD_JOBS=2`) to reduce memory pressure during image builds. This avoids intermittent BuildKit EOF failures on constrained machines.

What the tests cover:

| # | Scenario |
|---|---|
| T01 | Local write and read on agent-a |
| T02 | Shard replication: write to A, read from B |
| T03 | Shard replication: write to A, read from C |
| T04 | Idempotent write: same content → same ETag |
| T05 | All four storage policies (hot/warm/cold/archive) |
| T06 | Bucket isolation: same key in two buckets |
| T07 | DELETE removes object; GET returns 404 |
| T08 | Prefix-filtered bucket listing |
| T09 | Large object (512 KB) SHA-256 end-to-end integrity |
| T10 | All-node write mesh: distributed memory convergence |

### Running Phase 1 and Phase 2 side by side

Both compose files use the same host ports (7946/8946/9946). To run them simultaneously, use project names:

```bash
docker compose -p phase1 -f docker-compose.yml up -d
docker compose -p phase2 -f docker-compose.phase2.yml up -d
```

### Stop Phase 2 cluster

```bash
docker compose -f docker-compose.phase2.yml down
# Remove phase2 volumes (data is isolated from phase1):
docker compose -f docker-compose.phase2.yml down -v
```

## Stop

```bash
docker compose down
docker compose down -v
```
