# Scheduler and Placement

The scheduler decides where jobs run.

## Placement factors

- Resource fit (CPU, memory, optional GPU)
- Required capabilities
- Tier constraints and availability windows
- Current node load and queue depth

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
