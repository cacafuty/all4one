# Node Setup Guide

Step-by-step setup for adding a node to an All4One cluster.

## 1. Install agent binary

- Download from releases for your platform
- Verify checksum
- Ensure binary is executable

## 2. Create minimal configuration

```toml
[node]
tier = 1
availability = "always"
quorum_participant = false
data_dir = "/var/lib/all4one"

[roles]
scheduler = false
executor = true
storage = false

[network]
bind_address = "0.0.0.0"
rest_port = 7946
grpc_port = 7947

[discovery]
mdns = true
seeds = ["192.168.1.100:7947"]

[security]
mode = "shared-secret"
shared_secret = "compose-secret"
```

## 3. Start agent

- Run with `all4one-agent start --config <path>`
- Verify `GET /health`
- Verify visibility in `GET /v1/nodes`

## Local dry-run: 2 agents on one machine (no Docker)

Use this as a preflight before multi-machine setup. It validates certificate enrollment and peer discovery with two native processes on different ports.

- Bootstrap node (agent A): `deploy/compose/configs/agent-win-local-bootstrap-peerseed.toml`
- Peer node (agent B): `deploy/compose/configs/agent-win-local-peer.toml`

### Ports

- Agent A: REST `7946`, gRPC `7947`
- Agent B: REST `8946`, gRPC `8947`

### Start agent A (bootstrap issuer)

```powershell
$env:ALL4ONE_ADVERTISE_HOST = "127.0.0.1"
$env:ALL4ONE_ADVERTISE_GRPC_PORT = "7947"
$env:ALL4ONE_ADVERTISE_REST_PORT = "7946"
.\target\release\all4one-agent.exe start --config .\deploy\compose\configs\agent-win-local-bootstrap-peerseed.toml
```

### Start agent B (peer with pre-shared CA cert)

```powershell
$env:ALL4ONE_ADVERTISE_HOST = "127.0.0.1"
$env:ALL4ONE_ADVERTISE_GRPC_PORT = "8947"
$env:ALL4ONE_ADVERTISE_REST_PORT = "8946"
.\target\release\all4one-agent.exe start --config .\deploy\compose\configs\agent-win-local-peer.toml
```

### Verify both nodes from both sides

```powershell
Invoke-RestMethod -Uri "http://127.0.0.1:7946/v1/internal/nodes" | ConvertTo-Json -Depth 5
Invoke-RestMethod -Uri "http://127.0.0.1:8946/v1/internal/nodes" | ConvertTo-Json -Depth 5
```

Expected result: both responses list 2 peers (agent A + agent B) as `online`.

## Steam Deck example: Host agent + Steam Deck

Use this topology for a simple two-node cluster: one agent on your host machine (Windows or Linux) and one on the Steam Deck. Both are executor nodes that discover each other via mTLS certificate enrollment.

| Node | Where | Ports (REST / gRPC) | Role | Tier |
|---|---|---|---|---|
| `agent-host` | Native on base machine (Windows/Linux) | `7946` / `7947` | executor, CA issuer | 0 |
| `agent-deck` | Steam Deck on LAN | `7946` / `7947` | executor | 1 |

- The host agent acts as the cluster bootstrap (CA issuer).
- The Steam Deck agent enrolls via pre-shared CA certificate from the host.
- Both nodes execute jobs and discover each other automatically.

### Network topology & reachability

**Important:** When the Steam Deck connects to the host agent, the host sees the Steam Deck's IP as the connection source. However, this does **not** guarantee the host can connect back to that same IP — the connection might be from a NAT, firewall, or VPN that only allows one-way traffic.

**Solution:** Each node explicitly announces its reachable address via the `ALL4ONE_ADVERTISE_HOST` environment variable. During enrollment, the Steam Deck includes its advertised address in the join request (`grpc_endpoint` and `rest_endpoint` fields). The host stores these and uses them whenever it needs to contact the Steam Deck.

| Scenario | Host sees IP | Host can connect back? | Solution |
|---|---|---|---|
| Both on same LAN | Steam Deck's private IP | ✅ Yes | Set `ALL4ONE_ADVERTISE_HOST` to LAN IP |
| Steam Deck behind home router | Steam Deck's private IP (on router) or public IP | ❌ Likely not | Set `ALL4ONE_ADVERTISE_HOST` to routable address |
| VPN or firewall between | Varies | ❌ Usually not | Use relay or bastion host |

