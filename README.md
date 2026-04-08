# All4One

Plataforma de infraestructura distribuida P2P que convierte hardware heterogéneo
existente (portátiles, PCs de oficina, servidores, Raspberry Pi, móviles Android)
en un clúster unificado de cómputo y almacenamiento. Los procesos que corren sobre
él no saben que están en un entorno distribuido.

---

## Principio central

No existe un nodo obligatorio de orquestación. Cualquier nodo puede recibir jobs
y coordinar. El clúster funciona con lo que haya disponible en cada momento.

```
┌─────────────┐   ┌─────────────┐   ┌─────────────┐
│  Portátil   │   │ Raspberry   │   │  PC Oficina │
│  (Tier 1)   │◄──│   Pi Tier 0 │──►│  (Tier 1)   │
│  scheduler  │   │  scheduler  │   │  executor   │
│  executor   │   │  executor   │   │  storage    │
└─────────────┘   │  storage    │   └─────────────┘
                  │  raft/quórum│
                  └─────────────┘
         Cualquier nodo acepta jobs y coordina
```

---

## Modelo de negocio

Software on-premise con suscripción anual por número de nodos activos:

| Plan       | Nodos       |
|------------|-------------|
| Starter    | hasta 10    |
| Business   | hasta 50    |
| Enterprise | ilimitados  |

A futuro: cloud low-cost operado por All4One para empresas sin infraestructura
propia, con el mismo agente y modelo multi-tenant.

---

## Arquitectura en una página

```
                        AGENTE (único binario Rust)
                 ┌──────────────────────────────────────┐
                 │  config  │  node  │  discovery        │
                 │──────────────────────────────────────│
                 │  gossip (SWIM UDP:7947)               │
                 │──────────────────────────────────────│
                 │  raft (openraft, Fase 2+)             │
                 │──────────────────────────────────────│
                 │  scheduler  │  executor  │  storage   │
                 │──────────────────────────────────────│
                 │  api_rest (:7946) │ grpc (:7947)      │
                 │──────────────────────────────────────│
                 │  certificates (Fase 2+)               │
                 │──────────────────────────────────────│
                 │  lifecycle (Fase 3+, líder Raft only) │
                 └──────────────────────────────────────┘
```

El agente ejerce hasta tres roles simultáneamente según configuración:

- **SCHEDULER** — recibe jobs y decide placement
- **EXECUTOR** — ejecuta jobs
- **STORAGE** — almacena chunks

---

## Tiers de nodos

| Tier | Descripción                                     | Quórum |
|------|-------------------------------------------------|--------|
| 0    | 24/7. Servidores, NAS, Raspberry Pi dedicado.   | Sí     |
| 1    | Disponibilidad predecible con horario conocido. | Sí     |
| 2    | Oportunista. Portátiles personales, móviles.    | No     |

La metadata crítica y al menos una réplica de cada dato residen siempre en Tier 0.

---

## Inicio rápido (Fase 1)

### Primer nodo

```bash
# Instalación
curl -sSL https://releases.all4one.io/install.sh | bash

# Configuración mínima
cat > /etc/all4one/agent.toml << 'EOF'
[node]
tier = 0
availability = "always"
quorum_participant = true
data_dir = "/var/lib/all4one"

[roles]
scheduler = true
executor = true
storage = false

[network]
bind_address = "0.0.0.0"
grpc_port = 7947
rest_port = 7946

[security]
mode = "dev"
shared_secret = "change-me-before-production"

[discovery]
mdns = true
seeds = []
EOF

# Arranque
all4one-agent start
```

### Segundo nodo (mismo flag `seeds`)

```bash
cat > /etc/all4one/agent.toml << 'EOF'
[node]
tier = 1
availability = "cron:0 9-18 * * 1-5"
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

[security]
mode = "dev"
shared_secret = "change-me-before-production"

[discovery]
mdns = true
seeds = ["192.168.1.100:7947"]
EOF

all4one-agent start
```

### Primer job

```bash
cat > hello.yaml << 'EOF'
runtime: docker
source: "python:3.11-slim"
command: ["python", "-c", "print('Hello from All4One!')"]
resources:
  cpu_cores: 1
  memory_mb: 256
EOF

curl -X POST http://192.168.1.100:7946/v1/jobs \
  -H "Content-Type: application/yaml" \
  -H "X-All4One-Secret: change-me-before-production" \
  --data-binary @hello.yaml
```

