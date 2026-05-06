# Phase 2 - Durable Data and Trust

Goal: add replicated metadata, distributed storage policies, and production security controls.

## Scope

- Embedded Raft for metadata consensus
- Storage policies (`hot/warm/cold/archive`)
- Certificate lifecycle (CA, enroll, revoke)
- mTLS-ready transport model

## Implemented ✅

- Raft module with `SledLogStore` + `SledStateMachine` and full unit tests
- Certificate manager: CA creation, node cert issuance, CRL, idempotent init
- Storage engine: chunk split/compress/erasure-code/store + sled index
- **REST storage API**: `PUT/GET/DELETE /v1/storage/{bucket}/{key}`, `GET/POST /v1/storage/{bucket}`
- **gRPC chunk transport**: `TransferChunk` and `FetchChunk` RPCs for inter-node shard distribution
- **Cluster PKI bootstrap/enrollment**:
	- Bootstrap node (tier 0 or no seeds) initializes local CA material
	- Joining nodes use a known seed gRPC endpoint (`Join`) to obtain node cert/key + cluster CA cert
	- Join authorization supports two paths:
		- CA-based bootstrap trust path (recommended/default)
		- Optional `shared_secret` check for simpler local/lab enrollment
	- Enrollment bundle is persisted under each node data directory (`certs/node.crt`, `certs/node.key`, `certs/ca.crt`)
- `storage_node` capability flag advertised in cluster gossip
- Shard fan-out on write: after local write, shards are pushed to all online storage peers

## Remaining

- End-to-end mTLS activation (cert issuance/enrollment is implemented; transport-level cert verification still pending)
- Real Reed-Solomon erasure coding (current parity shards are zero-filled; recovery is partial)
- Repair/scrub automation (background daemon detecting and repairing missing shards)
- Multipart upload support
