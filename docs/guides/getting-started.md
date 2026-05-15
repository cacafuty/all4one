# Getting Started

This guide takes you from zero to a functional cluster and your first successful job.
It covers Phase 1 (no distributed storage, no Raft, no mTLS).

**All4One works from a single node.** Start on one machine, then scale from 1 to N
nodes when you need more capacity and resilience.

**Estimated time**: 10-20 minutes.

**Requirements**:
- One machine (Linux, macOS, or WSL2)
- Docker installed if you want to run Docker jobs
- Optional: additional machines on the same network for multi-node scaling

---

## Option: local lab with Docker Compose

If you want to validate a multi-node Phase 1 startup on a single machine, use the
local lab under `deploy/compose/`:

```bash
cd deploy/compose
docker compose up --build -d
docker compose ps
docker compose logs -f agent-a
```

This lab is for fast iteration, not a replacement for real hardware validation.
It helps you:

- Verify the Rust agent binary starts correctly in multiple nodes.
- Test per-node `agent.toml` variants quickly.
- Iterate before network-level testing.

If you want the same scenario using GitHub Release binaries instead of local build:

```bash
cd deploy/compose
VERSION=0.1.5 GH_REPO=cacafuty/all4one docker compose -f docker-compose.release.yml up --build -d
docker compose -f docker-compose.release.yml ps
docker compose -f docker-compose.release.yml logs -f agent-a
```

That flow downloads `all4one-agent` during image build and starts the same three
nodes (`agent-a`, `agent-b`, `agent-c`) with configs from `deploy/compose/configs/`.

---

## Step 1: install the agent

### Recommended option: autoinstall (download latest release and start)

The script `scripts/autoinstall.sh` downloads the latest release from GitHub,
verifies checksums, creates a minimal config (if missing), and starts the agent.
If the latest public release is a prerelease, `latest` will pick it.

```bash
curl -fsSL https://raw.githubusercontent.com/cacafuty/all4one/main/scripts/autoinstall.sh -o autoinstall
chmod +x autoinstall
./autoinstall
```

Common options:

```bash
./autoinstall --version 0.1.5 --shared-secret my-secret
./autoinstall --seeds 192.168.1.100:7947,192.168.1.101:7947
./autoinstall --no-start
```

If the repository or release requires authentication, use a GitHub token:

```bash
GITHUB_TOKEN=ghp_xxx ./autoinstall --version 0.1.5
# equivalent
./autoinstall --version 0.1.5 --github-token ghp_xxx
```

Prepared for future certificate-based join:

```bash
./autoinstall --join-cert /etc/all4one/node.crt --join-endpoint 192.168.1.100:7947 --no-start
```

In Phase 1, these join parameters are stored as reference in `agent.toml`.

