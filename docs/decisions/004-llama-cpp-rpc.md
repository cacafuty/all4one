# ADR-004: llama.cpp RPC for Inference Runtime

**Status**: Accepted  
**Date**: 2026-04-08

## Context

Inference workloads require lightweight deployment options across heterogeneous hardware.

## Decision

Use llama.cpp-compatible RPC patterns for selected inference workloads.

## Reasons

- Efficient CPU-first execution profile
- Flexible deployment on constrained nodes
- Good fit for runtime-level integration in agent executors

## Consequences

- Need compatibility testing per model/runtime version
- Additional operational tuning for throughput and latency