**Always set `ALL4ONE_ADVERTISE_HOST`** on every node to its externally-reachable address. This ensures the cluster can form bidirectional connections.

### 1. Start the host agent as CA issuer

Use the pre-built config at `deploy/compose/configs/agent-win-local.toml` on Windows (or equivalent on Linux).

**Windows (PowerShell):**
```powershell
$env:ALL4ONE_ADVERTISE_HOST = "<HOST_IP>"
$env:ALL4ONE_ADVERTISE_GRPC_PORT = "7947"
$env:ALL4ONE_ADVERTISE_REST_PORT = "7946"
.\target\release\all4one-agent.exe start --config .\deploy\compose\configs\agent-win-local.toml
```

**Linux host:**
```bash
export ALL4ONE_ADVERTISE_HOST="<HOST_IP>"
./all4one-agent start --config deploy/compose/configs/agent-win-local.toml
```

Expected output:
```
INFO Node ID: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx
INFO Tier: 0 | Roles: scheduler=false executor=true storage=false
INFO CA initialized at "/path/to/certs", ca.key perms: 0600
INFO Certificate issuer ready (cluster bootstrap node)
INFO REST API listening on 0.0.0.0:7946
```

Record the `<HOST_IP>` (the LAN IP used in `ALL4ONE_ADVERTISE_HOST`) — the Steam Deck will need it.

### 2. Download the Linux x86-64 release on the Steam Deck

The GitHub release includes a pre-built `all4one-agent` for Linux x86-64 (SteamOS 3+).

```bash
VERSION="0.1.8"
mkdir -p "$HOME/bin"
curl -L "https://github.com/cacafuty/all4one/releases/download/v${VERSION}/all4one-agent-linux-x86_64.tar.gz" \
     -o /tmp/all4one-agent.tar.gz
tar -xzf /tmp/all4one-agent.tar.gz -C "$HOME/bin"
chmod +x "$HOME/bin/all4one-agent"
export PATH="$HOME/bin:$PATH"
all4one-agent --version
```

### 3. Copy the host's CA certificate to Steam Deck

The host has created a CA certificate at `C:/ProgramData/all4one-local/certs/ca.crt` (Windows) or `/var/lib/all4one/certs/ca.crt` (Linux). Copy this file to the Steam Deck so it can verify the host during enrollment.

**On the host machine:**
```powershell
# Windows: CA cert is at
cat C:/ProgramData/all4one-local/certs/ca.crt
```

**Transfer to Steam Deck (example via scp or manually):**
```bash
# On Steam Deck
mkdir -p "$HOME/.local/share/all4one-steamdeck/certs"
# Paste the host's ca.crt content into this file:
nano "$HOME/.local/share/all4one-steamdeck/certs/ca.crt"
```

Or use `scp` if SSH is available:
```bash
scp user@<HOST_IP>:/path/to/ca.crt "$HOME/.local/share/all4one-steamdeck/certs/ca.crt"
```

### 4. Create a Steam Deck node config

Create a config file at `$HOME/.config/all4one/steamdeck.toml`:

```toml
[node]
tier = 1
availability = "manual"
quorum_participant = false
data_dir = "/home/deck/.local/share/all4one-steamdeck"

[roles]
scheduler = false
executor = true
storage = false

[network]
bind_address = "0.0.0.0"
rest_port = 7946
grpc_port = 7947

[discovery]
mdns = false
# Seed points to the host agent's gRPC port
seeds = ["<HOST_IP>:7947"]

[security]
mode = "ca"
ca_cert_path = "/home/deck/.local/share/all4one-steamdeck/certs/ca.crt"

**Note on security modes:**
- `mode = "ca"`: Certificate Authority-based enrollment (production-like, recommended for multi-node clusters)
- `mode = "shared-secret"`: Shared secret authentication (dev/testing only)

[executor]
max_concurrent_jobs = 1
docker_socket = "/var/run/docker.sock"
cgroups_enabled = false

[capabilities]
docker = false
python = "/usr/bin/python3"
wasm = false

[logging]
level = "info"
format = "text"
EOF
```

Replace `<HOST_IP>` with your host's LAN IP (e.g., `192.168.1.100`).