Prebuilt binaries are available on
[GitHub Releases](https://github.com/cacafuty/all4one/releases), including a
`checksums.sha256` file.

```bash
# Replace X.Y.Z with the version (for example: 0.2.0)
VERSION=X.Y.Z
GH_REPO=cacafuty/all4one
BASE=https://github.com/${GH_REPO}/releases/download/v${VERSION}

# Linux x86_64
curl -sSL ${BASE}/all4one-agent-linux-x86_64.tar.gz | tar xz -C /usr/local/bin
chmod +x /usr/local/bin/all4one-agent

# Linux ARM64 (Raspberry Pi)
curl -sSL ${BASE}/all4one-agent-linux-arm64.tar.gz | tar xz -C /usr/local/bin
chmod +x /usr/local/bin/all4one-agent

# macOS ARM64
curl -sSL ${BASE}/all4one-agent-macos-arm64.tar.gz | tar xz -C /usr/local/bin
chmod +x /usr/local/bin/all4one-agent

# Windows x86_64 (PowerShell)
# Download all4one-agent-windows-x86_64.zip from releases
# and extract all4one-agent.exe to your target directory.
```

Verify integrity before running:

```bash
curl -sSL ${BASE}/checksums.sha256 | sha256sum --check --ignore-missing
```

---

## Step 2: configure your node

```bash
mkdir -p /etc/all4one /var/lib/all4one

cat > /etc/all4one/agent.toml << 'EOF'
[node]
tier = 0
availability = "always"
quorum_participant = true
data_dir = "/var/lib/all4one"

[roles]
scheduler = true
executor = true
storage = false   # Storage is enabled in Phase 2

[network]
bind_address = "0.0.0.0"
grpc_port = 7947
rest_port = 7946

[discovery]
mdns = true
seeds = []

[security]
mode = "dev"
shared_secret = "my-test-secret"

[executor]
max_concurrent_jobs = 8
docker_socket = "/var/run/docker.sock"
cgroups_enabled = true
# Optional: Cap this agent's available memory for scheduling
# If not set, the system's detected available memory is used
# If set, the scheduler sees: min(system_available, memory_limit_mb)
# memory_limit_mb = 2048

[capabilities]
docker = true
python = "/usr/bin/python3"
wasm = true

[logging]
level = "info"
format = "text"
EOF

all4one-agent start --config /etc/all4one/agent.toml
```

Expected output:

```
INFO Starting All4One agent v0.1.0
INFO Node ID: f47ac10b-58cc-4372-a567-0e02b2c3d479
INFO Tier: 0 | Roles: scheduler+executor
WARN DEVELOPMENT MODE ACTIVE - do not use in production
INFO mDNS: announcing _all4one._tcp.local
INFO REST API listening on 0.0.0.0:7946
INFO gRPC listening on 0.0.0.0:7947
INFO SWIM gossip listening on UDP 0.0.0.0:7947
```

With a single node, you can send jobs directly to `localhost:7946`.
If you plan to add nodes, note this machine IP (`ip addr show` or `ifconfig`).

---

## Step 3: verify the node is healthy

```bash
curl -s http://localhost:7946/health | python3 -m json.tool
```

```json
{
  "status": "ok",
  "node_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
  "uptime_seconds": 12,
  "cluster_connected": true,
  "quorum_healthy": false
}
```

`quorum_healthy: false` is expected in Phase 1 without Raft.

If you are running one node only, jump directly to **Step 6**.

---

## Step 4: add a second node (optional)

This step is optional. All4One is fully functional with one node.
Use this step to distribute load and increase resilience.

On the second machine, replace `192.168.1.100` with your first node IP:

```bash
mkdir -p /etc/all4one /var/lib/all4one

cat > /etc/all4one/agent.toml << 'EOF'
[node]
tier = 1
availability = "cron:0 0-23 * * *"
quorum_participant = true
data_dir = "/var/lib/all4one"

[roles]
scheduler = false
executor = true
storage = false

[network]
bind_address = "0.0.0.0"
grpc_port = 7947
rest_port = 7946

[discovery]
mdns = true
seeds = ["192.168.1.100:7947"]

[security]
mode = "dev"
shared_secret = "my-test-secret"

[executor]
max_concurrent_jobs = 4
docker_socket = "/var/run/docker.sock"
cgroups_enabled = true
# Optional: Limit tier 1 node to 1024 MB to reserve system resources
# memory_limit_mb = 1024

[capabilities]
docker = true
python = "/usr/bin/python3"
wasm = false

[logging]
level = "info"
format = "text"
EOF

all4one-agent start --config /etc/all4one/agent.toml
```

---

## Step 5: verify node discovery

Only applies when running multiple nodes.

On the first node, logs should include:

```
INFO gossip: NodeJoined b2c3d4e5-f6a7-8901-bcde-f01234567890 (192.168.1.101, Tier 1)
```

Check through REST:

```bash
curl -s -H "X-All4One-Secret: my-test-secret" \
  http://localhost:7946/v1/nodes | python3 -m json.tool
```

---

## Step 6: submit your first job

From any machine that can reach the node:

```bash
curl -s -X POST http://192.168.1.100:7946/v1/jobs \
  -H "Content-Type: application/yaml" \
  -H "X-All4One-Secret: my-test-secret" \
  -d '
runtime: python
source: "volatile://scripts/hello.py"
command: ["-c", "import time; [print(f\"Line {i}\", flush=True) or time.sleep(0.5) for i in range(10)]"]
resources:
  cpu_cores: 1
  memory_mb: 128
'
```

---

## Step 7: stream job output

```bash
curl -s -N \
  -H "Accept: text/event-stream" \
  -H "X-All4One-Secret: my-test-secret" \
  http://192.168.1.100:7946/v1/jobs/c3d4e5f6-a7b8-9012-cdef-012345678901/output/stream
```

---

## Step 8: Docker job with capability constraints

```bash
cat > /tmp/docker-job.yaml << 'EOF'
runtime: docker
source: "alpine:3.19"
command: ["sh", "-c", "echo 'Running in Docker!' && uname -a && cat /proc/cpuinfo | grep 'model name' | head -1"]
resources:
  cpu_cores: 1
  memory_mb: 256
constraints:
  requires_capabilities:
    docker: true
priority: high
EOF

curl -s -X POST http://192.168.1.100:7946/v1/jobs \
  -H "Content-Type: application/yaml" \
  -H "X-All4One-Secret: my-test-secret" \
  --data-binary @/tmp/docker-job.yaml
```

---

## Next steps

- Add more nodes via [Node setup](node-setup.md).
- Enable distributed storage in [Phase 2](../phases/phase-2.md).
- Full job schema: [Job Spec](../api/job-spec.md).
- Full API details: [REST API](../api/rest-api.md).
- Configuration fields: [agent.toml reference](../api/config-reference.md).

---

## Troubleshooting

**Nodes are not discovered via mDNS**:
- Ensure firewall allows UDP multicast.
- Some corporate networks block mDNS; use `seeds`.
- Check that ports 7946/7947 are free.

**Jobs remain in `queued`**:
- Scheduler found no nodes matching constraints.
- Check `GET /v1/nodes` for online nodes and capabilities.
- Retry with a job without constraints.

**`401 Unauthorized` on all calls**:
- `shared_secret` does not match `agent.toml`.
- Header must be `X-All4One-Secret`.

**Docker socket permission denied**:
- The user running the agent must be in the `docker` group.
