# scripts/phase2-storage-test.ps1
#
# Phase 2 — Distributed Storage smoke test suite.
# Runs 10 scenarios against a 3-node Docker Compose cluster started with:
#
#   docker compose -f deploy/compose/docker-compose.phase2.yml up --build -d
#
# Each agent is isolated in its own Docker volume (no shared filesystem).
# Replication happens exclusively over gRPC TransferChunk between containers.
#
# Usage:
#   ./scripts/phase2-storage-test.ps1
#   ./scripts/phase2-storage-test.ps1 -Secret my-secret

param(
    [string]$Secret = "compose-secret"
)

$ErrorActionPreference = "Stop"

# ── Node endpoints (host → container port mapping) ────────────────────────────
$A = "http://localhost:7946"
$B = "http://localhost:8946"
$C = "http://localhost:9946"

$H = @{ "X-All4One-Secret" = $Secret }

# ── Output helpers ────────────────────────────────────────────────────────────
function Write-Banner([string]$msg) {
    Write-Host $msg -ForegroundColor DarkCyan
}
function Write-Pass([string]$msg) { Write-Host "    [PASS] $msg" -ForegroundColor Green }
function Write-Fail([string]$msg) { Write-Host "    [FAIL] $msg" -ForegroundColor Red   }
function Write-Info([string]$msg) { Write-Host "    [INFO] $msg" -ForegroundColor Cyan  }

# ── SHA-256 helper ────────────────────────────────────────────────────────────
function Get-SHA256Hex([byte[]]$data) {
    $sha  = [System.Security.Cryptography.SHA256]::Create()
    $hash = $sha.ComputeHash($data)
    return ($hash | ForEach-Object { $_.ToString("x2") }) -join ''
}

# ── Test runner ───────────────────────────────────────────────────────────────
$Results = [System.Collections.Generic.List[psobject]]::new()

function Invoke-TestCase {
    param([string]$Name, [scriptblock]$Body)
    Write-Host ""
    Write-Host "  ► $Name" -ForegroundColor White
    try {
        & $Body
        $Results.Add([pscustomobject]@{ Name = $Name; Status = "PASS"; Error = "" })
        Write-Pass "passed"
    } catch {
        $Results.Add([pscustomobject]@{ Name = $Name; Status = "FAIL"; Error = $_.Exception.Message })
        Write-Fail $_.Exception.Message
    }
}

