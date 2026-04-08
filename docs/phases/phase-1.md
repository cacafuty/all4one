# Fase 1 — "Ejecuta algo en algún sitio"

**Objetivo**: clúster mínimo funcional que descubre nodos en la red local,
acepta jobs via REST y los ejecuta en el nodo correcto según capabilities.
El desarrollador puede observar stdout en tiempo real.

**Plataformas objetivo**: Linux x86_64, Linux ARM64, macOS ARM64, **Windows x86_64**.

---

## Módulos activos

| Módulo         | Estado   | Notas                                              |
|----------------|----------|----------------------------------------------------|
| `config`       | Completo |                                                    |
| `node`         | Completo |                                                    |
| `discovery`    | Completo | mDNS + seeds                                       |
| `gossip`       | Completo | SWIM UDP, ClusterState, MembershipEvent            |
| `scheduler`    | Completo | Sin Raft — best-effort, race condition aceptada    |
| `executor`     | Completo | docker, jar, python, executable, wasm              |
| `api_rest`     | Parcial  | Solo endpoints de jobs y nodos. Sin auth real.     |
| `grpc_server`  | Parcial  | AgentService (sin RaftService). shared_secret.     |
| `grpc_client`  | Parcial  | Sin TLS.                                           |

## Módulos ausentes en Fase 1

| Módulo         | Fase que lo añade |
|----------------|-------------------|
| `raft`         | Fase 2            |
| `storage`      | Fase 2            |
| `certificates` | Fase 2            |
| `lifecycle`    | Fase 3            |
| FUSE           | Fase 4            |
| S3 API         | Fase 4            |
| Auth real      | Fase 4            |

---

## Alcance detallado

### Qué está incluido

- **Descubrimiento mDNS**: `_all4one._tcp.local` con TXT records. Al arrancar un
  segundo nodo en la misma red, ambos se descubren en segundos sin configuración.
- **Seeds fijos**: lista de IPs en `agent.toml` como fallback si mDNS no está disponible.
- **SWIM gossip**: heartbeat UDP cada 10s, detección de fallos SUSPECTED/OFFLINE,
  piggybacking de recursos en cada heartbeat.
- **Scheduler best-effort**: algoritmo de placement completo (locality/ventana/recursos/tier)
  sobre un snapshot de ClusterState. Sin Raft — el estado de jobs es local a cada nodo.
- **Executor completo**:
  - `docker.rs`: lanza contenedores via Docker socket. En Linux: `/var/run/docker.sock`.
    En Windows: `npipe:////./pipe/docker_engine` (Docker Desktop). Aplica límites de CPU/RAM.
  - `jar.rs`: lanza JARs con `java -jar`. Requiere `capabilities.java` configurado.
  - `python.rs`: lanza scripts Python. Requiere `capabilities.python` configurado.
  - `executable.rs`: lanza binarios nativos. Verifica plataforma compatible.
  - `wasm.rs`: lanza módulos WASM con wasmtime embebido.
- **Límites de recursos por plataforma** (aplicados en el executor al lanzar cada proceso):
  - Linux: cgroups v2 (`cpu.max`, `memory.max` en el cgroup del proceso).
  - macOS: `posix_spawnattr` + `task_policy_set` para CPU; `setrlimit(RLIMIT_AS)` para RAM.
  - Windows: **Job Objects** — `CreateJobObject` + `SetInformationJobObject` con
    `JobObjectExtendedLimitInformation` (`ProcessMemoryLimit`, `ActiveProcessLimit`).
    La cuota de CPU se implementa con `SetInformationJobObject` +
    `JobObjectCpuRateControlInformation` (`JOB_OBJECT_CPU_RATE_CONTROL_ENABLE`).
    Si el proceso no puede asignarse al Job Object (ej. ya pertenece a otro),
    el executor registra un warning y continúa — los límites no se aplican pero
    el job se ejecuta igualmente.
- **Captura de output**: stdout/stderr hasta 10 MB por job, truncado con aviso.
- **Streaming de output**: `GET /v1/jobs/{id}/output/stream` via SSE.
- **API REST (subset)**:
  - `POST /v1/jobs`
  - `GET /v1/jobs/{id}`
  - `GET /v1/jobs/{id}/output`
  - `GET /v1/jobs/{id}/output/stream`
  - `GET /v1/jobs`
  - `DELETE /v1/jobs/{id}`
  - `GET /v1/nodes`
  - `GET /v1/nodes/{id}`
  - `GET /v1/cluster/status`
  - `GET /health`
  - `GET /metrics`
