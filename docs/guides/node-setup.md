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
mode = "dev"
shared_secret = "compose-secret"
```

## 3. Start agent

- Run with `all4one-agent start --config <path>`
- Verify `GET /health`
- Verify visibility in `GET /v1/nodes`

## Steam Deck example: 3-container cluster + host agent + Steam Deck

Use this topology when your base machine runs the three Phase 2 Docker agents **and** a fourth native agent, and you want to attach a Steam Deck on the same LAN as a fifth executor node.

| Node | Where | Ports (REST / gRPC) | Role |
|---|---|---|---|
| `agent-a` | Docker on base machine | `7946` / `7947` | storage, scheduler, quorum |
| `agent-b` | Docker on base machine | `8946` / `8947` | storage, quorum |
| `agent-c` | Docker on base machine | `9946` / `9947` | storage, quorum |
| `agent-local` | Native process on base machine | `10946` / `10947` | executor (Windows/Linux host) |
| `agent-deck` | Steam Deck on LAN | `7946` / `7947` | executor |

- Storage stays on the three Docker agents, each with its own isolated named volume.
- The host agent and Steam Deck are Tier 2 executor-only nodes; they discover the cluster via a single seed.

### 1. Start the three Docker agents on the base machine

```bash
cd deploy/compose
docker compose -f docker-compose.phase2.yml up --build -d
docker compose -f docker-compose.phase2.yml ps
```

Record the LAN IP of the base machine as `<HOST_IP>`. All other nodes reach the Docker agents through this IP.

### 2. Start the host agent directly on the base machine

A pre-built config is included at `deploy/compose/configs/agent-win-local.toml` (works on Windows and Linux with minor path adjustments).

**Windows (PowerShell):**
```powershell
$env:ALL4ONE_ADVERTISE_HOST = "<HOST_IP>"
.\all4one-agent.exe start --config deploy\compose\configs\agent-win-local.toml
```

**Linux host:**
```bash
export ALL4ONE_ADVERTISE_HOST="<HOST_IP>"
./all4one-agent start --config deploy/compose/configs/agent-win-local.toml
```

The host agent uses ports `10946` (REST) and `10947` (gRPC) to avoid collisions with the Docker containers. It seeds from `127.0.0.1:7947` (agent-a), which is enough to discover the full cluster.

### 3. Download the Linux x86-64 release binary on the Steam Deck

The GitHub release includes a pre-built `all4one-agent` for Linux x86-64, which runs directly on SteamOS without needing Rust installed.

```bash
# Replace X.Y.Z with the latest version (e.g. 0.1.7)
VERSION="0.1.7"
mkdir -p "$HOME/bin"
curl -L "https://github.com/cacafuty/all4one/releases/download/v${VERSION}/all4one-agent-linux-x86_64.tar.gz" \
     -o /tmp/all4one-agent.tar.gz
tar -xzf /tmp/all4one-agent.tar.gz -C "$HOME/bin"
chmod +x "$HOME/bin/all4one-agent"
export PATH="$HOME/bin:$PATH"
# Verify
all4one-agent --version
```

### 4. Create a Steam Deck node config

Choose the Steam Deck LAN IP and keep it as `<STEAM_DECK_IP>`.

```bash
mkdir -p "$HOME/.config/all4one" "$HOME/.local/share/all4one-steamdeck"
cat > "$HOME/.config/all4one/steamdeck.toml" << 'EOF'
[node]
tier = 2
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
# Seed points to the host agent's gRPC port (10947 for agent-win-local).
seeds = ["<HOST_IP>:10947"]

[security]
mode = "dev"
shared_secret = "compose-secret"

[executor]
max_concurrent_jobs = 1
docker_socket = "/var/run/docker.sock"
cgroups_enabled = true

[capabilities]
docker = false
python = "/usr/bin/python3"
wasm = false

[logging]
level = "info"
format = "text"
EOF
```

Replace `<HOST_IP>` with the LAN IP of the base machine before starting the agent.

### 5. Start the Steam Deck node with its advertised IP

The other nodes must be able to dial back to the Steam Deck. Set `ALL4ONE_ADVERTISE_HOST` so the agent publishes its real LAN address instead of `0.0.0.0`.

```bash
export PATH="$HOME/bin:$PATH"
export ALL4ONE_ADVERTISE_HOST="<STEAM_DECK_IP>"
all4one-agent start --config "$HOME/.config/all4one/steamdeck.toml"
```

### 6. Verify the full 5-node cluster

From any node (including Steam Deck):

```bash
curl -s http://127.0.0.1:7946/health
curl -s -H "X-All4One-Secret: compose-secret" \
    "http://<HOST_IP>:7946/v1/nodes" | python3 -m json.tool
```

Expected result: `"total": 5` with agent-a/b/c, the host agent, and the Steam Deck all listed.

### 7. Open the operational UI

From any browser on the LAN:

- `http://<HOST_IP>:7946/` — agent-a dashboard
- `http://<HOST_IP>:8946/` — agent-b dashboard
- `http://<HOST_IP>:9946/` — agent-c dashboard
- `http://<HOST_IP>:10946/` — host agent dashboard

Use `compose-secret` in the secret field. Each dashboard shows the full 5-node topology.

### 8. Common failure points

- If the Steam Deck never appears in `/v1/nodes`, re-check `ALL4ONE_ADVERTISE_HOST`.
- If the host agent is missing, confirm `ALL4ONE_ADVERTISE_HOST` is set to the LAN IP (not `127.0.0.1`).
- If Docker agents can't reach the Steam Deck or host agent, confirm ports `7947` and `10947` are not blocked by the local firewall.

### 9. Steam Deck-only execution checklist (Phase 2 + Phase 3)

Use this section as a practical runbook for what to do from the Steam Deck after the base machine has the 3 Phase 2 Docker agents and the host agent running.

Before you start on Steam Deck:

- You know `<HOST_IP>` (base machine LAN IP).
- Port `7947` on `<HOST_IP>` is reachable from the Steam Deck (agent-a seed).
- You already replaced `<HOST_IP>` in your `steamdeck.toml` seeds.

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
