# Agent Architecture

The All4One agent is a single Rust binary composed of independent modules:

- `config`: startup configuration and validation
- `node`: node identity/profile and capability detection
- `discovery`: mDNS + static seed bootstrap
- `gossip`: SWIM membership and liveness
- `scheduler`: job placement engine
- `executor`: runtime adapters (docker/jar/python/wasm/executable)
- `storage`: chunking, indexing, and data policies
- `raft`: embedded consensus for replicated metadata
- `certificates`: CA, node certificates, revocation list
- `api_rest` and `grpc_server`: external and internal APIs

## Design goals

- One binary deployable from 1 to N nodes
- No mandatory central orchestrator
- Clear role separation with low coupling
- Predictable operation on heterogeneous hardware

## Runtime roles

Each node can enable:

- Scheduler
- Executor
- Storage

These roles are independent and configured via `agent.toml`.