- **Seguridad modo dev**: `shared_secret` en header `X-All4One-Secret`. Sin TLS.

### Qué NO está incluido

- Storage distribuido (no hay `PUT /v1/storage/`). Los `DataMount` en el JobSpec
  se ignoran silenciosamente en Fase 1 (campo presente en el schema pero sin efecto).
- Raft: el estado de jobs no se replica. Si el nodo scheduler cae, los jobs
  queued o scheduled se pierden.
- mTLS: las conexiones gRPC no están cifradas.
- Deduplicación fuerte de jobs: si el mismo job llega a dos schedulers
  simultáneamente, puede ejecutarse dos veces. Los jobs deben ser idempotentes.
- Endpoint `POST /v1/admin/tokens` y `DELETE /v1/admin/nodes/{id}` (requieren Raft).

---

## Estructura de carpetas Rust (Fase 1)

```
all4one/
├── Cargo.toml                    # workspace con members: agent, common
├── Cargo.lock
├── proto/
│   ├── agent.proto               # AgentService (sin RaftService en Fase 1)
│   └── build.rs                  # genera código Rust con prost
├── common/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── types.rs              # NodeId, JobId, ChunkId, ...
│       ├── job.rs                # JobSpec, JobStatus, JobEvent, ...
│       └── node.rs               # NodeProfile, NodeInfo, ClusterState, ...
└── agent/
    ├── Cargo.toml
    └── src/
        ├── main.rs               # tokio::main, inicializa módulos, inyecta Arc<>
        ├── config/
        │   ├── mod.rs
        │   └── schema.rs
        ├── node/
        │   └── mod.rs
        ├── discovery/
        │   ├── mod.rs
        │   ├── mdns.rs
        │   └── seeds.rs
        ├── gossip/
        │   ├── mod.rs
        │   ├── swim.rs
        │   └── state.rs
        ├── scheduler/
        │   ├── mod.rs
        │   ├── placement.rs
        │   └── queue.rs
        ├── executor/
        │   ├── mod.rs
        │   ├── docker.rs
        │   ├── jar.rs
        │   ├── python.rs
        │   ├── executable.rs
        │   └── wasm.rs
        ├── api_rest/
        │   ├── mod.rs
        │   ├── jobs.rs
        │   ├── nodes.rs
        │   ├── cluster.rs
        │   └── middleware.rs     # shared_secret extractor
        ├── grpc_server/
        │   ├── mod.rs
        │   └── agent_service.rs  # implementa AgentService
        └── grpc_client/
            └── mod.rs
```

---

## Criterios de aceptación (Fase 1)

Los siguientes tests deben pasar antes de declarar Fase 1 completa:

1. **Descubrimiento**: portátil Linux + Raspberry Pi en la misma red WiFi.
   Al arrancar el segundo agente, ambos aparecen en `GET /v1/nodes` del primero
   en menos de **15 segundos**.

2. **Placement correcto**: un job con `runtime: docker` y `constraints.requires_capabilities.docker: true`
   se asigna únicamente a un nodo con `capabilities.docker: true`, aunque el
   scheduler receptor no tenga Docker.

3. **Latencia submit → RUNNING < 3 segundos**: desde `POST /v1/jobs` hasta que
   `GET /v1/jobs/{id}` devuelve `status: running`.

4. **Streaming de output en tiempo real**: `GET /v1/jobs/{id}/output/stream` emite
   líneas de stdout a medida que el proceso las produce, con latencia < 1 segundo
   por línea.

5. **Detección de fallos**: al desconectar el cable de red del nodo B, el nodo A
   lo marca SUSPECTED en < 35 segundos y OFFLINE en < 100 segundos.

6. **RAM del agente en reposo**: el proceso `all4one-agent` con scheduler + executor
   activos y 0 jobs corriendo consume < **20 MB de RAM** en Linux ARM64.

7. **Cross-platform**: el binario compila y arranca sin errores en:
   - Linux x86_64 (`cargo build --target x86_64-unknown-linux-gnu`)
   - Linux ARM64 (`cargo build --target aarch64-unknown-linux-gnu`)
   - macOS ARM64 (`cargo build --target aarch64-apple-darwin`)
   - Windows x86_64 (`cargo build --target x86_64-pc-windows-msvc`)

