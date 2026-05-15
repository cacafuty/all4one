# Distributed Storage

All4One storage provides an S3-like object API backed by local chunk files, with peer-to-peer shard distribution across storage nodes.

## Core features

- Chunking, hashing (SHA-256 per shard and full object)
- Tiered policy (`hot` / `warm` / `cold` / `archive`)
- Compression with zstd (level varies by policy)
- Custom all4one compression container (`A4O1` magic + codec metadata) for fast compressed-payload detection
- Erasure coding (data shards + parity shards; real RS planned)
- Metadata index via `sled` embedded DB
- Per-agent access timestamp (`last_accessed_at`) to drive cold-object compression behavior
- Node capability flag: `roles.storage = true` enables this role

## Storage policies

| Policy  | Shards     | zstd level | Description                  |
|---------|-----------|-----------|------------------------------|
| hot     | 1 data     | 0 (none)  | 3x replication, no compress  |
| warm    | 4+2 RS     | 3         | Default; balanced             |
| cold    | 6+3 RS     | 19        | High-compression archival     |
| archive | 8+4 RS     | 22        | Maximum durability/compress   |

## Write path

1. REST client sends `PUT /v1/storage/{bucket}/{key}` with optional `X-All4One-Policy` header.
2. The local node resolves an effective policy from local recency:
	- if this node accessed the object in the last 24h, keep the requested policy,
	- otherwise force `archive` (strong compression).
3. The local storage node compresses, erasure-codes, and writes shards to `{data_dir}/chunks/{bucket}/`.
	- Compressed payloads are wrapped in all4one format, preserving compatibility with legacy plain-zstd objects.
4. Object metadata is written to the local sled index (`{data_dir}/objects.db`).
5. The node selects replication targets preferring lower tiers (tier `0` first) and pushes shards via gRPC `TransferChunk`.
6. Target count is at least 3 sites total (writer + peers) when enough online agents exist; with fewer agents online, it uses all available sites.
7. Replica copies are stored as `archive` and do not update `last_accessed_at`; they stay highly compressed until the replica node actually serves a read.

## Read path

1. REST client sends `GET /v1/storage/{bucket}/{key}`.
2. The node reads shards from local disk, reconstructs the object via `decode_erasure`, decompresses, and verifies SHA-256.
3. Local metadata `last_accessed_at` is refreshed.
4. If the object is not readable locally, the node performs parallel peer read-through over peer REST endpoints and returns the first successful response.
5. Read-through results are cached locally and marked as accessed.

## Node-to-node transport (gRPC)

The internal shard distribution uses **gRPC** (port `:7947`), not REST. This is intentional:

- REST is the external client interface (S3-like; apps, curl, SDKs).
- gRPC is the internal interface (nodes talk to each other; binary streaming, lower overhead).

### RPCs

| RPC              | Direction       | Description                          |
|------------------|----------------|--------------------------------------|
| `TransferChunk`  | writer → peers | Push a shard to a peer storage node   |
| `FetchChunk`     | reader → peers | Pull a shard from a peer (recovery)   |

## Recovery

- Verify SHA-256 hash on reads; corrupt data is rejected.
- Erasure coding allows partial shard loss recovery (real RS coding planned for Phase 4).
- Future scrub daemon will detect drift and trigger re-replication.

## Shard file layout

```
{data_dir}/chunks/{bucket}/{bucket}-{key}-shard-{i}
{data_dir}/chunks/{bucket}/{bucket}-{key}-shard-{i}.meta
{data_dir}/objects.db   ← sled index (tree per bucket)
```

## Operational notes

- Enable with `roles.storage = true` in agent config.
- Keep `data_dir` on a writable, monitored filesystem.
- Use `GET /v1/cluster/diagnostics` for storage health and disk usage.

## Shared volume folder listener

Agents can watch a local folder that represents the shared volume and sync filesystem changes to distributed object storage.

- New/modified files: synced as `PUT /v1/storage/{bucket}/{key...}`
- Removed files: synced as `DELETE /v1/storage/{bucket}/{key...}`
- Sync is polling-based to remain cross-platform (including Windows)

Configuration block:

```toml
[shared_volume]
enabled = true
bucket = "shared-volume"
local_dir = "C:/ProgramData/all4one-local/shared" # optional; default: {data_dir}/shared
policy = "warm"
scan_interval_seconds = 2
```
