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

## Steam Deck example: join a local 3-container cluster

Use this topology when your base machine runs the three Phase 2 Docker agents and you want to attach a Steam Deck on the same LAN as an extra Linux executor.

- Base machine: `agent-a`, `agent-b`, `agent-c` from `deploy/compose/docker-compose.phase2.yml`
- Storage stays on the three Docker agents, each with its own isolated named volume
- Steam Deck joins as a Tier 2, non-quorum node with its own local `data_dir`

### 1. Start the base cluster on the main machine

```bash
cd deploy/compose
docker compose -f docker-compose.phase2.yml up --build -d
docker compose -f docker-compose.phase2.yml ps
```

Record the LAN IP of the base machine as `<HOST_IP>`. The Steam Deck must be able to reach these mapped ports on that host:

- `<HOST_IP>:7947` for `agent-a`
- `<HOST_IP>:8947` for `agent-b`
- `<HOST_IP>:9947` for `agent-c`

### 2. Build the agent on the Steam Deck

SteamOS is Linux, but release binaries may not always match its glibc version. Building locally on the Steam Deck is the safest path.

```bash
curl https://sh.rustup.rs -sSf | sh -s -- -y
source "$HOME/.cargo/env"
git clone https://github.com/cacafuty/all4one.git
cd all4one
cargo build -p all4one-agent --release
mkdir -p "$HOME/bin"
cp target/release/all4one-agent "$HOME/bin/"
export PATH="$HOME/bin:$PATH"
```

### 3. Create a Steam Deck node config

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
seeds = ["<HOST_IP>:7947", "<HOST_IP>:8947", "<HOST_IP>:9947"]

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

### 4. Start the Steam Deck node with its advertised IP

The Docker nodes must be able to dial back to the Steam Deck. Set `ALL4ONE_ADVERTISE_HOST` so the agent publishes its real LAN address instead of `0.0.0.0` or a local hostname.

```bash
export PATH="$HOME/bin:$PATH"
export ALL4ONE_ADVERTISE_HOST="<STEAM_DECK_IP>"
all4one-agent start --config "$HOME/.config/all4one/steamdeck.toml"
```

If you prefer not to copy the binary into `$HOME/bin`, run `target/release/all4one-agent` from the repository checkout instead.

### 5. Verify from the Steam Deck

```bash
curl -s http://127.0.0.1:7946/health
curl -s -H "X-All4One-Secret: compose-secret" \
	"http://<HOST_IP>:7946/v1/nodes" | python3 -m json.tool
```

Expected result: the cluster view on the base machine shows `4` total nodes and the Steam Deck appears as a Tier 2 executor.

### 6. Open the operational UI from the Steam Deck

From the Steam Deck browser, open:

- `http://<HOST_IP>:7946/`
- `http://<HOST_IP>:8946/`
- `http://<HOST_IP>:9946/`

Use the same `compose-secret` in the UI secret field.

### 7. Common failure points

- If the Steam Deck never appears in `/v1/nodes`, re-check `ALL4ONE_ADVERTISE_HOST`.
- If enrollment loops forever, confirm the base machine ports `7947`, `8947`, and `9947` are reachable from the Steam Deck.
- If the Docker agents see the Steam Deck briefly and then mark it offline, confirm the Steam Deck firewall is not blocking TCP or UDP on `7947`.

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