8. **Límites de recursos en Windows**: un job Docker con `resources.memory_mb: 512`
   en un nodo Windows no puede consumir más de 512 MB. Verificable con Process Explorer
   observando el Job Object asignado al contenedor.

9. **Cola y retry**: al enviar un job con `constraints.tier_min: 0` cuando no
   hay ningún nodo Tier 0 online, el job queda en `status: queued`. Al arrancar
   un nodo Tier 0, el job se asigna y ejecuta en < 15 segundos.

---

## Dependencias con fases anteriores

Ninguna — Fase 1 es la base.

---

## Dependencias de crates (Fase 1)

```toml
# agent/Cargo.toml
[dependencies]
tokio       = { version = "1", features = ["full"] }
axum        = "0.7"
tonic       = "0.11"
prost       = "0.12"
serde       = { version = "1", features = ["derive"] }
serde_json  = "1"
serde_yaml  = "0.9"
mdns-sd     = "0.9"
chitchat    = "0.6"
bincode     = "1"
uuid        = { version = "1", features = ["v4", "serde"] }
chrono      = { version = "0.4", features = ["serde"] }
tracing     = "0.1"
tracing-subscriber = { version = "0.3", features = ["json"] }
anyhow      = "1"
thiserror   = "1"
wasmtime    = "20"
toml        = "0.8"
```

---

## Lista de tareas ordenadas

Las tareas están ordenadas por dependencia: cada tarea puede empezarse solo cuando
las anteriores están completas. El test de cada tarea es el criterio de "hecho"
para esa unidad de trabajo específica — son más granulares que los criterios de
aceptación finales de la fase.

---

### Tarea 1 — Workspace Rust y estructura de crates

**Qué hacer**: crear el workspace Cargo con los crates `common` y `agent`.
Definir los tipos base en `common`: `NodeId`, `JobId`, `ChunkId`, `FileId`
como newtypes sobre `Uuid`. Añadir `NodeProfile`, `NodeResources`,
`NodeCapabilities`, `ClusterState`, `NodeInfo` en `common/src/node.rs`.
Añadir `JobSpec`, `JobStatus`, `JobEvent` en `common/src/job.rs`.

**Test**:
```bash
cargo build --workspace
# Sin errores de compilación en ningún crate.
cargo test --workspace
# Todos los unit tests pasan (serialización/deserialización serde de cada struct).
```

---

### Tarea 2 — Módulo `config`: carga de agent.toml

**Qué hacer**: implementar `config/schema.rs` con todos los structs de configuración.
Implementar `config/mod.rs` que carga el fichero, valida campos requeridos y
devuelve `Arc<Config>`. Si falta un campo requerido o el TOML tiene sintaxis
inválida, el proceso termina con código 1 y mensaje descriptivo.

**Test**:
```bash
# Con config válida:
all4one-agent start --config /etc/all4one/agent.toml
# Arranca sin errores.

# Con campo requerido ausente (eliminar node.tier):
all4one-agent start --config /tmp/bad.toml
# Termina con exit code 1.
# stderr contiene: "node.tier is required"

# Con TOML inválido:
echo "node.tier = [no válido" > /tmp/invalid.toml
all4one-agent start --config /tmp/invalid.toml
# Termina con exit code 1.
# stderr contiene el error de parsing TOML con número de línea.
```

---

### Tarea 3 — Módulo `node`: identidad persistente

**Qué hacer**: implementar `node/mod.rs`. La función `node_id()` lee
`{data_dir}/node-id` si existe; si no, genera un UUID v4, lo escribe y lo devuelve.
La función `profile()` construye el `NodeProfile` combinando config y
capabilities detectadas en disco.

**Test**:
```bash
# Primer arranque — genera node-id:
all4one-agent start --config /tmp/test.toml &
cat /var/lib/all4one/node-id
# Contiene un UUID v4 válido (formato xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx).

# Segundo arranque — mismo UUID:
UUID1=$(cat /var/lib/all4one/node-id)
pkill all4one-agent && all4one-agent start --config /tmp/test.toml &
UUID2=$(cat /var/lib/all4one/node-id)
[ "$UUID1" = "$UUID2" ] && echo "OK — mismo UUID" || echo "FAIL"

# GET /v1/nodes devuelve el node_id correcto:
curl -s http://localhost:7946/v1/nodes | python3 -m json.tool | grep '"id"'
# "id": "<UUID coincide con node-id en disco>"
```

