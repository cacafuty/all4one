# Configuración de un nodo

Guía paso a paso para instalar y configurar el agente en un dispositivo nuevo.

---

## Requisitos previos

| Plataforma      | Requisitos                                                           |
|-----------------|----------------------------------------------------------------------|
| Linux x86_64    | Kernel >= 5.4, systemd (opcional), Docker (opcional)                |
| Linux ARM64     | Kernel >= 5.4, Raspberry Pi OS o Ubuntu ARM64                       |
| macOS ARM64     | macOS 13+, macFUSE (opcional, para Fase 4)                          |
| macOS x86_64    | macOS 12+, macFUSE (opcional)                                       |
| Windows x86_64  | Windows 10 1903+, WinFsp (opcional, para Fase 4)                   |
| Android ARM64   | Android 10+, app instalada desde APK                                |

**Puertos que deben estar accesibles** en el firewall:
- `7946/TCP` — API REST (clientes externos)
- `7947/TCP` — gRPC (otros agentes)
- `7947/UDP` — SWIM gossip (otros agentes)

---

## Instalación

### Linux (x86_64 y ARM64)

```bash
# Descargar el binario
curl -sSL https://releases.all4one.io/latest/all4one-agent-linux-$(uname -m) \
  -o /usr/local/bin/all4one-agent
chmod +x /usr/local/bin/all4one-agent

# Verificar instalación
all4one-agent --version
# all4one-agent 0.1.0 (linux-x86_64)

# Crear directorio de datos
mkdir -p /var/lib/all4one/storage
```

### macOS

```bash
# Con Homebrew (cuando esté disponible)
brew install all4one/tap/all4one-agent

# O manualmente:
curl -sSL https://releases.all4one.io/latest/all4one-agent-darwin-arm64 \
  -o /usr/local/bin/all4one-agent
chmod +x /usr/local/bin/all4one-agent
```

### Windows

```powershell
# Con winget (cuando esté disponible)
winget install All4One.Agent

# O manualmente: descargar MSI de https://releases.all4one.io/latest/
```

---

## Configuración mínima por tipo de nodo

### Nodo Tier 0 — servidor 24/7 con storage y quórum

```toml
# /etc/all4one/agent.toml

[node]
tier = 0
availability = "always"
quorum_participant = true
data_dir = "/var/lib/all4one"

[roles]
scheduler = true
executor = true
storage = true

[network]
bind_address = "0.0.0.0"
grpc_port = 7947
rest_port = 7946
advertise_address = "192.168.1.100"   # IP fija del servidor

[discovery]
mdns = true
seeds = []   # este es el primer nodo, no hay seeds aún

[security]
mode = "dev"                          # cambiar a "prod" en Fase 2
shared_secret = "cambia-esto"

[executor]
max_concurrent_jobs = 16
docker_socket = "/var/run/docker.sock"
cgroups_enabled = true

[storage]
storage_path = "/var/lib/all4one/storage"
chunk_size_mb = 64
encryption = false

[capabilities]
docker = true
java = "/usr/bin/java"
python = "/usr/bin/python3"
wasm = true
gpu_enabled = false

[logging]
level = "info"
format = "json"
file = "/var/log/all4one/agent.log"
```

### Nodo Tier 1 — PC de oficina con horario laboral

```toml
[node]
tier = 1
availability = "cron:0 9-18 * * 1-5"   # lunes-viernes 9:00-18:00
quorum_participant = true
data_dir = "C:/ProgramData/all4one"     # Windows

[roles]
scheduler = false                        # solo ejecuta, no planifica
executor = true
storage = true

[network]
bind_address = "0.0.0.0"
grpc_port = 7947
rest_port = 7946

[discovery]
mdns = true
seeds = ["192.168.1.100:7947"]           # IP del Tier 0

[security]
mode = "dev"
shared_secret = "cambia-esto"

[executor]
max_concurrent_jobs = 4
docker_socket = "npipe:////./pipe/docker_engine"   # Windows Docker Desktop

[storage]
storage_path = "C:/ProgramData/all4one/storage"
chunk_size_mb = 64

[capabilities]
docker = true
java = "C:/Program Files/Java/jdk-21/bin/java.exe"
python = "C:/Python311/python.exe"
wasm = true

[logging]
level = "info"
format = "text"
```

### Nodo Tier 2 — Raspberry Pi solo executor

