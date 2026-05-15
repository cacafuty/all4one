# Scheduler and Placement

The scheduler decides where jobs run.

## Placement factors

- Resource fit (CPU, memory, optional GPU)
- Required capabilities
- Tier constraints and availability windows
- Current node load and queue depth
- Live running-job count per node (least-running first)
- Executable and shell compatibility by node operating system

### Current balancing policy

When multiple nodes are eligible, placement chooses the node with fewer jobs currently in `running` state. Ties are broken deterministically by node ID.

For `runtime=executable`, scheduler also enforces OS compatibility:

- Windows-only commands (`cmd`, `pwsh`, `.exe`, `.bat`, `.cmd`) are routed only to nodes advertising `operating_system=windows`.
- Unix shell commands (`sh`, `bash`, `.sh`) are routed only to nodes advertising `operating_system=linux`.

If a node has not reported `operating_system` yet (empty metadata), executable jobs are allowed as a temporary fallback. Known OS values still enforce strict compatibility.

If a node fails terminal execution for a job, the job is re-queued and retried on another eligible node, excluding nodes already attempted for that job.

### Memory constraints and resource visibility

Each agent reports detected system memory to the scheduler. Agents may optionally declare a `memory_limit_mb` in their executor configuration to cap available memory for scheduling purposes:

```toml
[executor]
memory_limit_mb = 4096  # Report only 4096 MB available to scheduler, even if system has more
```

**Effective available memory** = `min(detected_system_memory, config_memory_limit)`

For example:
- System has 16 GB detected
- Config sets `memory_limit_mb = 4096`
- Scheduler sees 4 GB available for that agent

This allows operators to:
- Reserve system RAM for background processes
- Partition large machines into logical resource domains
- Prevent job starvation from memory exhaustion

Memory is currently visible in cluster telemetry (`GET /v1/cluster/diagnostics`) but is not a hard constraint on job dispatch. Future phases may add memory-aware placement rules.

## Flow

1. Validate incoming `JobSpec`
2. Build eligible candidate set
3. Score candidates
4. Dispatch local or remote execution
5. Track state transitions and retries

## Failure behavior

- Retry when candidate becomes unavailable
- Re-route delegated jobs when safe
- Preserve idempotency for repeated submissions
- On terminal failure, re-queue to a different node (never retry on the same failed node)

## Load Balancing Validation

Tested with 10 concurrent jobs (3-second runtime each) across 2-node cluster:
- Host (tier=0, max_concurrent=2, Windows executor)
- Steam Deck (tier=1, max_concurrent=1, Linux executor)

**Results:**
- Job 1 → Host (0 running on either node, host picked first by UUID)
- Job 2 → Deck (Host=1 running, Deck=0 running, Deck selected by least-load rule)
- Job 3 → Host (Both=1 running, tie-break to host by UUID)
- Jobs 4-10 → Deck (Distributed to Deck while Host near capacity)

**Conclusion:** Scheduler correctly counts running jobs per node and routes to least-loaded candidate. Load distribution is observable and functional.

## Cluster telemetry visibility

Cluster APIs expose live telemetry per node (idle/running/queued, memory estimates, freshness from heartbeat) to improve operator visibility.

Current policy: these telemetry signals are visibility-first and can be used later to refine placement, but are not mandatory hard filters yet.