**Critical:** Before starting, determine the Steam Deck's reachable IP address from the host's perspective. This is the value for `<STEAM_DECK_IP>` below. If you set `ALL4ONE_ADVERTISE_HOST` incorrectly:
- The host will enroll the Steam Deck successfully (one-way connection works)
- But the host won't be able to dispatch jobs to it (no bidirectional path)
- Check with `ping <STEAM_DECK_IP>` from the host to verify reachability

### 5. Start the Steam Deck agent

```bash
export PATH="$HOME/bin:$PATH"
export ALL4ONE_ADVERTISE_HOST="<STEAM_DECK_IP>"
all4one-agent start --config "$HOME/.config/all4one/steamdeck.toml"
```

Expected output:
```
INFO Node ID: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx
INFO Tier: 1 | Roles: scheduler=false executor=true storage=false
INFO Verified CA cert at: /home/deck/.local/share/all4one-steamdeck/certs/ca.crt
INFO gRPC Join request from node_id=... granted
INFO Node certificates saved, node.key perms: 0600
INFO Enrollment successful via seed=<HOST_IP>:7947 cert_expiry_unix=...
INFO REST API listening on 0.0.0.0:7946
```

### 6. Verify the 2-node cluster

From the host:
```powershell
curl http://127.0.0.1:7946/v1/internal/nodes | ConvertFrom-Json | ConvertTo-Json
```

Or from Steam Deck:
```bash
curl http://127.0.0.1:7946/v1/internal/nodes | python3 -m json.tool
```

Expected result: both endpoints list 2 peers (host + Steam Deck) as `online`.

### 7. Test job dispatch

Submit a job from the host to the Steam Deck:
```powershell
$job = @{
    runtime = "executable"
    source = "bash"
    command = @("-c", "echo 'Running on Steam Deck'")
    resources = @{ cpu_cores = 1; memory_mb = 128 }
    constraints = @{ tier_min = 1; requires_capabilities = @{ docker = $false } }
} | ConvertTo-Json

curl -X POST http://127.0.0.1:7946/v1/jobs `
    -ContentType "application/json" `
    -Body $job
```

The job should be assigned to the Steam Deck (tier=1) and complete successfully.

### 8. Troubleshooting

**Steam Deck never appears in `/v1/internal/nodes`:**
  1. Verify `ALL4ONE_ADVERTISE_HOST` is set on the Steam Deck to its reachable IP
  2. From the host, test: `ping <STEAM_DECK_IP>` and `nc -zv <STEAM_DECK_IP> 7947` (or `Test-NetConnection -ComputerName <STEAM_DECK_IP> -Port 7947` on Windows)
  3. Check host agent logs for "Node discovered" or enrollment errors
  4. If host sees the Steam Deck but can't reach its gRPC port, the advertised endpoint is unreachable

**Steam Deck appears in cluster but jobs won't dispatch to it:**
  - This indicates enrollment succeeded (one-way connection from Steam Deck to host) but the host can't connect back to the Steam Deck
  - Verify the Steam Deck's advertised address: `curl -s http://<STEAM_DECK_IP>:7946/v1/internal/nodes | grep grpc_endpoint`
  - From the host, try to connect to that endpoint: `nc -zv <ENDPOINT_IP> <ENDPOINT_PORT>`
  - If blocked, adjust `ALL4ONE_ADVERTISE_HOST` to a routable address or use a relay

**Enrollment fails (CA verification error):**
  - Confirm the CA certificate file exists: `ls -la /home/deck/.local/share/all4one-steamdeck/certs/ca.crt`
  - Verify it matches the host's: `diff /path/to/host/ca.crt /home/deck/.local/share/all4one-steamdeck/certs/ca.crt`
  - If different, re-copy the CA from the host to the Steam Deck

**Ports blocked:**
  - Ensure ports 7946 (REST) and 7947 (gRPC) are open on both machines
  - Use `netstat -an | grep 7946` (Linux/macOS) or `Get-NetTCPConnection -LocalPort 7946` (Windows) to verify listening
  - Check firewall: `sudo ufw status` (Linux) or Windows Defender Firewall rules

## Certificate-based enrollment (CA-only, no shared_secret)

This test validates mTLS cluster enrollment using only CA certificates, without relying on shared secrets. Useful for production-like setups where shared_secret should be disabled.

### Setup