---

## Fases de implementación

| Fase | Nombre                          | Descripción breve                           |
|------|---------------------------------|---------------------------------------------|
| 1    | Ejecuta algo en algún sitio     | Gossip + scheduler + executor. Sin storage. |
| 2    | Los datos viven en el clúster   | Raft + storage + mTLS + PKI interna.        |
| 3    | Los datos se gestionan solos    | Lifecycle engine + tiering automático.      |
| 4    | Transparencia total ante procesos | FUSE + LD_PRELOAD + SDK + S3 API.         |
| 5    | Plataformas y madurez           | Android + GPU + multi-tenant + UI web.      |

---

## Documentación

### Arquitectura
- [Visión general](docs/architecture/overview.md)
- [Módulos del agente](docs/architecture/agent.md)
- [Protocolos de red](docs/architecture/networking.md)
- [Scheduler y placement](docs/architecture/scheduler.md)
- [Almacenamiento distribuido](docs/architecture/storage.md)
- [Lifecycle engine](docs/architecture/lifecycle.md)
- [IA e inferencia](docs/architecture/ai-inference.md)

### API
- [Especificación Job](docs/api/job-spec.md)
- [API REST completa](docs/api/rest-api.md)
- [Referencia agent.toml](docs/api/config-reference.md)

### Fases
- [Fase 1](docs/phases/phase-1.md) · [Fase 2](docs/phases/phase-2.md) · [Fase 3](docs/phases/phase-3.md) · [Fase 4](docs/phases/phase-4.md) · [Fase 5](docs/phases/phase-5.md)

### Guías
- [Configuración de un nodo](docs/guides/node-setup.md)
- [Primeros pasos](docs/guides/getting-started.md)

### Decisiones de arquitectura
- [ADR-001 Rust para el agente](docs/decisions/001-rust-agent.md)
- [ADR-002 Sin nodo central](docs/decisions/002-no-central-node.md)
- [ADR-003 Sin MinIO](docs/decisions/003-no-minio.md)
- [ADR-004 llama.cpp RPC](docs/decisions/004-llama-cpp-rpc.md)
- [ADR-005 gRPC interno](docs/decisions/005-grpc-internal.md)
- [ADR-006 PKI + mTLS](docs/decisions/006-pki-mtls.md)

---

## Stack tecnológico (resumen de licencias)

| Crate / librería | Licencia     | Uso                         |
|------------------|--------------|-----------------------------|
| tokio            | MIT          | async runtime               |
| axum             | MIT          | API REST                    |
| tonic            | MIT          | gRPC server y client        |
| prost            | Apache 2.0   | Protocol Buffers            |
| openraft         | Apache 2.0   | consenso Raft embebido      |
| chitchat         | Apache 2.0   | SWIM gossip                 |
| mdns-sd          | MIT          | descubrimiento mDNS         |
| reed-solomon     | Apache 2.0   | erasure coding              |
| zstd             | MIT          | compresión                  |
| rcgen            | MIT          | certificados X.509          |
| rustls           | Apache 2.0   | TLS                         |
| sled             | MIT          | índice de chunks            |
| fuser            | MIT          | FUSE Linux/macOS            |
| WinFsp           | LGPL         | FUSE Windows (enlace dinámico) |
| wasmtime         | Apache 2.0   | runtime WASM                |
| llama.cpp        | MIT          | inferencia IA y RPC         |
| bincode          | MIT          | serialización SWIM          |

> **MinIO descartado**: AGPL v3 incompatible con redistribución propietaria.
> Ver [ADR-003](docs/decisions/003-no-minio.md).

---

## Plataformas soportadas

| Plataforma      | Executor | Storage | Quórum | FUSE |
|-----------------|----------|---------|--------|------|
| Linux x86_64    | ✓        | ✓       | ✓      | Nativo |
| Linux ARM64     | ✓        | ✓       | ✓      | Nativo |
| macOS ARM64     | ✓        | ✓       | ✓      | macFUSE |
| macOS x86_64    | ✓        | ✓       | ✓      | macFUSE |
| Windows x86_64  | ✓        | ✓       | ✓      | WinFsp |
| Android ARM64   | —        | ✓       | —      | — |

iOS: descartado en v1 por sandbox de Apple.
