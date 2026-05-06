# scripts/phase3-failure-sim.ps1
#
# Phase 3 - Failure simulation for operational timeline/degradation validation.
#
# Scenario:
# 1) Stop one agent container
# 2) Verify cluster degrades (online node count drops)
# 3) Start the agent container again
# 4) Verify cluster recovers
#
# Usage:
#   ./scripts/phase3-failure-sim.ps1
#   ./scripts/phase3-failure-sim.ps1 -Secret compose-secret -TargetContainer all4one-agent-c

param(
    [string]$ComposeFile = "deploy/compose/docker-compose.phase2.yml",
    [string]$Secret = "compose-secret",
    [string]$TargetContainer = "all4one-agent-c",
    [string]$ProbeNode = "http://localhost:7946",
    [int]$TimeoutSeconds = 90
)

$ErrorActionPreference = "Stop"

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

function Write-Fail([string]$msg) {
    Write-Host "  [FAIL] $msg" -ForegroundColor Red
}

function Get-Nodes() {
    return Invoke-RestMethod -Uri "$ProbeNode/v1/nodes" -Method Get -Headers $Headers
}

function Wait-ForOnlineCount([int]$ExpectedMin, [int]$ExpectedMax, [string]$Label) {
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        try {
            $nodes = Get-Nodes
            if ($nodes.online -ge $ExpectedMin -and $nodes.online -le $ExpectedMax) {
                Write-Pass "$Label reached: online=$($nodes.online)/$($nodes.total)"
                return $nodes
            }
            Write-Info "$Label pending: online=$($nodes.online)/$($nodes.total)"
        } catch {
            Write-Info "$Label pending: probe error ($($_.Exception.Message))"
        }
        Start-Sleep -Seconds 3
    }

    throw "$Label not reached within $TimeoutSeconds seconds"
}

Write-Banner "ALL4ONE - Phase 3 Failure Simulation"
Write-Info "Probe node: $ProbeNode"
Write-Info "Target container: $TargetContainer"

# Ensure cluster starts healthy-ish before chaos
$baseline = Wait-ForOnlineCount -ExpectedMin 3 -ExpectedMax 3 -Label "Baseline 3/3 online"

Write-Banner "Injecting failure"
docker compose -f $ComposeFile stop $TargetContainer | Out-Null
Write-Info "Stopped $TargetContainer"

$degraded = Wait-ForOnlineCount -ExpectedMin 1 -ExpectedMax 2 -Label "Degraded state"

Write-Banner "Recovering node"
docker compose -f $ComposeFile start $TargetContainer | Out-Null
Write-Info "Started $TargetContainer"

$recovered = Wait-ForOnlineCount -ExpectedMin 3 -ExpectedMax 3 -Label "Recovered 3/3 online"

Write-Banner "Failure simulation complete"
Write-Pass "Baseline online=$($baseline.online), degraded online=$($degraded.online), recovered online=$($recovered.online)"
Write-Info "Open $ProbeNode/ and check timeline entries for node status changes during this run."