---

### Tarea 4 — Módulo `api_rest`: servidor HTTP base

**Qué hacer**: arrancar el servidor axum en el puerto configurado. Implementar
los handlers `/health` y `/metrics`. Implementar el middleware de `shared_secret`
para `mode=dev`. Añadir el header `X-Request-Id` en todas las respuestas.

**Test**:
```bash
# Health check básico:
curl -s http://localhost:7946/health
# { "status": "ok", "node_id": "...", "uptime_seconds": N, ... }

# Sin secret en modo dev → 401:
curl -s -o /dev/null -w "%{http_code}" http://localhost:7946/v1/nodes
# 401

# Con secret incorrecto → 401:
curl -s -o /dev/null -w "%{http_code}" \
  -H "X-All4One-Secret: wrong" http://localhost:7946/v1/nodes
# 401

# Con secret correcto → 200:
curl -s -o /dev/null -w "%{http_code}" \
  -H "X-All4One-Secret: mi-secreto" http://localhost:7946/v1/nodes
# 200

# Header X-Request-Id presente en cada respuesta:
curl -s -I http://localhost:7946/health | grep -i "x-request-id"
# x-request-id: <UUID>
```

---

### Tarea 5 — Módulo `grpc_server`: servidor gRPC base con shared_secret

**Qué hacer**: arrancar el servidor tonic en el puerto configurado. Implementar
el interceptor de `shared_secret` en metadata gRPC. Implementar el RPC
`GetClusterState` que devuelve el `ClusterState` local actual.

**Test**:
```bash
# Sin secret → UNAUTHENTICATED:
grpcurl -plaintext \
  -H "x-all4one-secret: wrong" \
  localhost:7947 all4one.agent.v1.AgentService/GetClusterState
# Error: Code: Unauthenticated

# Con secret correcto → respuesta con ClusterState:
grpcurl -plaintext \
  -H "x-all4one-secret: mi-secreto" \
  localhost:7947 all4one.agent.v1.AgentService/GetClusterState
# { "stateJson": "...", "version": "0" }
```

---

### Tarea 6 — Módulo `discovery`: mDNS + seeds

**Qué hacer**: implementar `discovery/mdns.rs` que anuncia
`_all4one._tcp.local` con los TXT records correctos y escucha anuncios de
otros nodos. Implementar `discovery/seeds.rs` que conecta a cada seed al
arrancar y solicita `GetClusterState`. Ambos envían `DiscoveredNode` al
canal mpsc del módulo `gossip`.

**Test** (requiere dos máquinas o dos instancias en localhost con puertos distintos):
```bash
# Instancia A en puerto 7946/7947:
all4one-agent start --config config-a.toml &

# Instancia B con seed apuntando a A:
# seeds = ["127.0.0.1:7947"]
all4one-agent start --config config-b.toml &

# Tras < 5 segundos, A conoce a B:
curl -s -H "X-All4One-Secret: s" http://localhost:7946/v1/nodes | \
  python3 -c "import sys,json; d=json.load(sys.stdin); print(len(d['nodes']))"
# 2
```

---

### Tarea 7 — Módulo `gossip`: SWIM y ClusterState

**Qué hacer**: implementar SWIM sobre UDP. El nodo envía Ping cada 10s a un
peer aleatorio, procesa Ack/PingReq/Suspect/Alive/Dead. Actualiza
`ClusterState` y publica `MembershipEvent` via `tokio::broadcast`. Implementar
la tarea de scrubbing que transiciona SUSPECTED → OFFLINE.

**Test**:
```bash
# Con dos nodos corriendo, desconectar la red del nodo B (o matarlo):
pkill -f "config-b.toml"

# Nodo A marca B como SUSPECTED en < 35s:
sleep 35
curl -s -H "X-All4One-Secret: s" http://localhost:7946/v1/nodes | \
  python3 -c "import sys,json; nodes=json.load(sys.stdin)['nodes']; \
  [print(n['profile']['id'], n['status']) for n in nodes]"
# <id-B> suspected

# Nodo A marca B como OFFLINE en < 100s desde la desconexión:
sleep 70
curl -s -H "X-All4One-Secret: s" http://localhost:7946/v1/nodes | \
  python3 -c "import sys,json; nodes=json.load(sys.stdin)['nodes']; \
  [print(n['profile']['id'], n['status']) for n in nodes]"
# <id-B> offline
```

