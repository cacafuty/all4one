param(
    [string]$ExamplesRoot = "deploy/compose/examples/phase1-fire-test",
    [string]$Secret = "compose-secret",
    [switch]$AsJson
)

$ErrorActionPreference = "Stop"

$submitTargets = @{
    "agent-a" = "http://localhost:7946"
    "agent-b" = "http://localhost:8946"
    "agent-c" = "http://localhost:9946"
}

function Wait-ClusterReady {
    param([string]$Url, [hashtable]$Headers)

    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        try {
            $response = Invoke-RestMethod -Uri "$Url/v1/nodes" -Headers $Headers -Method Get
            if ($response.total -ge 3 -and $response.online -ge 3) {
                return $response
            }
        }
        catch {
        }
        Start-Sleep -Seconds 2
    }

    throw "Cluster did not reach total=3 and online=3 in time."
}

function Wait-JobTerminal {
    param(
        [string]$BaseUrl,
        [hashtable]$Headers,
        [string]$JobId
    )

    for ($attempt = 0; $attempt -lt 60; $attempt++) {
        $job = Invoke-RestMethod -Uri "$BaseUrl/v1/jobs/$JobId" -Headers $Headers -Method Get
        if ($job.status -in @("completed", "failed", "cancelled")) {
            return $job
        }
        Start-Sleep -Seconds 2
    }

    throw "Job $JobId did not reach terminal state in time."
}

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$examplesPath = Resolve-Path (Join-Path $repoRoot $ExamplesRoot)
$headers = @{ "X-All4One-Secret" = $Secret }

$cluster = Wait-ClusterReady -Url $submitTargets["agent-a"] -Headers $headers
$nodeMap = @{}
foreach ($targetName in $submitTargets.Keys) {
    $health = Invoke-RestMethod -Uri "$($submitTargets[$targetName])/health" -Method Get
    $nodeMap[$health.node_id] = $targetName
}

$jobFiles = Get-ChildItem -Path $examplesPath -Recurse -Filter *.yaml | Sort-Object FullName
$report = @()

foreach ($jobFile in $jobFiles) {
    $targetName = $jobFile.Directory.Name
    if (-not $submitTargets.ContainsKey($targetName)) {
        throw "Unknown target directory '$targetName' for example file '$($jobFile.FullName)'."
    }

    $baseUrl = $submitTargets[$targetName]
    $payload = Get-Content -Path $jobFile.FullName -Raw
    $submitted = Invoke-RestMethod -Uri "$baseUrl/v1/jobs" -Headers $headers -ContentType "application/yaml" -Method Post -Body $payload
    $final = Wait-JobTerminal -BaseUrl $baseUrl -Headers $headers -JobId $submitted.job_id

    $report += [pscustomobject]@{
        Example = $jobFile.BaseName
        SubmittedTo = $targetName
        JobId = $submitted.job_id
        Runtime = (Get-Content -Path $jobFile.FullName | Select-String -Pattern '^runtime:' | Select-Object -First 1).ToString().Split(':')[1].Trim()
        Status = $final.status
        ExitCode = $final.exit_code
        AssignedNodeId = $final.assigned_to
        AssignedNode = $nodeMap[$final.assigned_to]
        Error = $final.error
        SourceFile = $jobFile.FullName.Substring($repoRoot.Path.Length + 1)
    }
}

if ($AsJson) {
    $report | ConvertTo-Json -Depth 4
}
else {
    $report | Format-Table -AutoSize
}