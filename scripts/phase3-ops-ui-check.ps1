# scripts/phase3-ops-ui-check.ps1
#
# Phase 3 - Operational UI smoke check for multi-device clusters.
#
# What it does:
# - Queries /v1/nodes, /v1/cluster/diagnostics, /v1/jobs on each provided node
# - Prints a compact operational summary
# - Optionally submits a small demo job to generate UI activity
#
# Usage:
#   ./scripts/phase3-ops-ui-check.ps1
#   ./scripts/phase3-ops-ui-check.ps1 -Nodes "http://10.0.0.21:7946,http://10.0.0.22:7946,http://10.0.0.23:7946" -Secret compose-secret
#   ./scripts/phase3-ops-ui-check.ps1 -SubmitDemoJob

param(
    [string]$Nodes = "http://localhost:7946,http://localhost:8946,http://localhost:9946",
    [string]$Secret = "compose-secret",
    [switch]$SubmitDemoJob
)

$ErrorActionPreference = "Stop"

$NodeList = $Nodes.Split(",") | ForEach-Object { $_.Trim() } | Where-Object { $_ -ne "" }
if ($NodeList.Count -eq 0) {
    throw "No node endpoints provided."
}

$Headers = @{}
if ($Secret -ne "") {
    $Headers["X-All4One-Secret"] = $Secret
}

function Write-Banner([string]$msg) {
    Write-Host ""
    Write-Host $msg -ForegroundColor DarkCyan
}

function Write-Info([string]$msg) {
    Write-Host "  [INFO] $msg" -ForegroundColor Cyan
}

function Write-Pass([string]$msg) {
    Write-Host "  [PASS] $msg" -ForegroundColor Green
}

function Write-Warn([string]$msg) {
    Write-Host "  [WARN] $msg" -ForegroundColor Yellow
}

function Invoke-JsonGet([string]$uri) {
    return Invoke-RestMethod -Uri $uri -Headers $Headers -Method Get
}

function Submit-DemoJob([string]$node) {
    $body = @"
runtime: docker
source: python:3.11-slim
command: ["python", "-c", "print('phase3-ui-demo')"]
resources:
  cpu_cores: 1
  memory_mb: 128
"@

    try {
        $resp = Invoke-RestMethod -Uri "$node/v1/jobs" -Headers $Headers -Method Post -Body $body -ContentType "application/yaml"
        Write-Pass "Submitted demo job on $node job_id=$($resp.job_id)"
    } catch {
        Write-Warn "Could not submit demo job on $node ($($_.Exception.Message)). UI checks continue."
    }
}

Write-Banner "ALL4ONE - Phase 3 Operational UI Smoke Check"
Write-Info "Nodes: $($NodeList -join ', ')"

foreach ($node in $NodeList) {
    try {
        $diag = Invoke-JsonGet "$node/v1/cluster/diagnostics"
        $nodes = Invoke-JsonGet "$node/v1/nodes"
        $jobs = Invoke-JsonGet "$node/v1/jobs"

        Write-Pass "$node reachable"
        Write-Info "Cluster: online=$($nodes.online)/$($nodes.total) quorumHealthy=$($diag.cluster_info.quorum_healthy) stateSync=$($diag.distributed_state.cluster_synchronized)"

        $queued = ($jobs.jobs | Where-Object { $_.status -eq "queued" }).Count
        $running = ($jobs.jobs | Where-Object { $_.status -eq "running" }).Count
        $completed = ($jobs.jobs | Where-Object { $_.status -eq "completed" }).Count
        $failed = ($jobs.jobs | Where-Object { $_.status -eq "failed" }).Count

        Write-Info "Jobs: total=$($jobs.total) queued=$queued running=$running completed=$completed failed=$failed"
        Write-Info "Open UI: $node/"
    } catch {
        Write-Warn "$node failed check: $($_.Exception.Message)"
    }
}

if ($SubmitDemoJob) {
    Write-Banner "Submitting demo jobs for UI activity"
    foreach ($node in $NodeList) {
        Submit-DemoJob -node $node
    }
}

Write-Banner "Phase 3 UI smoke check complete"