---

### Tarea 8 — Módulo `executor`: runtime Docker

**Qué hacer**: implementar `executor/docker.rs`. Lanza un contenedor Docker
con los límites de CPU/RAM del `JobSpec`. Captura stdout/stderr. Publica
`JobEvent::Started`, `JobEvent::OutputLine`, `JobEvent::Completed` o
`JobEvent::Failed`. Implementar `kill()` con SIGTERM→30s→SIGKILL.

**Test**:
```bash
# Job que imprime líneas y termina con exit_code=0:
JOB_ID=$(curl -s -X POST http://localhost:7946/v1/jobs \
  -H "Content-Type: application/yaml" \
  -H "X-All4One-Secret: s" \
  -d 'runtime: docker
source: alpine:3.19
command: ["sh", "-c", "echo hello && sleep 1 && echo world"]
resources: {cpu_cores: 1, memory_mb: 128}' | python3 -c "import sys,json; print(json.load(sys.stdin)['job_id'])")

sleep 5
curl -s -H "X-All4One-Secret: s" http://localhost:7946/v1/jobs/$JOB_ID/output
# { "stdout": "hello\nworld\n", "stderr": "", "truncated": false }

curl -s -H "X-All4One-Secret: s" http://localhost:7946/v1/jobs/$JOB_ID | \
  python3 -c "import sys,json; print(json.load(sys.stdin)['exit_code'])"
# 0

# Job con límite de memoria — OOM:
curl -s -X POST http://localhost:7946/v1/jobs \
  -H "Content-Type: application/yaml" \
  -H "X-All4One-Secret: s" \
  -d 'runtime: docker
source: python:3.11-slim
command: ["python", "-c", "x=[bytearray(1024*1024) for _ in range(1000)]"]
resources: {cpu_cores: 1, memory_mb: 64}'
# El job termina con status=failed y error contiene "OOMKilled" o similar.
```

---

### Tarea 9 — Módulo `executor`: runtimes Python, JAR, Executable, WASM

**Qué hacer**: implementar `executor/python.rs`, `executor/jar.rs`,
`executor/executable.rs`, `executor/wasm.rs`. Cada uno implementa el trait
`Runtime`. Verificar capabilities antes de lanzar (si el nodo no tiene Python,
rechazar con error claro).

**Test**:
```bash
# Python:
curl -s -X POST http://localhost:7946/v1/jobs \
  -H "Content-Type: application/yaml" -H "X-All4One-Secret: s" \
  -d 'runtime: python
source: ""
command: ["-c", "import sys; print(sys.version)"]
resources: {cpu_cores: 1, memory_mb: 128}'
# stdout contiene la versión de Python del nodo.

# Executable (binario nativo):
curl -s -X POST http://localhost:7946/v1/jobs \
  -H "Content-Type: application/yaml" -H "X-All4One-Secret: s" \
  -d 'runtime: executable
source: "volatile://bins/echo-test"
command: ["--message=hola"]
resources: {cpu_cores: 1, memory_mb: 64}'
# stdout: "hola"

# WASM:
curl -s -X POST http://localhost:7946/v1/jobs \
  -H "Content-Type: application/yaml" -H "X-All4One-Secret: s" \
  -d 'runtime: wasm
source: "volatile://wasm/hello.wasm"
command: []
resources: {cpu_cores: 1, memory_mb: 64}'
# stdout: salida del módulo WASM.

# Nodo sin Python — constraints correctas:
# En un nodo con capabilities.python = null:
curl -s -X POST http://localhost:7946/v1/jobs \
  -H "Content-Type: application/yaml" -H "X-All4One-Secret: s" \
  -d 'runtime: python
source: ""
command: ["-c", "print(1)"]
constraints: {requires_capabilities: {python_min: "3.11"}}
resources: {cpu_cores: 1, memory_mb: 128}'
# status: queued (no hay candidatos con Python, queda en cola)
```

---

### Tarea 10 — Módulo `scheduler`: placement y cola

