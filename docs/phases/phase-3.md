# Phase 3 - Operational UI and Cluster Visibility

Goal: provide a clear, real-time operational view of the cluster so operators can see what is happening at any moment.

## Scope

- Live cluster topology (nodes, roles, tiers, health)
- Job lifecycle dashboard (queued, running, completed, failed)
- Storage visibility (bucket/object activity, shard replication status)
- Event timeline (membership changes, failures, recoveries)
- Basic operational controls (safe retries, job cancel, node drain request)

## Exit criteria

- Operators can identify cluster state, active incidents, and workload status from one UI
- UI data matches API/cluster state with bounded freshness
- Critical operational actions are auditable and role-gated

## Phase 2 + Phase 3 combined validation (multi-device)

Use this when you want to validate durable storage behavior (Phase 2) and operational visibility (Phase 3) at the same time.

### Example topology

- Device A: node A (or compose mapped to `:7946`)
- Device B: node B (or compose mapped to `:8946`)
- Device C: node C (or compose mapped to `:9946`)

### Variant: base machine + 3 isolated Docker agents + Steam Deck

Use this when the base machine hosts the whole Phase 2 compose lab and the Steam Deck joins over the local network as an extra Linux node.

- Base machine: `docker-compose.phase2.yml` with `agent-a`, `agent-b`, and `agent-c`
- Each Docker agent keeps its own named volume, so there is no shared filesystem between them
- Steam Deck: extra Tier 2 executor reachable at `http://<STEAM_DECK_IP>:7946`

For the Steam Deck-side setup, see [Node Setup Guide](../guides/node-setup.md#steam-deck-example-join-a-local-3-container-cluster).

### Step 1 - Generate Phase 2 storage activity

From any machine with access to your cluster endpoints:

```powershell
./scripts/phase2-storage-test.ps1 -Secret compose-secret
```

This creates write/read/delete/list traffic and shard replication events that should appear in the operational UI.

### Step 2 - Validate Phase 3 operational UI and API summaries

Run:

```powershell
./scripts/phase3-ops-ui-check.ps1 -Nodes "http://10.0.0.21:7946,http://10.0.0.22:7946,http://10.0.0.23:7946" -Secret compose-secret -SubmitDemoJob
```

If you are validating Docker host mappings locally:

```powershell
./scripts/phase3-ops-ui-check.ps1 -Nodes "http://localhost:7946,http://localhost:8946,http://localhost:9946" -Secret compose-secret -SubmitDemoJob
```

If you keep the three Docker agents on one base machine and add a Steam Deck on the LAN:

```powershell
./scripts/phase3-ops-ui-check.ps1 -Nodes "http://<HOST_IP>:7946,http://<HOST_IP>:8946,http://<HOST_IP>:9946,http://<STEAM_DECK_IP>:7946" -Secret compose-secret -SubmitDemoJob
```

### Step 3 - Open the UI from different devices

Open these pages in browsers on each device:

- `http://10.0.0.21:7946/`
- `http://10.0.0.22:7946/`
- `http://10.0.0.23:7946/`

For the base machine + Steam Deck variant, open the three base-machine endpoints from the Steam Deck browser and optionally also `http://<STEAM_DECK_IP>:7946/` for the local node view.

Provide the same `X-All4One-Secret` value in the UI secret field when REST authorization is enabled.

### What to verify

- Node topology reflects the same online/offline state on all devices.
- In the Steam Deck variant, all UIs converge to the same `4`-node topology after the extra node joins.
- Job lifecycle counters change while demo jobs run.
- Distributed state panel reports quorum and synchronization status.
- Storage panel reflects object count and storage accessibility.
- Operational timeline shows recent node/job changes after refresh cycles.

## Three-step operational validation

1. Generate Phase 2 traffic:

```powershell
./scripts/phase2-storage-test.ps1 -Secret compose-secret
```

2. Check Phase 3 operational UI/API summary:

```powershell
./scripts/phase3-ops-ui-check.ps1 -Nodes "http://localhost:7946,http://localhost:8946,http://localhost:9946" -Secret compose-secret -SubmitDemoJob
```

3. Simulate node failure/recovery and validate timeline/degradation:

```powershell
./scripts/phase3-failure-sim.ps1 -Secret compose-secret -TargetContainer all4one-agent-c -ProbeNode "http://localhost:7946"
```

Expected outcome:

- `online` count drops during failure and returns after recovery.
- `/v1/ops/events` emits node status transitions.
- UI timeline at `/` shows live operational events without relying only on inferred polling diffs.