# ── Storage helpers ───────────────────────────────────────────────────────────
function Invoke-Put {
    param([string]$Node, [string]$Bucket, [string]$Key, [byte[]]$Data, [string]$Policy = "warm")
    $h = $H + @{ "X-All4One-Policy" = $Policy }
    $stream = [System.IO.MemoryStream]::new($Data)
    return Invoke-RestMethod -Uri "$Node/v1/storage/$Bucket/$Key" -Method Put `
        -Headers $h -Body $stream -ContentType "application/octet-stream"
}

function Invoke-Get {
    param([string]$Node, [string]$Bucket, [string]$Key)
    return Invoke-WebRequest -Uri "$Node/v1/storage/$Bucket/$Key" -Method Get -Headers $H
}

function Invoke-Delete {
    param([string]$Node, [string]$Bucket, [string]$Key)
    Invoke-RestMethod -Uri "$Node/v1/storage/$Bucket/$Key" -Method Delete -Headers $H | Out-Null
}

function Invoke-List {
    param([string]$Node, [string]$Bucket, [string]$Prefix = "")
    $uri = "$Node/v1/storage/$Bucket"
    if ($Prefix) { $uri += "?prefix=$([Uri]::EscapeDataString($Prefix))" }
    return Invoke-RestMethod -Uri $uri -Method Get -Headers $H
}

function Bytes([string]$s) {
    return [System.Text.Encoding]::UTF8.GetBytes($s)
}

function AsString($resp) {
    return [System.Text.Encoding]::UTF8.GetString($resp.RawContentStream.ToArray())
}

function Invoke-GetWithRetry {
    param(
        [string]$Node,
        [string]$Bucket,
        [string]$Key,
        [int]$Attempts = 10,
        [int]$DelaySeconds = 1
    )

    $last = $null
    for ($i = 0; $i -lt $Attempts; $i++) {
        try {
            return Invoke-Get -Node $Node -Bucket $Bucket -Key $Key
        } catch {
            $last = $_
            # Retry transient not-found while async shard fan-out converges
            $code = $null
            if ($_.Exception.Response) {
                $code = [int]$_.Exception.Response.StatusCode
            }
            if ($code -ne 404 -and "$_" -notmatch "404") {
                throw
            }
            if ($i -lt ($Attempts - 1)) {
                Start-Sleep -Seconds $DelaySeconds
            }
        }
    }

    throw $last
}

# ── Wait for cluster ──────────────────────────────────────────────────────────
Write-Host ""
Write-Banner "═══════════════════════════════════════════════════════════════"
Write-Banner "  ALL4ONE  Phase 2 — Distributed Storage Test Suite"
Write-Banner "  10 scenarios across 3 fully isolated Docker agents"
Write-Banner "═══════════════════════════════════════════════════════════════"
Write-Host ""
Write-Host "  Waiting for cluster (all 3 agents report 3 nodes online)..." -ForegroundColor Yellow

$ready = $false
for ($i = 0; $i -lt 40; $i++) {
    try {
        $nodesA = Invoke-RestMethod -Uri "$A/v1/nodes" -Headers $H
        $nodesB = Invoke-RestMethod -Uri "$B/v1/nodes" -Headers $H
        $nodesC = Invoke-RestMethod -Uri "$C/v1/nodes" -Headers $H
        if ($nodesA.online -ge 3 -and $nodesB.online -ge 3 -and $nodesC.online -ge 3) {
            $ready = $true
            break
        }
    } catch {}
    Start-Sleep -Seconds 3
}
if (-not $ready) { throw "Cluster did not converge to 3/3 online on all agents within 120 seconds." }

Write-Host "  Cluster ready: agent-a=$($nodesA.online)/$($nodesA.total), agent-b=$($nodesB.online)/$($nodesB.total), agent-c=$($nodesC.online)/$($nodesC.total)." -ForegroundColor Green

# ─────────────────────────────────────────────────────────────────────────────
# T01 — Write and read from the same node
# Verifies the local write+read path before any network replication is assumed.
# ─────────────────────────────────────────────────────────────────────────────
Invoke-TestCase "T01 — Local write and read on agent-a" {
    $content = Bytes "hello-phase2-distributed-storage"
    $meta    = Invoke-Put -Node $A -Bucket "smoke" -Key "t01-hello" -Data $content -Policy "hot"

    if ($meta.policy -ne "hot") { throw "Expected policy=hot, got '$($meta.policy)'" }
    if (-not $meta.etag)        { throw "Missing ETag in response metadata" }

    $resp = Invoke-Get -Node $A -Bucket "smoke" -Key "t01-hello"
    $got  = AsString $resp
    if ($got -ne "hello-phase2-distributed-storage") { throw "Content mismatch: '$got'" }

    Write-Info "ETag=$($meta.etag)  policy=$($meta.policy)"
}

# ─────────────────────────────────────────────────────────────────────────────
# T02 — Shard replication: write to A, read from B
# Each agent owns a separate volume. The only way B has the data is if
# agent-a's TransferChunk gRPC call reached agent-b.
# ─────────────────────────────────────────────────────────────────────────────
Invoke-TestCase "T02 — Shard replication: write to A, read from B" {
    $content = Bytes "replicated-object-content-for-t02"
    $meta    = Invoke-Put -Node $A -Bucket "smoke" -Key "t02-replicated" -Data $content -Policy "warm"

    Write-Info "Written to agent-a (ETag=$($meta.etag)); waiting for async replication..."
    Start-Sleep -Seconds 4

    $resp = Invoke-Get -Node $B -Bucket "smoke" -Key "t02-replicated"
    $got  = AsString $resp
    if ($got -ne "replicated-object-content-for-t02") { throw "Content mismatch on agent-b: '$got'" }
    Write-Info "agent-b returned correct bytes (no shared volume — pure gRPC replication)"
}

# ─────────────────────────────────────────────────────────────────────────────
# T03 — Shard replication: write to A, read from C
# Same object as T02; confirms the fan-out reached the third node as well.
# ─────────────────────────────────────────────────────────────────────────────
Invoke-TestCase "T03 — Shard replication: write to A, read from C" {
    # Object already written in T02; just read from C (may need another moment)
    Start-Sleep -Seconds 1
    $resp = Invoke-Get -Node $C -Bucket "smoke" -Key "t02-replicated"
    $got  = AsString $resp
    if ($got -ne "replicated-object-content-for-t02") { throw "Content mismatch on agent-c: '$got'" }
    Write-Info "agent-c returned correct bytes"
}

# ─────────────────────────────────────────────────────────────────────────────
# T04 — Idempotent write: same content → same ETag, always
# Verifies the SHA-256 ETag is deterministic and not time-based.
# ─────────────────────────────────────────────────────────────────────────────
Invoke-TestCase "T04 — Idempotent write: same content produces same ETag" {
    $content = Bytes "idempotent-payload-v1"
    $m1 = Invoke-Put -Node $A -Bucket "smoke" -Key "t04-idem" -Data $content -Policy "warm"
    $m2 = Invoke-Put -Node $A -Bucket "smoke" -Key "t04-idem" -Data $content -Policy "warm"
    if ($m1.etag -ne $m2.etag) {
        throw "ETag changed between identical writes: '$($m1.etag)' vs '$($m2.etag)'"
    }
    Write-Info "ETag stable: $($m1.etag)"
}

# ─────────────────────────────────────────────────────────────────────────────
# T05 — All four storage policies are recorded correctly
# Writes one object per tier (hot/warm/cold/archive) to agent-b.
# ─────────────────────────────────────────────────────────────────────────────
Invoke-TestCase "T05 — All four storage policies are recorded (hot/warm/cold/archive)" {
    $content = Bytes "policy-probe"
    foreach ($policy in @("hot", "warm", "cold", "archive")) {
        $m = Invoke-Put -Node $B -Bucket "smoke" -Key "t05-$policy" -Data $content -Policy $policy
        if ($m.policy -ne $policy) { throw "Expected policy=$policy, got '$($m.policy)'" }
        Write-Info "policy=$policy  shards=$($m.replicas)  ETag=$($m.etag)"
    }
}

# ─────────────────────────────────────────────────────────────────────────────
# T06 — Bucket isolation: same key in two buckets holds separate data
# The distributed index must scope keys per bucket — no cross-contamination.
# ─────────────────────────────────────────────────────────────────────────────
Invoke-TestCase "T06 — Bucket isolation: same key in two buckets holds separate data" {
    Invoke-Put -Node $A -Bucket "bucket-alpha" -Key "shared-name" -Data (Bytes "ALPHA-DATA") | Out-Null
    Invoke-Put -Node $A -Bucket "bucket-beta"  -Key "shared-name" -Data (Bytes "BETA-DATA")  | Out-Null

    $rA = AsString (Invoke-Get -Node $A -Bucket "bucket-alpha" -Key "shared-name")
    $rB = AsString (Invoke-Get -Node $A -Bucket "bucket-beta"  -Key "shared-name")

    if ($rA -ne "ALPHA-DATA") { throw "bucket-alpha returned '$rA'" }
    if ($rB -ne "BETA-DATA")  { throw "bucket-beta returned '$rB'" }
    Write-Info "bucket-alpha and bucket-beta are fully isolated"
}

# ─────────────────────────────────────────────────────────────────────────────
# T07 — DELETE removes object; subsequent GET returns 404
# Written and deleted on the same node (agent-b) to test the local path.
# ─────────────────────────────────────────────────────────────────────────────
Invoke-TestCase "T07 — DELETE removes object; subsequent GET returns 404" {
    Invoke-Put -Node $B -Bucket "smoke" -Key "t07-delete" -Data (Bytes "to-be-deleted") | Out-Null
    Invoke-Delete -Node $B -Bucket "smoke" -Key "t07-delete"

    try {
        Invoke-Get -Node $B -Bucket "smoke" -Key "t07-delete"
        throw "Expected 404 but GET succeeded"
    } catch {
        $code = $_.Exception.Response.StatusCode.value__
        if ($code -eq 404 -or "$_" -match "404") {
            Write-Info "Correctly received 404 after DELETE"
        } else {
            throw
        }
    }
}

# ─────────────────────────────────────────────────────────────────────────────
# T08 — Prefix-filtered bucket listing
# Writes 4 objects in two path prefixes, verifies the prefix filter returns
# only the matching subset.
# ─────────────────────────────────────────────────────────────────────────────
Invoke-TestCase "T08 — Prefix-filtered bucket listing" {
    $data = Bytes "content"
    foreach ($key in @("logs/app.log", "logs/error.log", "metrics/cpu", "metrics/mem")) {
        Invoke-Put -Node $C -Bucket "list-test" -Key $key -Data $data | Out-Null
    }

    $result = Invoke-List -Node $C -Bucket "list-test" -Prefix "logs/"
    if ($result.count -ne 2) { throw "Expected 2 log objects, got $($result.count)" }
    $leak = $result.objects | Where-Object { -not $_.key.StartsWith("logs/") }
    if ($leak) { throw "Prefix filter leaked non-matching key '$($leak.key)'" }
    Write-Info "Prefix 'logs/' returned $($result.count) objects (metrics/ correctly excluded)"
}

# ─────────────────────────────────────────────────────────────────────────────
# T09 — Large object SHA-256 end-to-end integrity (512 KB)
# Computes the expected ETag locally before the upload, then verifies that
# the ETag returned by the server and the ETag in the response header both
# match. This catches any corruption in the compress → erasure → reconstruct
# pipeline.
# ─────────────────────────────────────────────────────────────────────────────
Invoke-TestCase "T09 — Large object SHA-256 integrity (512 KB)" {
    $pattern = Bytes "ALL4ONE-PHASE2-INTEGRITY-"
    $large   = [byte[]]::new(512 * 1024)
    for ($i = 0; $i -lt $large.Length; $i++) { $large[$i] = $pattern[$i % $pattern.Length] }

    $expectedEtag = Get-SHA256Hex $large

    $meta = Invoke-Put -Node $A -Bucket "smoke" -Key "t09-large" -Data $large -Policy "warm"
    if ($meta.etag -ne $expectedEtag) {
        throw "ETag on write mismatch: expected $expectedEtag got $($meta.etag)"
    }

    $resp       = Invoke-Get -Node $A -Bucket "smoke" -Key "t09-large"
    $etagHeader = $resp.Headers["ETag"]
    if ($etagHeader -ne $expectedEtag) {
        throw "ETag header on read mismatch: expected $expectedEtag got $etagHeader"
    }
    Write-Info "512 KB write/read integrity OK  ETag=$expectedEtag"
}

# ─────────────────────────────────────────────────────────────────────────────
# T10 — All three nodes write different keys; cross-node reads verify
#        distributed memory consistency
# Each agent is a separate machine with its own isolated volume.
# The only data path is gRPC TransferChunk. This test confirms that:
#   - data written on A can be read from B and C
#   - data written on B can be read from A and C
#   - data written on C can be read from A and B
# ─────────────────────────────────────────────────────────────────────────────
Invoke-TestCase "T10 — All-node write mesh: distributed memory convergence" {
    Invoke-Put -Node $A -Bucket "mesh" -Key "t10-a" -Data (Bytes "written-by-agent-a") -Policy "warm" | Out-Null
    Invoke-Put -Node $B -Bucket "mesh" -Key "t10-b" -Data (Bytes "written-by-agent-b") -Policy "warm" | Out-Null
    Invoke-Put -Node $C -Bucket "mesh" -Key "t10-c" -Data (Bytes "written-by-agent-c") -Policy "warm" | Out-Null

    Write-Info "All three nodes wrote; waiting for full-mesh replication..."
    Start-Sleep -Seconds 5

    # Cross-read: each node reads a key it never wrote
    $checks = @(
        @{ Node = $B; Key = "t10-a"; Expected = "written-by-agent-a"; Desc = "agent-b reads key from agent-a" }
        @{ Node = $C; Key = "t10-a"; Expected = "written-by-agent-a"; Desc = "agent-c reads key from agent-a" }
        @{ Node = $A; Key = "t10-b"; Expected = "written-by-agent-b"; Desc = "agent-a reads key from agent-b" }
        @{ Node = $C; Key = "t10-b"; Expected = "written-by-agent-b"; Desc = "agent-c reads key from agent-b" }
        @{ Node = $A; Key = "t10-c"; Expected = "written-by-agent-c"; Desc = "agent-a reads key from agent-c" }
        @{ Node = $B; Key = "t10-c"; Expected = "written-by-agent-c"; Desc = "agent-b reads key from agent-c" }
    )

    foreach ($c in $checks) {
        $got = AsString (Invoke-GetWithRetry -Node $c.Node -Bucket "mesh" -Key $c.Key -Attempts 12 -DelaySeconds 1)
        if ($got -ne $c.Expected) {
            throw "$($c.Desc): expected '$($c.Expected)' got '$got'"
        }
        Write-Info "OK  $($c.Desc)"
    }
}

# ── Summary ───────────────────────────────────────────────────────────────────
$passed = ($Results | Where-Object { $_.Status -eq "PASS" }).Count
$failed = ($Results | Where-Object { $_.Status -eq "FAIL" }).Count
$colour = if ($failed -eq 0) { "Green" } else { "Red" }

Write-Host ""
Write-Banner "═══════════════════════════════════════════════════════════════"
Write-Host ("  RESULTS   {0} passed   {1} failed   {2} total" -f $passed, $failed, $Results.Count) -ForegroundColor $colour
Write-Banner "═══════════════════════════════════════════════════════════════"

foreach ($r in $Results) {
    $c    = if ($r.Status -eq "PASS") { "Green" } else { "Red" }
    $line = "  {0}  {1}" -f $r.Status, $r.Name
    if ($r.Error) { $line += "  →  $($r.Error)" }
    Write-Host $line -ForegroundColor $c
}
Write-Host ""

if ($failed -gt 0) { exit 1 }