**Prerequisites:**
- v0.1.8 or later binary
- Host (Windows or Linux) with LAN IP `<HOST_IP>`
- Steam Deck with LAN IP `<STEAM_DECK_IP>` on the same subnet
- Both nodes able to reach each other on ports 7946 and 7947

**Topology:**

| Node | Role | Config | Purpose |
|---|---|---|---|
| Host | Tier 0, CA issuer | `agent-win-local.toml` | Generates CA key and issues peer certificates |
| Steam Deck | Tier 1, executor | `agent-steam-deck.toml` | Enrolls via CA, joins cluster |

### 1. Start the host agent as CA issuer

Update `agent-win-local.toml` on the host:

```toml
[node]
tier = 0
data_dir = "C:/ProgramData/all4one-local"  # Windows

[discovery]
seeds = []  # Empty: tier=0 acts as bootstrap issuer

[security]
mode = "dev"
# No shared_secret: CA-only enrollment
```

Start on Windows:
```powershell
$env:ALL4ONE_ADVERTISE_HOST = "<HOST_IP>"
$env:ALL4ONE_ADVERTISE_GRPC_PORT = "7947"
$env:ALL4ONE_ADVERTISE_REST_PORT = "7946"
.\all4one-agent.exe start --config .\deploy\compose\configs\agent-win-local.toml
```

Expected output:
```
INFO CA initialized at "C:/ProgramData/all4one-local\certs", ca.key perms: 0600
INFO Certificate issuer ready (cluster bootstrap node)
INFO REST API listening on 0.0.0.0:7946
```

The host creates `ca.key` and `ca.crt` in the data_dir for signing peer certificates during enrollment.

### 2. Download v0.1.8 on Steam Deck

```bash
VERSION="0.1.8"
mkdir -p "$HOME/bin"
curl -L "https://github.com/cacafuty/all4one/releases/download/v${VERSION}/all4one-agent-linux-x86_64.tar.gz" \
     -o /tmp/all4one-agent.tar.gz
tar -xzf /tmp/all4one-agent.tar.gz -C "$HOME/bin"
chmod +x "$HOME/bin/all4one-agent"
export PATH="$HOME/bin:$PATH"
```

### 3. Configure Steam Deck for CA enrollment

Create `agent-steam-deck.toml`:

```toml
[node]
tier = 1
data_dir = "/home/deck/.local/share/all4one"

[roles]
scheduler = true
executor = true
storage = true

[network]
bind_address = "0.0.0.0"
grpc_port = 7947
rest_port = 7946

[discovery]
mdns = false
# Seeds point to host CA issuer
seeds = ["<HOST_IP>:7947"]

[security]
mode = "dev"
# CA-only: no shared_secret
```

Save this as `~/.config/all4one/agent-steam-deck.toml` on Steam Deck.

### 4. Start the Steam Deck agent

```bash
export ALL4ONE_ADVERTISE_HOST="<STEAM_DECK_IP>"
export PATH="$HOME/bin:$PATH"
all4one-agent start --config ~/.config/all4one/agent-steam-deck.toml
```

Expected enrollment flow:

1. Steam Deck agent starts (no existing credentials).
2. Queries seed `<HOST_IP>:7947` to discover the CA issuer.
3. Calls gRPC `Join` RPC on the host → passes Steam Deck node ID + public key.
4. Host agent signs the public key using its CA key → returns signed certificate + CA certificate.
5. Steam Deck stores `/home/deck/.local/share/all4one/certs/node.crt` and `node.key`.
6. Subsequent gRPC calls use mTLS (mutually authenticated).
7. Steam Deck discovers other peers via `/v1/internal/nodes` (minimal peer list, no sensitive data leakage).

Expected output on Steam Deck:
```
INFO Starting All4One agent v0.1.8
INFO Node ID: <STEAM_DECK_NODE_ID>
INFO Tier: 1 | Roles: scheduler=true executor=true storage=true
INFO Enrollment successful via seed=<HOST_IP>:7947 cert_expiry_unix=<TIMESTAMP>
INFO REST API listening on 0.0.0.0:7946
```

### 5. Verify cluster formation

From Steam Deck:
```bash
curl -s http://127.0.0.1:7946/health | python3 -m json.tool

# Check peer list (no shared_secret required for /v1/internal/nodes)
curl -s http://<HOST_IP>:7946/v1/internal/nodes | python3 -m json.tool
```

