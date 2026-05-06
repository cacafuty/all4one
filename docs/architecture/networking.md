# Networking

All4One networking combines REST, gRPC, and SWIM membership traffic.

## Ports

- REST API: `:7946`
- gRPC internal API: `:7947` (TCP)
- SWIM gossip: `:7947` (UDP)

## Protocols

- REST for client-facing operations and diagnostics
- gRPC for node-to-node RPC and delegated execution
- SWIM for liveness and membership dissemination

## Discovery

- mDNS for local network discovery
- Static `seeds` for deterministic bootstrap

## Trust Bootstrap (Phase 2)

- Cluster bootstrap node initializes embedded CA material in its local data directory.
- A new node can join by knowing:
	- one reachable seed gRPC address, and
	- the expected cluster CA (returned in enrollment response and persisted locally).
- Enrollment is performed through gRPC `Join`, which returns node certificate/key plus CA certificate for local trust store setup.
- Join authorization can also enforce an optional `shared_secret` gate in local clusters.
- Production deployments should prefer CA-led trust policy and avoid shared secrets for node admission.

## Operational guidance

- Open both TCP/UDP on configured ports
- Keep node clocks synchronized (NTP)
- Prefer stable advertise addresses in mixed networks
