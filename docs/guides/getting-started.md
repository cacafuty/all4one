# Primeros pasos

Esta guía lleva desde cero hasta un clúster funcional de dos nodos con el primer
job ejecutándose. Cubre Fase 1 (sin storage, sin Raft, sin mTLS).

**Tiempo estimado**: 15–20 minutos.

**Qué necesitas**:
- Dos máquinas en la misma red local (o una sola para empezar)
- Linux, macOS o WSL2 en ambas
- Docker instalado en al menos una de ellas

---

## Paso 1: instalar el agente

En ambas máquinas:

```bash
# Linux x86_64
curl -sSL https://releases.all4one.io/latest/all4one-agent-linux-x86_64 \
  -o /usr/local/bin/all4one-agent && chmod +x /usr/local/bin/all4one-agent

# Linux ARM64 (Raspberry Pi)
curl -sSL https://releases.all4one.io/latest/all4one-agent-linux-aarch64 \
  -o /usr/local/bin/all4one-agent && chmod +x /usr/local/bin/all4one-agent

# macOS ARM64
curl -sSL https://releases.all4one.io/latest/all4one-agent-darwin-arm64 \
  -o /usr/local/bin/all4one-agent && chmod +x /usr/local/bin/all4one-agent
```

---

## Paso 2: configurar el primer nodo (Nodo A)

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
storage = false   # storage se activa en Fase 2

[network]
bind_address = "0.0.0.0"
grpc_port = 7947
rest_port = 7946

[discovery]
mdns = true
seeds = []

[security]
mode = "dev"
shared_secret = "mi-secreto-de-prueba"

[executor]
max_concurrent_jobs = 8
docker_socket = "/var/run/docker.sock"
cgroups_enabled = true

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

Deberías ver:
```
INFO Starting All4One agent v0.1.0
INFO Node ID: f47ac10b-58cc-4372-a567-0e02b2c3d479
INFO Tier: 0 | Roles: scheduler+executor
⚠️  MODO DESARROLLO ACTIVO — no usar en producción
INFO mDNS: announcing _all4one._tcp.local
INFO REST API listening on 0.0.0.0:7946
INFO gRPC listening on 0.0.0.0:7947
INFO SWIM gossip listening on UDP 0.0.0.0:7947
```

Anota la **IP del Nodo A** (`ip addr show` o `ifconfig`). La usarás en el Nodo B.

---

## Paso 3: verificar que el Nodo A responde

```bash
# En otra terminal, en el Nodo A
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

`quorum_healthy: false` es normal en Fase 1 sin Raft.

---

## Paso 4: configurar el segundo nodo (Nodo B)

En el Nodo B, reemplaza `192.168.1.100` con la IP real del Nodo A:

```bash
mkdir -p /etc/all4one /var/lib/all4one

cat > /etc/all4one/agent.toml << 'EOF'
[node]
tier = 1
availability = "cron:0 0-23 * * *"   # siempre disponible para esta prueba
quorum_participant = true
data_dir = "/var/lib/all4one"

[roles]
scheduler = false   # solo ejecuta
executor = true
storage = false

[network]
bind_address = "0.0.0.0"
grpc_port = 7947
rest_port = 7946

[discovery]
mdns = true
seeds = ["192.168.1.100:7947"]   # ← IP del Nodo A

[security]
mode = "dev"
shared_secret = "mi-secreto-de-prueba"

[executor]
max_concurrent_jobs = 4
docker_socket = "/var/run/docker.sock"
cgroups_enabled = true

[capabilities]
docker = true
python = "/usr/bin/python3"
wasm = false   # si no hay wasmtime disponible

[logging]
level = "info"
format = "text"
EOF

all4one-agent start --config /etc/all4one/agent.toml
```

---

## Paso 5: verificar que los nodos se descubren

En el Nodo A, el log debería mostrar:
```
INFO gossip: NodeJoined b2c3d4e5-f6a7-8901-bcde-f01234567890 (192.168.1.101, Tier 1)
```

Confirmar via REST:
```bash
curl -s -H "X-All4One-Secret: mi-secreto-de-prueba" \
  http://localhost:7946/v1/nodes | python3 -m json.tool
