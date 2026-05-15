# REST API - Complete Specification

Base URL: `http(s)://{node}:7946`

All responses include `X-Request-Id`.

## Authentication

- Dev mode: `X-All4One-Secret`
- Prod mode: mTLS + policy checks

## Health and diagnostics

- `GET /health`
- `GET /metrics`
- `GET /v1/cluster/status`
- `GET /v1/cluster/diagnostics`
- `GET /` (dashboard)

The dashboard (`GET /`) is the Phase 3 operational UI. It aggregates:

- Cluster pulse (online nodes, quorum, synchronization)
- Node topology and role visibility
- Job lifecycle counters and recent jobs
- Storage accessibility and object counters
- Operational timeline generated from recent cluster/job changes
- Shared volume explorer with cluster-wide file visibility and direct download

The explorer can show objects that are not physically stored on the consulted node (`remote-only`) if they are available on peer storage nodes. Download still works through the current node via read-through retrieval.

For scripted multi-device checks, see `scripts/phase3-ops-ui-check.ps1`.

## Jobs

- `POST /v1/jobs`
- `GET /v1/jobs` (supports `status`, `node_id`, `limit`, `local_only` query parameters)
- `GET /v1/jobs/{id}` (supports `local_only` query parameter)
- `DELETE /v1/jobs/{id}`
- `GET /v1/jobs/{id}/output`
- `GET /v1/jobs/{id}/output/stream` (SSE)

By default, `GET /v1/jobs` and `GET /v1/jobs/{id}` aggregate across online peers so any node dashboard can show cluster activity. Use `local_only=true` to query only the local node and disable fan-out.

For retries, failed jobs can be re-queued by the origin node and dispatched again to another eligible node. Retry dispatch excludes nodes already attempted for the same job.

## Nodes and cluster

- `GET /v1/nodes`
- `GET /v1/nodes/{id}`
- `GET /v1/internal/node`
- `GET /v1/ops/events` (SSE, live operational events)

`GET /v1/nodes` now includes a `telemetry` array in addition to `nodes`, with per-node runtime status snapshots for observability:

- `idle`
- `running_jobs`
- `queued_jobs`
- `estimated_used_memory_mb`
- `estimated_free_memory_mb`
- `last_seen_ms_ago`
- `telemetry_fresh`

This telemetry is informational and does not by itself enforce scheduler filtering.

## Security lifecycle

- `POST /v1/security/enroll/token`
- `POST /v1/security/enroll`
- `POST /v1/security/revoke`

## Storage

Available on nodes with `roles.storage = true`. Clients receive `503 Service Unavailable` on nodes without this role.

Keys may contain slashes (e.g. `logs/2026/01/app.log`).

### Create bucket

```
POST /v1/storage/{bucket}
```
Response: `201 Created`

### Upload object

```
PUT /v1/storage/{bucket}/{key...}
```
Headers:
- `X-All4One-Policy`: `hot` | `warm` (default) | `cold` | `archive`

Response: `201 Created` with `ObjectMetadata` JSON.

The node applies adaptive compression by local access recency:
- Accessed by this agent in last 24h: requested policy is preserved.
- Not accessed in last 24h (or first write on this agent): effective policy is forced to `archive`.

After writing locally, the node selects replica targets with tier priority (tier `0` first) and fans out shards via gRPC `TransferChunk` (fire-and-forget, eventual consistency).

Replication target is minimum 3 sites total (including the writer) when enough agents are online.

Replica writes use `archive` and do not count as access on the replica node, so unused copies remain compressed to reduce disk usage.

### Download object

```
GET /v1/storage/{bucket}/{key...}
```
Response: `200 OK` with raw bytes body. Header `ETag` contains the SHA-256 of the object.

If the object is not readable locally, the agent performs parallel peer read-through (`GET` to other online agents), returns the first success, and caches the object locally.

### Delete object

```
DELETE /v1/storage/{bucket}/{key...}
```
Response: `204 No Content`

### List objects

```
GET /v1/storage/{bucket}?prefix=...&max_keys=...
```
Response: `200 OK` with JSON `{ "bucket", "objects": [...], "count" }`.

### Node-to-node chunk transport

Shard replication between agents does **not** use REST. It uses the `TransferChunk` / `FetchChunk` gRPC RPCs on port `:7947`. See [architecture/storage.md](../architecture/storage.md) for details.

### Local shared-folder listener

When `[shared_volume].enabled = true`, the agent scans a local shared folder and syncs changes to storage APIs automatically:

- file create/update -> `PUT /v1/storage/{bucket}/{key...}`
- file delete -> `DELETE /v1/storage/{bucket}/{key...}`

This listener runs inside each agent and is intended to keep the local shared folder and distributed storage aligned while jobs are running.

## Inference

- model registration/loading endpoints
- synchronous and streaming inference endpoints

## Status model

`queued -> scheduled -> running -> completed|failed|cancelled`

## Error model

JSON body with machine-readable `code`, user-facing `message`, and optional `details`.