**Qué hacer**: implementar `scheduler/placement.rs` con el algoritmo completo
de 8 pasos (filtrado + puntuación). Implementar `scheduler/queue.rs` con
`JobQueue` por prioridad. Implementar la tarea de reevaluación de la cola
suscrita a `MembershipEvent`.

**Test**:
```bash
# Placement correcto — job Docker va al nodo con Docker:
# Nodo A: capabilities.docker=true
# Nodo B: capabilities.docker=false
JOB=$(curl -s -X POST http://nodoA:7946/v1/jobs \
  -H "Content-Type: application/yaml" -H "X-All4One-Secret: s" \
  -d 'runtime: docker
source: alpine:3.19
command: ["echo", "ok"]
constraints: {requires_capabilities: {docker: true}}
resources: {cpu_cores: 1, memory_mb: 128}')
echo $JOB | python3 -c "import sys,json; print(json.load(sys.stdin)['assigned_to'])"
# UUID del nodo A (el único con Docker)

# Cola y reevaluación — job queda en cola hasta que aparece el nodo correcto:
# Con solo nodos Tier 1 online, enviar job con tier_min=0:
JOB_ID=$(curl -s -X POST ... -d '...tier_min: 0...' | python3 -c "...['job_id']")
curl -s http://localhost:7946/v1/jobs/$JOB_ID | python3 -c "...['status']"
# queued

# Arrancar un nodo Tier 0 → job se asigna en < 15s:
all4one-agent start --config tier0.toml &
sleep 15
curl -s http://localhost:7946/v1/jobs/$JOB_ID | python3 -c "...['status']"
# scheduled o running
```

---

### Tarea 11 — `grpc_client` + delegación de jobs entre nodos

**Qué hacer**: implementar `grpc_client/mod.rs` con pool de conexiones por
`NodeId`. Implementar `launch_job` que abre el stream de `JobEvent` y lo
relaya al gossip local. Conectar el scheduler para que delegue jobs al nodo
remoto cuando `nodo_elegido != self`.

**Test**:
```bash
# Enviar job al nodo A (scheduler), que lo delega al nodo B (executor):
# Nodo A: roles.executor=false, roles.scheduler=true
# Nodo B: roles.executor=true, roles.scheduler=false, capabilities.docker=true

JOB_ID=$(curl -s -X POST http://nodoA:7946/v1/jobs \
  -H "Content-Type: application/yaml" -H "X-All4One-Secret: s" \
  -d 'runtime: docker
source: alpine:3.19
command: ["sh", "-c", "hostname && sleep 2"]
constraints: {requires_capabilities: {docker: true}}
resources: {cpu_cores: 1, memory_mb: 128}' | python3 -c "import sys,json; print(json.load(sys.stdin)['job_id'])")

# assigned_to debe ser nodo B:
curl -s -H "X-All4One-Secret: s" http://nodoA:7946/v1/jobs/$JOB_ID | \
  python3 -c "import sys,json; print(json.load(sys.stdin)['assigned_to'])"
# UUID del nodo B

# El output del job es accesible desde nodo A aunque lo ejecutó nodo B:
sleep 5
curl -s -H "X-All4One-Secret: s" http://nodoA:7946/v1/jobs/$JOB_ID/output | \
  python3 -c "import sys,json; print(json.load(sys.stdin)['stdout'])"
# nombre del hostname del contenedor en nodo B
```

---

### Tarea 12 — Streaming SSE de output en tiempo real

**Qué hacer**: implementar `GET /v1/jobs/{id}/output/stream` con Server-Sent
Events. El handler se suscribe al canal interno de `JobEvent` y emite cada
`OutputLine` como evento SSE. Si el job ya terminó, emite los eventos
históricos del buffer y cierra la conexión.