```

```json
{
  "nodes": [
    { "profile": { "id": "f47ac10b-...", "tier": 0 }, "status": "online", ... },
    { "profile": { "id": "b2c3d4e5-...", "tier": 1 }, "status": "online", ... }
  ],
  "total": 2,
  "online": 2,
  "offline": 0
}
```

---

## Paso 6: enviar el primer job

Desde cualquier máquina que tenga acceso a la red (puede ser tu portátil personal):

```bash
# Job simple: Python en cualquier nodo
curl -s -X POST http://192.168.1.100:7946/v1/jobs \
  -H "Content-Type: application/yaml" \
  -H "X-All4One-Secret: mi-secreto-de-prueba" \
  -d '
runtime: python
source: "volatile://scripts/hello.py"
command: ["-c", "import time; [print(f\"Línea {i}\", flush=True) or time.sleep(0.5) for i in range(10)]"]
resources:
  cpu_cores: 1
  memory_mb: 128
'
```

```json
{
  "job_id": "c3d4e5f6-a7b8-9012-cdef-012345678901",
  "status": "scheduled",
  "assigned_to": "b2c3d4e5-f6a7-8901-bcde-f01234567890",
  "created_at": "2026-04-08T10:30:00Z",
  ...
}
```

---

## Paso 7: ver el output en tiempo real

```bash
# Streaming SSE del output del job
curl -s -N \
  -H "Accept: text/event-stream" \
  -H "X-All4One-Secret: mi-secreto-de-prueba" \
  http://192.168.1.100:7946/v1/jobs/c3d4e5f6-a7b8-9012-cdef-012345678901/output/stream
```

```
event: stdout
data: {"line": "Línea 0"}

event: stdout
data: {"line": "Línea 1"}

event: stdout
data: {"line": "Línea 2"}
...

event: completed
data: {"exit_code": 0}
```

---

## Paso 8: job Docker con constraints de capabilities

Este job solo puede correr en nodos con Docker instalado:

```bash
cat > /tmp/docker-job.yaml << 'EOF'
runtime: docker
source: "alpine:3.19"
command: ["sh", "-c", "echo 'Corriendo en Docker!' && uname -a && cat /proc/cpuinfo | grep 'model name' | head -1"]
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
  -H "X-All4One-Secret: mi-secreto-de-prueba" \
  --data-binary @/tmp/docker-job.yaml
```

---

## Siguientes pasos

- Añadir más nodos siguiendo [Configuración de un nodo](node-setup.md).
- Activar storage distribuido: ver [Fase 2](../phases/phase-2.md).
- Especificación completa del Job: ver [Job Spec](../api/job-spec.md).
- API REST completa: ver [REST API](../api/rest-api.md).
- Todos los campos de configuración: ver [Referencia agent.toml](../api/config-reference.md).

---

## Diagnóstico de problemas comunes

**Los nodos no se descubren via mDNS**:
- Verifica que el firewall permite tráfico UDP multicast.
- En algunas redes corporativas, mDNS está bloqueado. Usa `seeds` como alternativa.
- Comprueba que los puertos 7946 y 7947 no están en uso: `ss -tlnp | grep 794`.

**El job queda en `queued` para siempre**:
- El scheduler no encuentra nodos que cumplan las constraints.
- Verifica con `GET /v1/nodes` que hay nodos online con las capabilities requeridas.
- Prueba con un job sin constraints para confirmar que el scheduler funciona.

**`401 Unauthorized` en todas las llamadas**:
- El `shared_secret` en el header no coincide con el de `agent.toml`.
- Verifica que el header es `X-All4One-Secret` (no `X-All4One-Secret-Key` ni similar).

**Docker: `permission denied` al acceder al socket**:
- El usuario que corre el agente debe estar en el grupo `docker`.
- En Linux: `usermod -aG docker all4one && newgrp docker`.
