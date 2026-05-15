# Job Specification

`JobSpec` is the workload contract submitted to `POST /v1/jobs`.

## Core fields

- `id`: optional UUID for idempotent submission
- `runtime`: `docker | jar | python | executable | wasm | inference_group`
- `source`: image, path, module, or model URI depending on runtime
- `command`: runtime arguments

## Resources

- `cpu_cores` (required)
- `memory_mb` (required)
- `gpu_min_memory_mb` (optional)
- `cuda_min` (optional)
- `max_duration_minutes` (optional)

## Data mounts

- `data[]` supports source URI to target mount mapping

## Constraints

- `tier_min`
- `tier_max`
- `requires_capabilities` (docker/gpu/python/java/wasm)
- optional locality hints

## Retry and scheduling options

- retry policy
- backoff strategy
- timeout policy

## Runtime notes

- Docker: hard memory/CPU constraints through container flags
- Non-docker on Unix: best-effort hard memory limit via `RLIMIT_AS`
- Non-docker on Windows: Job Object enforcement is planned

## Executable portability rules

When `runtime` is `executable`, the scheduler validates command portability against target node OS:

- Windows targets only: `.exe`, `.bat`, `.cmd`, `cmd`, `cmd.exe`, `powershell`, `powershell.exe`, `pwsh`, `pwsh.exe`
- Linux targets only: `.sh`, `sh`, `bash`

If a job fails on one node and retries are available, it is re-queued for another compatible node. The failed node is excluded for that job retry sequence.

## Minimal example

```yaml
runtime: docker
source: alpine:3.20
command: ["sh", "-c", "echo hello"]
resources:
  cpu_cores: 1
  memory_mb: 128
constraints:
  tier_min: 0
  requires_capabilities:
    docker: true
```