Expected output for `/v1/internal/nodes`:
```json
{
  "peers": [
    {
      "id": "<HOST_NODE_ID>",
      "grpc_endpoint": "<HOST_IP>:7947",
      "rest_endpoint": "<HOST_IP>:7946",
      "status": "alive"
    },
    {
      "id": "<STEAM_DECK_NODE_ID>",
      "grpc_endpoint": "<STEAM_DECK_IP>:7947",
      "rest_endpoint": "<STEAM_DECK_IP>:7946",
      "status": "alive"
    }
  ]
}
```

Note: This endpoint exposes minimal data (ID, gRPC/REST endpoints, status) **without** sensitive fields like capabilities, Docker sockets, or GPU info. Full node details remain in the authenticated `/v1/nodes` endpoint.

### 6. Verify mTLS connectivity

The Steam Deck and host agent now use mTLS for all gRPC communication. To inspect certificate chain:

From Steam Deck:
```bash
openssl x509 -in ~/.local/share/all4one/certs/node.crt -text -noout
```

The certificate will be signed by the CA issuer on the host and valid for the Steam Deck node ID.

### Troubleshooting CA test

| Issue | Cause | Fix |
|---|---|---|
| `Enrollment failed: seed unreachable` | Host not listening on `<HOST_IP>:7947` | Verify `ALL4ONE_ADVERTISE_HOST=<HOST_IP>` on host |
| `Enrollment failed: CA not initialized` | Host is not tier=0 or has seeds configured | Re-check `tier=0` and empty `seeds=[]` in host config |
| Steam Deck doesn't appear in peer list | Enrollment succeeded but discovery hasn't run yet | Wait 10 seconds (discovery polls every 5s) and retry |
| `Join RPC failed: unauthorized` | Host expects shared_secret but it's not in config | Ensure `[security]` block has no `shared_secret` line on both nodes |

#### Step A - start your local Steam Deck agent

```bash
export PATH="$HOME/bin:$PATH"
export ALL4ONE_ADVERTISE_HOST="<STEAM_DECK_IP>"
all4one-agent start --config "$HOME/.config/all4one/steamdeck.toml"
```

Expected outcome:

- Agent starts locally on Steam Deck (`:7946` / `:7947`).
- Within a few seconds the cluster should see 5 nodes total.

#### Step B - verify cluster membership from Steam Deck

```bash
curl -s http://127.0.0.1:7946/health | python3 -m json.tool

curl -s -H "X-All4One-Secret: compose-secret" \
    "http://<HOST_IP>:7946/v1/nodes" | python3 -m json.tool
```

Expected outcome:

- Steam Deck health reports `status: ok`.
- Node list shows 5 nodes: agent-a, agent-b, agent-c, host agent, Steam Deck.

#### Step C - validate Phase 3 visibility from Steam Deck browser

Open from Steam Deck:

- `http://<HOST_IP>:7946/`
- `http://<HOST_IP>:10946/`

In each UI page, set the same secret (`compose-secret`) in the secret field.

What to check in the UI:

- Node topology shows 5 nodes.
- Job lifecycle cards update when jobs run.
- Operational timeline receives live events (node/job transitions).
- Cluster pulse and distributed state are consistent across dashboards.

#### Step D - trigger activity from Steam Deck (optional)

```bash
cat > /tmp/steamdeck-probe.yaml << 'EOF'
runtime: python
source: "volatile://scripts/probe.py"
command: ["-c", "import time; print('steamdeck-probe'); time.sleep(2); print('done')"]
resources:
    cpu_cores: 1
    memory_mb: 128
EOF

curl -s -X POST "http://<HOST_IP>:7946/v1/jobs" \
    -H "Content-Type: application/yaml" \
    -H "X-All4One-Secret: compose-secret" \
    --data-binary @/tmp/steamdeck-probe.yaml | python3 -m json.tool
```

Then refresh the Phase 3 UI and verify recent jobs and timeline entries are updated.

## 4. Optional service installation

- Linux: systemd unit
- macOS: launchd
- Windows: service wrapper

## 5. Production enrollment (Phase 2+)

- Generate enrollment token on trusted node
- Join using enrollment API
- Persist issued cert/key and CA bundle

## 6. Safe drain and shutdown

- Announce drain before shutdown
- Wait for migration of critical work/data
