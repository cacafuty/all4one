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

For scripted multi-device checks, see `scripts/phase3-ops-ui-check.ps1`.

## Jobs

- `POST /v1/jobs`
- `GET /v1/jobs` (supports `status`, `node_id`, `limit` query parameters)
- `GET /v1/jobs/{id}`
- `DELETE /v1/jobs/{id}`
- `GET /v1/jobs/{id}/output`
- `GET /v1/jobs/{id}/output/stream` (SSE)

## Nodes and cluster

- `GET /v1/nodes`
- `GET /v1/nodes/{id}`
- `GET /v1/internal/node`
- `GET /v1/ops/events` (SSE, live operational events)

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

After writing locally, the node fans out shards to peer storage nodes via gRPC `TransferChunk` (fire-and-forget, eventual consistency).

### Download object

```
GET /v1/storage/{bucket}/{key...}
```
Response: `200 OK` with raw bytes body. Header `ETag` contains the SHA-256 of the object.

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

## Inference

- model registration/loading endpoints
- synchronous and streaming inference endpoints

## Status model

`queued -> scheduled -> running -> completed|failed|cancelled`

## Error model

JSON body with machine-readable `code`, user-facing `message`, and optional `details`.
