# ADR-005: Internal gRPC for Node-to-Node APIs

**Status**: Accepted  
**Date**: 2026-04-08

## Context

Nodes need low-latency, strongly typed, streaming-capable communication.

## Decision

Use internal gRPC for inter-node RPC.

## Reasons

- Contract-first APIs via protobuf
- Efficient binary transport
- Bi-directional streaming support
- Mature Rust ecosystem integration

## Consequences

- Proto/schema evolution discipline is required
- Compatibility testing needed for rolling upgrades
