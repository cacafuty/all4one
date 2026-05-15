# Release Notes

## v0.1.11 (Unreleased)

### New Features

#### Distributed Storage with Replication
- **Multi-site object replication** with configurable policies: `hot` (local only), `warm` (2-3 copies), `cold` (erasure-coded), `archive` (compressed + erasure).
- **Automatic failover**: Objects remain accessible even when nodes go down, as long as any replica survives.
- **Storage role gating**: Nodes can opt-in/out of storage duties via `[roles] storage = true/false`.

#### All4One Compression Format
- **Custom container format** (`A4O1` magic header) for all4one objects with integrated version/codec tracking.
- **Legacy zstd support**: Automatically detects and decompresses older zstd-only payloads.
- **Transparent compression**: Policy-driven compression (zstd) applied at write time, decompressed on read.

#### Shared Volume Listener
- **Automatic folder sync**: Configured local directory is monitored for file changes (create/update/delete).
- **Object storage bridge**: Files are automatically uploaded as objects into a designated bucket (default: `shared-volume`).
- **Bidirectional**: Files deleted from folder trigger object deletion; replicas follow replication policy.
- **Multi-node**: Each node can have its own shared folder; all files are replicated and discoverable cluster-wide.

#### Cluster-Wide Storage Explorer
- **Aggregated object listing**: `/v1/storage-explorer/{bucket}` returns merged view of all objects in the cluster.
- **Remote-only visibility**: Objects present on other nodes (not local) are shown with presence marker.
- **Peer query in parallel**: Fast aggregation by querying online peers concurrently.
- **Presence tracking**: Each object reports `local_present` and `available_on` (list of node endpoints).

#### Dashboard Enhanced
- **Shared Volume Explorer UI section**: Displays all cluster-wide objects in the shared-volume bucket.
- **File metadata display**: Size, storage policy, last access time, presence (local/remote), and available endpoints.
- **One-click downloads**: Download links work through the current node (read-through for remote files).
- **Prefix filtering**: Optional client-side prefix filter for large buckets.
- **Automatic refresh**: Configurable auto-refresh with manual reload button.

### Technical Improvements

- **Compression lifecycle**: Objects transition from hot (uncompressed) → warm (compressed) → cold (erasure) → archive based on recency.
- **Read-through caching**: Fetching a remote object caches it locally for future fast access.
- **Metadata persistence**: Object access timestamps tracked for policy-driven lifecycle decisions.
- **Storage health monitoring**: Cluster reports accessible storage capacity and object count via REST API.

### Backward Compatibility

- ✅ All existing APIs remain unchanged and compatible.
- ✅ Legacy zstd-compressed objects auto-decompress.
- ✅ Nodes without `storage=true` skip listener and replication overhead.
- ✅ Existing jobs and scheduler behavior unaffected.

### Documentation

- [docs/architecture/storage.md](docs/architecture/storage.md) — Detailed replication and lifecycle behavior.
- [docs/api/rest-api.md](docs/api/rest-api.md) — Storage explorer and shared-volume endpoints.
- [docs/guides/node-setup.md](docs/guides/node-setup.md) — Steam Deck and multi-node setup.

### Platform Support

- ✅ Linux x86-64 (Ubuntu, SteamOS, etc.)
- ✅ Linux ARM64 (Raspberry Pi, etc.)
- ✅ macOS ARM64 (Apple Silicon)
- ✅ Windows x86-64

### Known Limitations

- Raft consensus (Phase 3+) not yet enabled; quorum mode is advisory only.
- Erasure coding recovery not yet implemented (shards stored independently).
- No built-in backup/export yet; rely on multi-site replication for durability.

---

## v0.1.10

- Executor improvements
- Scheduler stability fixes
- Initial storage module skeleton

---

## v0.1.9

- Gossip-based peer discovery (SWIM protocol)
- Certificate enrollment (Phase 2)
- REST API foundation
- Docker job executor
