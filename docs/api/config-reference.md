# Configuration Reference - agent.toml

This document defines the runtime configuration for the All4One agent.

## Load behavior

- File path is provided with `--config`.
- On missing or invalid config, startup fails with exit code `1`.
- Advertised peer host is currently controlled at runtime with the `ALL4ONE_ADVERTISE_HOST` environment variable.

## [node]

- `tier`: `0 | 1 | 2`
- `availability`: `always | cron:<expr> | manual | learned`
- `quorum_participant`: `true | false`
- `data_dir`: writable absolute path
- `reliability_score`: `0.0..1.0`

## [roles]

- `scheduler`: enables job admission and placement
- `executor`: enables job execution
- `storage`: enables object storage/chunk service

## [network]

- `bind_address`: bind interface
- `rest_port`: REST API port
- `grpc_port`: gRPC + SWIM UDP port
- To override the host announced to peers, set `ALL4ONE_ADVERTISE_HOST` before startup

## [discovery]

- `mdns`: enable local LAN discovery
- `seeds`: static bootstrap list (`"ip:port"`)

## [security]

- `mode`: `dev | prod`
- `shared_secret`: enables the shared-secret authorization option (available when `mode = "dev"`)
- `certs_dir`: required for production mTLS

## [executor]

- `max_concurrent_jobs`
- `docker_socket`
- `cgroups_enabled` (Linux)
- output capture limits and memory/CPU limits are enforced by runtime

## [storage]

- `chunk_size_mb`: `1..512`
- `default_policy`: `hot | warm | cold | archive`
- `encryption_at_rest`: `true | false`
- `archive_restore_ttl_hours`

## [gossip]

- `heartbeat_interval_secs`
- `suspect_timeout_secs`
- `offline_timeout_secs`
- `indirect_probe_k`

## [raft]

- `election_timeout_min_ms`
- `election_timeout_max_ms`
- `heartbeat_interval_ms`
- `snapshot_threshold`

## [capabilities]

Node capability declarations and startup checks for Docker, Java, Python, GPU, and executable runtime support.

## [logging]

- `level`: `trace | debug | info | warn | error`
- `format`: `json | text`

## Example

```toml
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
rest_port = 7946
grpc_port = 7947

[security]
mode = "dev"
shared_secret = "compose-secret"
```