**Test**:
```bash
# Abrir el stream antes de que termine el job:
JOB_ID=$(curl -s -X POST http://localhost:7946/v1/jobs \
  -H "Content-Type: application/yaml" -H "X-All4One-Secret: s" \
  -d 'runtime: docker
source: alpine:3.19
command: ["sh", "-c", "for i in 1 2 3 4 5; do echo \"linea $i\"; sleep 1; done"]
resources: {cpu_cores: 1, memory_mb: 128}' | python3 -c "import sys,json; print(json.load(sys.stdin)['job_id'])")

# El stream emite líneas a medida que llegan (no al final):
time curl -s -N \
  -H "Accept: text/event-stream" \
  -H "X-All4One-Secret: s" \
  http://localhost:7946/v1/jobs/$JOB_ID/output/stream
# Debe mostrar "linea 1", "linea 2", ... con ~1s entre cada una.
# Al terminar: event: completed\ndata: {"exit_code": 0}
# Tiempo total: ~5 segundos (no espera a que termine para emitir la primera línea).

# Stream sobre job ya terminado emite histórico y cierra:
curl -s -N \
  -H "Accept: text/event-stream" \
  -H "X-All4One-Secret: s" \
  http://localhost:7946/v1/jobs/$JOB_ID/output/stream
# Emite las 5 líneas + completed y cierra inmediatamente.
```

---

### Tarea 13 — Cancelación de jobs (DELETE)

**Qué hacer**: implementar `DELETE /v1/jobs/{id}`. Si el job está en cola,
eliminarlo. Si está corriendo localmente, SIGTERM → espera 30s → SIGKILL.
Si está corriendo en otro nodo, delegar via gRPC. Devolver `status: cancelled`.

**Test**:
```bash
# Cancelar job en cola:
JOB_ID=$(curl -s -X POST ... -d '...tier_min: 0 (sin nodos Tier 0)...' | ...)
curl -s -X DELETE -H "X-All4One-Secret: s" http://localhost:7946/v1/jobs/$JOB_ID | \
  python3 -c "import sys,json; print(json.load(sys.stdin)['status'])"
# cancelled

# Cancelar job corriendo — SIGTERM funciona:
JOB_ID=$(curl -s -X POST ... -d '...command: ["sleep", "60"]...' | ...)
sleep 2  # esperar a que esté running
TIME_START=$SECONDS
curl -s -X DELETE -H "X-All4One-Secret: s" http://localhost:7946/v1/jobs/$JOB_ID
echo "Tardó $((SECONDS - TIME_START))s"
# Debe ser < 5s (el proceso responde a SIGTERM)

# DELETE sobre job ya terminado → 409:
curl -s -o /dev/null -w "%{http_code}" \
  -X DELETE -H "X-All4One-Secret: s" \
  http://localhost:7946/v1/jobs/$JOB_ID
# 409
```

---

### Tarea 14 — Compilación Windows x86_64 y Job Objects

**Qué hacer**: asegurar que el crate compila para `x86_64-pc-windows-msvc`.
Implementar en `executor/mod.rs` la rama `#[cfg(target_os = "windows")]` que
aplica límites de recursos via `CreateJobObject` /
`SetInformationJobObject`. El socket Docker en Windows es
`npipe:////./pipe/docker_engine`.

**Test**:
```bash
# Cross-compilar desde Linux (requiere cross o toolchain Windows):
cross build --target x86_64-pc-windows-msvc --release
# Sin errores de compilación.

# En una máquina Windows con Docker Desktop:
.\all4one-agent.exe start --config agent.toml
# Arranca sin errores.
# GET /health → 200 OK.

# Job Docker con límite de memoria en Windows:
# Enviar job con memory_mb: 256.
# Verificar con Process Explorer o Get-Process que el Job Object del
# contenedor tiene ProcessMemoryLimit = 268435456 (256 MB en bytes).
```

---

### Tarea 15 — Prueba de integración final (criterios de aceptación de fase)

**Qué hacer**: ejecutar el conjunto completo de criterios de aceptación de la
fase con el hardware objetivo real.

**Test**: ver sección [Criterios de aceptación](#criterios-de-aceptación-fase-1)
de este documento. Todos los criterios 1–9 deben pasar antes de cerrar Fase 1.

```bash
# Resumen rápido de smoke test:
# 1. Dos nodos se descubren en < 15s ✓
# 2. Placement respeta capabilities ✓
# 3. Latencia submit → RUNNING < 3s ✓
# 4. Streaming SSE con latencia < 1s por línea ✓
# 5. Detección de fallo SUSPECTED < 35s, OFFLINE < 100s ✓
# 6. RAM en reposo < 20 MB en ARM64 ✓
# 7. Compila en Linux x86_64, ARM64, macOS ARM64, Windows x86_64 ✓
# 8. Job Objects aplica límites de memoria en Windows ✓
# 9. Cola reevalúa al aparecer nodo con tier correcto en < 15s ✓
```
