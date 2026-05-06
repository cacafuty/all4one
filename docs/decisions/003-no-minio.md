# ADR-003: Do Not Adopt MinIO

**Status**: Accepted  
**Date**: 2026-04-08

## Context

The platform needs integrated distributed object storage tightly coupled with scheduler and lifecycle behavior.

## Decision

All4One will implement and own its storage module instead of embedding MinIO.

## Reasons

- Product/control requirements exceed simple S3 compatibility.
- Deep coupling with tier-aware scheduling and cluster lifecycle is required.
- Licensing and redistribution constraints are unfavorable for the intended product model.

## Consequences

- Higher implementation and maintenance effort.
- Full control over behavior, roadmap, and integration depth.
