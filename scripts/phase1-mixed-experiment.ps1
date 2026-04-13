param(
    [string]$AgentExePath = "",
    [string]$ConfigPath = "deploy/compose/configs/agent-win-local.toml",
    [string]$ApiBase = "http://localhost:7946",
    [string]$Secret = "compose-secret",
    [string]$AdvertiseHost = "host.docker.internal",
    [switch]$StartLocal
)

$ErrorActionPreference = "Stop"

function Invoke-AgentApi {
    param(
        [string]$Method,
        [string]$Path,
        [object]$Body = $null,
        [string]$ContentType = "application/json"
    )

    $headers = @{ "X-All4One-Secret" = $Secret }
    $uri = "$ApiBase$Path"

    if ($null -eq $Body) {
        return Invoke-RestMethod -Method $Method -Uri $uri -Headers $headers
    }

    return Invoke-RestMethod -Method $Method -Uri $uri -Headers $headers -Body $Body -ContentType $ContentType
}

function Wait-NodesConverged {
    param([int]$TimeoutSeconds = 60)

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        try {
            $nodes = Invoke-AgentApi -Method GET -Path "/v1/nodes"
            if ($nodes.total -ge 3 -and $nodes.online -ge 3) {
                return $nodes
            }
        } catch {
            Start-Sleep -Seconds 1
            continue
        }
        Start-Sleep -Seconds 1
    }

    throw "Timeout waiting for compose convergence (expected >=3 online nodes)."
}

function Wait-NoOnlineTier2 {
    param(
        [int]$TimeoutSeconds = 45,
        [int]$MinOnlineNodes = 3
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        $nodes = Invoke-AgentApi -Method GET -Path "/v1/nodes"
        $onlineTier2 = @($nodes.nodes | Where-Object {
            [string]$_.status -eq "online" -and [int]$_.profile.tier -ge 2
        }).Count

        if ($nodes.online -ge $MinOnlineNodes -and $onlineTier2 -eq 0) {
            return
        }

        Start-Sleep -Seconds 1
    }

    throw "Timeout waiting for zero online Tier 2 nodes before probe submission."
}

function Wait-JobTransition {
    param(
        [string]$JobId,
        [string[]]$TargetStates,
        [int]$TimeoutSeconds = 30
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        $job = Invoke-AgentApi -Method GET -Path "/v1/jobs/$JobId"
        if ($TargetStates -contains [string]$job.status) {
            return $job
        }
        Start-Sleep -Seconds 1
    }

    throw "Timeout waiting for job $JobId to reach one of: $($TargetStates -join ', ')."
}

Write-Host "[1/5] Starting compose cluster"
docker compose -f deploy/compose/docker-compose.yml up --build -d | Out-Host

Write-Host "[2/5] Waiting for 3 Docker nodes online"
$nodes = Wait-NodesConverged -TimeoutSeconds 90
$nodes | ConvertTo-Json -Depth 8 | Out-Host

if ($StartLocal) {
    Write-Host "Ensuring no online Tier 2 node before queue probe"
    Wait-NoOnlineTier2 -TimeoutSeconds 45 -MinOnlineNodes 3
}

Write-Host "[3/5] Submitting tier_min=2 probe job (must stay queued until local Windows node appears)"
$probePayload = @'
runtime: executable
source: cmd
command: ["/C", "echo hello-from-tier2"]
resources:
  cpu_cores: 1
  memory_mb: 128
constraints:
  tier_min: 2
'@
$probe = Invoke-AgentApi -Method POST -Path "/v1/jobs" -Body $probePayload -ContentType "application/yaml"
$probe | ConvertTo-Json -Compress | Out-Host

$probeJob = Invoke-AgentApi -Method GET -Path "/v1/jobs/$($probe.job_id)"
if ([string]$probeJob.status -ne "queued") {
    throw "Expected probe job to be queued before Tier 2 appears, got: $($probeJob.status)"
}
Write-Host "Probe job is queued as expected: $($probe.job_id)"

$localProc = $null
if ($StartLocal) {
    if ([string]::IsNullOrWhiteSpace($AgentExePath)) {
        throw "-StartLocal requires -AgentExePath"
    }
    if (-not (Test-Path $AgentExePath)) {
        throw "Agent executable not found at: $AgentExePath"
    }
    if (-not (Test-Path $ConfigPath)) {
        throw "Config file not found at: $ConfigPath"
    }

    Write-Host "[4/5] Starting local Windows agent"
    $env:ALL4ONE_ADVERTISE_HOST = $AdvertiseHost
    $localProc = Start-Process -FilePath $AgentExePath -ArgumentList @("start", "--config", $ConfigPath) -PassThru
    Write-Host "Local agent PID: $($localProc.Id)"
} else {
    Write-Host "[4/5] Local agent start skipped (run this script with -StartLocal -AgentExePath <path>)"
}

Write-Host "[5/5] Waiting probe job transition after Tier 2 node appears"
try {
    $moved = Wait-JobTransition -JobId $probe.job_id -TargetStates @("running", "completed", "failed") -TimeoutSeconds 20
    $moved | ConvertTo-Json -Compress | Out-Host
} catch {
    Write-Warning $_
}

Write-Host "Done. Next: run the Docker memory-limit job from deploy/compose/README.md and capture evidence on Windows."