```toml
[node]
tier = 1          # Raspberry Pi puede ser Tier 1 si está siempre encendida
availability = "always"
quorum_participant = true
data_dir = "/var/lib/all4one"

[roles]
scheduler = false
executor = true
storage = true    # útil para cache de chunks en red local

[network]
bind_address = "0.0.0.0"
grpc_port = 7947
rest_port = 7946
advertise_address = "192.168.1.50"

[discovery]
mdns = true
seeds = ["192.168.1.100:7947"]

[security]
mode = "dev"
shared_secret = "cambia-esto"

[executor]
max_concurrent_jobs = 2   # Raspberry Pi tiene recursos limitados
cgroups_enabled = true

[storage]
storage_path = "/mnt/usb/all4one/storage"   # almacenamiento externo USB
chunk_size_mb = 64

[capabilities]
docker = false    # Raspberry Pi OS puede no tener Docker instalado
python = "/usr/bin/python3"
wasm = true

[logging]
level = "warn"    # reducir logging en dispositivos con SD card
format = "text"
```

---

## Primer arranque

```bash
# Arrancar en primer plano (para verificar que funciona)
all4one-agent start --config /etc/all4one/agent.toml

# Output esperado:
# 2026-04-08T10:00:00Z INFO all4one_agent: Starting All4One agent
# 2026-04-08T10:00:00Z INFO all4one::node: Node ID: f47ac10b-58cc-4372-a567-0e02b2c3d479
# 2026-04-08T10:00:00Z INFO all4one::config: Tier: 0, Roles: scheduler+executor+storage
# ⚠️  MODO DESARROLLO ACTIVO — no usar en producción
# 2026-04-08T10:00:00Z INFO all4one::discovery::mdns: Announcing _all4one._tcp.local
# 2026-04-08T10:00:00Z INFO all4one::api_rest: Listening on 0.0.0.0:7946
# 2026-04-08T10:00:00Z INFO all4one::grpc_server: Listening on 0.0.0.0:7947
```

---

## Instalación como servicio

### systemd (Linux)

```bash
cat > /etc/systemd/system/all4one-agent.service << 'EOF'
[Unit]
Description=All4One Agent
After=network.target docker.service
Wants=docker.service

[Service]
Type=simple
User=all4one
ExecStart=/usr/local/bin/all4one-agent start --config /etc/all4one/agent.toml
Restart=on-failure
RestartSec=5
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
EOF

useradd --system --no-create-home all4one
mkdir -p /var/lib/all4one /var/log/all4one
chown -R all4one:all4one /var/lib/all4one /var/log/all4one

systemctl daemon-reload
systemctl enable all4one-agent
systemctl start all4one-agent
systemctl status all4one-agent
```

### Windows Service

```powershell
# Registrar como servicio Windows
all4one-agent install-service --config "C:\ProgramData\all4one\agent.toml"
Start-Service All4OneAgent
```

### macOS launchd

```bash
cat > /Library/LaunchDaemons/io.all4one.agent.plist << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>io.all4one.agent</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/local/bin/all4one-agent</string>
    <string>start</string>
    <string>--config</string>
    <string>/etc/all4one/agent.toml</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
</dict>
</plist>
EOF

launchctl load /Library/LaunchDaemons/io.all4one.agent.plist
```

---

## Verificación del estado

```bash
# Salud del nodo local
curl http://localhost:7946/health
# { "status": "ok", "node_id": "f47ac10b-...", "uptime_seconds": 120, ... }

# Nodos del clúster
curl -H "X-All4One-Secret: cambia-esto" http://localhost:7946/v1/nodes

# Estado del clúster
curl -H "X-All4One-Secret: cambia-esto" http://localhost:7946/v1/cluster/status
```

---

## Enrolamiento en clúster Fase 2 (mTLS)

Cuando el clúster está en `mode = "prod"`, cada nodo nuevo debe enrolarse:

```bash
# En cualquier nodo ya miembro del clúster — generar token:
all4one-agent generate-token
# TOKEN=a3f8c2d1-b4e5-f6a7-b8c9-d0e1f2a3b4c5 (expira en 1h)

# En el nodo nuevo — enrolar:
all4one-agent enroll \
  --token a3f8c2d1-b4e5-f6a7-b8c9-d0e1f2a3b4c5 \
  --endpoint 192.168.1.100:7947

# Cambiar mode a "prod" en agent.toml:
# [security]
# mode = "prod"
# cert_dir = "/var/lib/all4one/certs"

# Reiniciar el agente:
systemctl restart all4one-agent
```

---

## Drenado controlado

Para apagar un nodo de forma ordenada sin perder datos:

```bash
# Anunciar drenado en 30 minutos (el clúster migra chunks en riesgo)
all4one-agent drain --in 30m

# Verificar que el drenado completó
curl -H "X-All4One-Secret: cambia-esto" \
  http://localhost:7946/v1/nodes/$(all4one-agent node-id)
# "status": "draining" → esperar a "offline"

# Apagar el agente
systemctl stop all4one-agent
```
