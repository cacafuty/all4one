# Lifecycle Engine

The lifecycle engine automates data and placement transitions across storage tiers.

## Responsibilities

- Track object/job heat and access frequency
- Promote/demote data across `hot/warm/cold/archive`
- Trigger background migration and rebalance workflows
- Enforce retention and restore windows

## Inputs

- Access telemetry
- Node availability and capacity
- Policy constraints and SLA targets

## Outputs

- Migration plans
- Queue of replication/erasure tasks
- Operational events for observability

## Safety rules

- Never violate minimum recoverability thresholds
- Keep at least one durable copy on stable tiers
- Apply transitions gradually with rollback options
