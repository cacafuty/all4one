# ADR-002: No central orchestration node

**Status**: Accepted
**Date**: 2026-04-08

---

## Context

All4One is sold to companies with heterogeneous existing hardware — from Raspberry
Pi to rack servers. The typical customer does not want to designate or maintain a
dedicated "master server". Many customers have hardware with partial availability
(laptops that shut down, office PCs outside business hours).

The fundamental design question: who coordinates the cluster?

---

## Decision

**No mandatory orchestration node exists.** Any node can receive jobs and
coordinate. The cluster works with whatever is available at any given moment.
Consensus (Raft) is embedded in each agent, and the nodes that participate in quorum
are those with `quorum_participant = true` — there is no special node for this.

---

## Reasons

### Minimum functional cluster of a single node

With a mandatory central node, the customer needs at least two machines:
the master and a worker. Without a central node, a developer can install the
agent on their laptop and have a functional cluster for local development.

In sales: the "install this and in 5 minutes you have your cluster" demo is only
possible without a central node.

### No SPOF (Single Point of Failure)

If the central node goes down, the entire system stops. In the distributed model:
- While there is quorum (majority of `quorum_participant=true` nodes online),
  the cluster operates normally.
- If quorum is lost, the cluster enters degraded mode (no Raft writes,
  but already running jobs continue and schedulers without Raft
  keep placing jobs best-effort).

### Temporal node availability is a design parameter

Tier 1 and Tier 2 nodes have partial availability by design. The central-node
model would need to treat node absence as error cases to handle.
In the distributed model, temporal availability is a feature the scheduler
knows about and plans around:

```
availability = "cron:0 9-18 * * 1-5"
```

The scheduler filters this node out of the 18:00–09:00 window instead of
attempting to connect to a node it knows is unavailable.

---

## Rejected alternatives

### Master-worker architecture (Kubernetes-style)

**Rejected because**:

1. **Infrastructure dependency**: the master must be an always-available server.
   Many Starter/Business target customers don't have such a server
   or don't want to manage it.

2. **SPOF**: the master going down stops scheduling. In K8s, the control plane
   is the critical point — master high availability requires 3+ dedicated nodes
   (etcd cluster + redundant API server).

3. **Operational overhead**: customers buy All4One to avoid managing
   infrastructure. A separate master is infrastructure to manage.

### External coordinator (ZooKeeper / etcd)

**Rejected because**:

1. **External dependency**: requires installing and maintaining ZooKeeper or etcd
   separately. Contradicts the principle of "a single binary with no dependencies".

2. **etcd license**: etcd uses Apache 2.0, but requires separate installation and
   management. Adds an infrastructure piece that can fail independently.

3. **Coordination latency**: every operation requiring consensus (RegisterJob,
   PutChunkMap) needs an extra network call to the external coordinator. With
   embedded Raft, writing to the log is local to the leader node.

---

## Accepted trade-offs

### Greater complexity per agent

Each agent implements scheduling, gossip, and consensus. This makes the agent
more complex than a simple worker that only executes master commands.

**Mitigation**: modules are independent with well-defined interfaces.
An Android node only activates `storage` — it does not implement scheduler or Raft.

### More complex debugging

Without centralized logs, correlating events across nodes requires:
- Consistent request IDs (`X-Request-Id` in REST, `correlation_id` in gRPC).
- Synchronized timestamps (NTP required in production).
- External log aggregation (Loki, Elasticsearch) for production.

**Mitigation**: structured JSON tracing with `request_id` enables post-hoc
correlation. The admin web UI (Phase 3) aggregates metrics from all nodes.

### Scheduling race condition in Phase 1

Without Raft, if the same job with an explicit `id` arrives at two schedulers
simultaneously, it may execute twice.

**Phase 1 mitigation**: jobs must be idempotent. If the `id` already exists
in local state, the current state is returned without relaunching.

**Definitive solution in Phase 2**: `RaftCommand::RegisterJob` guarantees the
job is registered exactly once in the quorum. The second attempt receives
a duplicate key error.
