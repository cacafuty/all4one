# ADR-006: Embedded PKI and mTLS

**Status**: Accepted  
**Date**: 2026-04-08

## Context

Production clusters require strong identity and encrypted transport without external PKI dependency.

## Decision

Use an embedded cluster CA, node enrollment flow, CRL-based revocation, and mTLS between nodes.

## Reasons

- Strong node identity and trust boundaries
- Immediate revocation support
- Self-contained deployment model

## Consequences

- Certificate lifecycle operations become part of agent responsibilities
- Enrollment/revocation flows require robust auditing and monitoring
