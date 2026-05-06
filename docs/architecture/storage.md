# Distributed Storage

All4One storage provides an S3-like object API backed by local chunk files, with peer-to-peer shard distribution across storage nodes.

## Core features

- Chunking, hashing (SHA-256 per shard and full object)
- Tiered policy (`hot` / `warm` / `cold` / `archive`)
- Compression with zstd (level varies by policy)
- Erasure coding (data shards + parity shards; real RS planned)
- Metadata index via `sled` embedded DB
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
2. The local storage node compresses, erasure-codes, and writes shards to `{data_dir}/chunks/{bucket}/`.
3. Object metadata is written to the local sled index (`{data_dir}/objects.db`).
4. The node reads all shards and fans them out to every online peer with `capabilities.storage_node = true` via the gRPC `TransferChunk` RPC (fire-and-forget, eventual consistency).

## Read path

1. REST client sends `GET /v1/storage/{bucket}/{key}`.
2. The node reads shards from local disk, reconstructs the object via `decode_erasure`, decompresses, and verifies SHA-256.
3. If a local data shard is missing, future work will fetch it from a peer via the `FetchChunk` gRPC RPC.

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
