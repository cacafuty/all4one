# Referencia de configuración — agent.toml

El agente carga su configuración desde `agent.toml` al arrancar. Si el fichero
no existe o tiene errores de sintaxis, el proceso termina con código 1 y un
mensaje de error descriptivo en stderr.

---

## Ejemplo completo

```toml
[node]
tier = 0
availability = "always"
quorum_participant = true
data_dir = "/var/lib/all4one"
reliability_score_initial = 0.5

[roles]
scheduler = true
executor = true
storage = true

[network]
bind_address = "0.0.0.0"
grpc_port = 7947
rest_port = 7946
metrics_port = 9090
advertise_address = "192.168.1.100"

[discovery]
mdns = true
seeds = ["192.168.1.100:7947", "192.168.1.101:7947"]

[security]
mode = "prod"
shared_secret = ""
cert_dir = "/var/lib/all4one/certs"

[executor]
max_concurrent_jobs = 8
docker_socket = "/var/run/docker.sock"
cgroups_enabled = true
output_max_bytes = 10485760

[storage]
storage_path = "/var/lib/all4one/storage"
chunk_size_mb = 64
encryption = false
default_policy = "auto"
default_min_replicas = 2
restore_ttl_hours = 24

[gossip]
heartbeat_interval_secs = 10
suspect_timeout_secs = 30
dead_timeout_secs = 60
fanout = 3

[raft]
election_timeout_min_ms = 150
election_timeout_max_ms = 300
heartbeat_interval_ms = 50
snapshot_threshold = 1000

[capabilities]
docker = true
java = "/usr/bin/java"
python = "/usr/bin/python3"
wasm = true
gpu_enabled = false
cuda_path = ""
executables_path = "/var/lib/all4one/executables"

[logging]
level = "info"
format = "json"
file = ""
```

---

## Sección `[node]`

### `tier`
- **Tipo Rust**: `Tier` (enum: `0 | 1 | 2`)
- **Default**: no tiene — campo requerido
- **Valores válidos**: `0` (Tier 0, 24/7), `1` (Tier 1, horario predecible), `2` (Tier 2, oportunista)
- **Si se omite**: error en arranque: `"node.tier is required"`

### `availability`
- **Tipo Rust**: `AvailabilitySchedule` (enum serializado como String)
- **Default**: no tiene — campo requerido
- **Valores válidos**:
  - `"always"` — Tier 0 siempre disponible
  - `"cron:0 9-18 * * 1-5"` — expresión cron que define la ventana de disponibilidad
  - `"manual"` — el nodo se considera disponible solo cuando está online
  - `"learned"` — el agente infiere el patrón de disponibilidad (Fase 5, requiere 2 semanas de historial)
- **Si se omite**: error en arranque

### `quorum_participant`
- **Tipo Rust**: `bool`
- **Default**: `false`
- **Valores válidos**: `true | false`
- **Si se omite**: `false` — el nodo no participa en quórum Raft ni almacena metadata crítica
- **Nota**: los nodos Tier 2 deben tener `quorum_participant = false`

### `data_dir`
- **Tipo Rust**: `PathBuf`
- **Default**: no tiene — campo requerido
- **Valores válidos**: cualquier path absoluto con permisos de escritura
- **Si se omite**: error en arranque
- **Contenido**: `node-id`, `certs/`, logs internos

### `reliability_score_initial`
- **Tipo Rust**: `f32`
- **Default**: `0.5`
- **Valores válidos**: `0.0` a `1.0`
- **Si se omite**: `0.5`
- **Nota**: el score real se actualiza por el clúster con historial de jobs completados/fallados

---

## Sección `[roles]`

### `scheduler`
- **Tipo Rust**: `bool`
- **Default**: `true`
- **Si se omite**: `true`
- **Efecto**: activa el módulo scheduler. El nodo acepta `POST /v1/jobs` y ejecuta el algoritmo de placement.

### `executor`
- **Tipo Rust**: `bool`
- **Default**: `true`
- **Si se omite**: `true`
- **Efecto**: activa el módulo executor. El nodo puede ejecutar jobs asignados por cualquier scheduler del clúster.

### `storage`
- **Tipo Rust**: `bool`
- **Default**: `false`
- **Si se omite**: `false`
- **Efecto** (Fase 2+): activa el módulo storage. El nodo almacena chunks y sirve lecturas.

---

## Sección `[network]`

### `bind_address`
- **Tipo Rust**: `IpAddr`
- **Default**: `"0.0.0.0"`
- **Si se omite**: `"0.0.0.0"` (escucha en todas las interfaces)

### `grpc_port`
- **Tipo Rust**: `u16`
- **Default**: `7947`
- **Si se omite**: `7947`
- **Nota**: mismo número de puerto para TCP (gRPC) y UDP (SWIM)

### `rest_port`
- **Tipo Rust**: `u16`
- **Default**: `7946`
- **Si se omite**: `7946`

### `metrics_port`
- **Tipo Rust**: `u16`
- **Default**: `9090`
- **Si se omite**: `9090`

### `advertise_address`
- **Tipo Rust**: `Option<IpAddr>`
- **Default**: `null` (el agente detecta la IP automáticamente)
- **Si se omite**: el agente usa la IP de la interfaz de red principal
- **Cuándo especificar**: cuando el agente está detrás de NAT o tiene múltiples interfaces y la detección automática no elige la correcta

---

## Sección `[discovery]`

### `mdns`
- **Tipo Rust**: `bool`
- **Default**: `true`
- **Si se omite**: `true`
- **Efecto**: activa el descubrimiento mDNS en redes locales. Deshabilitar en redes donde mDNS no funciona (VPN sin multicast, redes segmentadas).

### `seeds`
- **Tipo Rust**: `Vec<SocketAddr>`
- **Default**: `[]`
- **Si se omite**: `[]`
- **Formato**: lista de `"IP:puerto"` de nodos conocidos del clúster
- **Ejemplo**: `["192.168.1.100:7947", "192.168.1.101:7947"]`
- **Nota**: no es necesario listar todos los nodos — con uno accesible el agente obtiene el ClusterState completo

---

## Sección `[security]`

### `mode`
- **Tipo Rust**: `SecurityMode` (enum: `"dev" | "prod"`)
- **Default**: no tiene — campo requerido
- **Valores válidos**:
  - `"dev"` — shared_secret en headers, sin TLS. El agente imprime advertencia en arranque.
  - `"prod"` — mTLS con PKI interna (Fase 2+). Requiere certificados en `cert_dir`.
- **Si se omite**: error en arranque

### `shared_secret`
- **Tipo Rust**: `String`
- **Default**: `""`
- **Si se omite**: `""` (cualquier valor pasa en modo dev si el campo está vacío)
- **Solo relevante en**: `mode = "dev"`
- **Nota**: si `mode = "dev"` y `shared_secret = ""`, el agente acepta requests sin validar el header. Solo para desarrollo local aislado.

### `cert_dir`
- **Tipo Rust**: `PathBuf`
- **Default**: `"{data_dir}/certs"`
- **Si se omite**: `"{data_dir}/certs"`
- **Solo relevante en**: `mode = "prod"`
- **Contenido esperado**:
  - `node.key` (permisos 0600) — clave privada Ed25519 del nodo
  - `node.crt` — certificado firmado por la CA del clúster
  - `ca.crt` — certificado público de la CA del clúster

---

## Sección `[executor]`

### `max_concurrent_jobs`
- **Tipo Rust**: `u32`
- **Default**: `8`
- **Si se omite**: `8`
- **Nota**: el scheduler no asigna más jobs a este nodo si ya tiene `max_concurrent_jobs` corriendo

### `docker_socket`
- **Tipo Rust**: `PathBuf`
- **Default**: `"/var/run/docker.sock"`
- **Si se omite**: `"/var/run/docker.sock"`
- **Solo relevante si**: `capabilities.docker = true`

### `cgroups_enabled`
- **Tipo Rust**: `bool`
- **Default**: `true` en Linux, `false` en macOS/Windows
- **Si se omite**: detectado automáticamente por plataforma
- **Por plataforma**:
  - Linux: cgroups v2 (`cpu.max`, `memory.max`). Requiere kernel >= 5.4 y permisos de escritura en `/sys/fs/cgroup`.
  - macOS: campo ignorado. Los límites se aplican via `task_policy_set` (CPU) y `setrlimit(RLIMIT_AS)` (RAM).
  - Windows: campo ignorado. Los límites se aplican via Job Objects — `JobObjectCpuRateControlInformation` para CPU y `JobObjectExtendedLimitInformation.ProcessMemoryLimit` para RAM. Si el proceso ya pertenece a otro Job Object, el executor registra un warning y el job continúa sin límites aplicados.

### `output_max_bytes`
- **Tipo Rust**: `usize`
- **Default**: `10485760` (10 MB)
- **Si se omite**: `10485760`
- **Efecto**: cuando el output capturado de un job supera este límite, se trunca y se añade `[OUTPUT TRUNCATED]`

---

## Sección `[storage]`

### `storage_path`
- **Tipo Rust**: `PathBuf`
- **Default**: `"{data_dir}/storage"`
- **Si se omite**: `"{data_dir}/storage"`
- **Solo relevante si**: `roles.storage = true`

### `chunk_size_mb`
- **Tipo Rust**: `u32`
- **Default**: `64`
- **Valores válidos**: `1` a `512`
- **Si se omite**: `64`
- **Nota**: configurable por bucket via headers en la API. Este valor es el default del nodo.

### `encryption`
- **Tipo Rust**: `bool`
- **Default**: `false`
- **Si se omite**: `false`
- **Efecto**: si `true`, cada chunk se cifra con AES-256-GCM antes de escribir en disco

### `default_policy`
- **Tipo Rust**: `String`
- **Default**: `"auto"`
- **Valores válidos**: `"auto" | "manual" | "tiered"`
- **Si se omite**: `"auto"`
- **Nota**: política de storage aplicada cuando el cliente no especifica `X-All4One-Policy`

### `default_min_replicas`
- **Tipo Rust**: `u8`
- **Default**: `2`
- **Valores válidos**: `1` a `8`
- **Si se omite**: `2`

### `restore_ttl_hours`
- **Tipo Rust**: `u32`
- **Default**: `24`
- **Si se omite**: `24`
- **Efecto**: cuántas horas permanece accesible un objeto restaurado desde Archive antes de re-archivarse

---

## Sección `[gossip]`

### `heartbeat_interval_secs`
- **Tipo Rust**: `u64`
- **Default**: `10`
- **Si se omite**: `10`
- **Nota**: reducir aumenta el tráfico de red; aumentar ralentiza la detección de fallos

### `suspect_timeout_secs`
- **Tipo Rust**: `u64`
- **Default**: `30`
- **Si se omite**: `30`
- **Efecto**: segundos sin respuesta directa ni indirecta antes de marcar un nodo SUSPECTED

### `dead_timeout_secs`
- **Tipo Rust**: `u64`
- **Default**: `60`
- **Si se omite**: `60`
- **Efecto**: segundos adicionales en SUSPECTED antes de marcar OFFLINE

### `fanout`
- **Tipo Rust**: `u32`
- **Default**: `3`
- **Si se omite**: `3`
- **Efecto**: número de nodos K elegidos para indirect probing cuando un nodo no responde directamente

---

## Sección `[raft]`

Solo relevante si `node.quorum_participant = true`.

### `election_timeout_min_ms`
- **Tipo Rust**: `u64`
- **Default**: `150`
- **Si se omite**: `150`

### `election_timeout_max_ms`
- **Tipo Rust**: `u64`
- **Default**: `300`
- **Si se omite**: `300`

### `heartbeat_interval_ms`
- **Tipo Rust**: `u64`
- **Default**: `50`
- **Si se omite**: `50`
- **Nota**: debe ser menor que `election_timeout_min_ms`

### `snapshot_threshold`
- **Tipo Rust**: `u64`
- **Default**: `1000`
- **Si se omite**: `1000`
- **Efecto**: número de entradas en el log Raft antes de hacer un snapshot para compactar

---

## Sección `[capabilities]`

### `docker`
- **Tipo Rust**: `bool`
- **Default**: `false`
- **Si se omite**: `false`
- **Verificación al arranque**: si `true`, comprueba que el socket Docker es accesible

### `java`
- **Tipo Rust**: `Option<PathBuf>`
- **Default**: `null`
- **Si se omite**: `null` (Java no disponible en este nodo)
- **Ejemplo**: `"/usr/bin/java"` o `"/usr/lib/jvm/java-21/bin/java"`
- **Verificación al arranque**: si especificado, ejecuta `java -version` y extrae la versión

### `python`
- **Tipo Rust**: `Option<PathBuf>`
- **Default**: `null`
- **Si se omite**: `null`
- **Ejemplo**: `"/usr/bin/python3"` o `"/home/user/.pyenv/shims/python3"`
- **Verificación al arranque**: si especificado, ejecuta `python3 --version`

### `wasm`
- **Tipo Rust**: `bool`
- **Default**: `false`
- **Si se omite**: `false`
- **Nota**: usa `wasmtime` embebido — no requiere binario externo

### `gpu_enabled`
- **Tipo Rust**: `bool`
- **Default**: `false`
- **Si se omite**: `false`
- **Verificación al arranque**: si `true`, detecta GPU via NVML (Nvidia) o equivalente

### `cuda_path`
- **Tipo Rust**: `Option<PathBuf>`
- **Default**: `null`
- **Si se omite**: `null`
- **Solo relevante si**: `gpu_enabled = true`
- **Ejemplo**: `"/usr/local/cuda-12.1"`

### `executables_path`
- **Tipo Rust**: `PathBuf`
- **Default**: `"{data_dir}/executables"`
- **Si se omite**: `"{data_dir}/executables"`
- **Descripción**: directorio donde se descargan y almacenan los ejecutables nativos para el runtime `executable`

---

## Sección `[logging]`

### `level`
- **Tipo Rust**: `String`
- **Default**: `"info"`
- **Valores válidos**: `"trace" | "debug" | "info" | "warn" | "error"`
- **Si se omite**: `"info"`

### `format`
- **Tipo Rust**: `String`
- **Default**: `"json"`
- **Valores válidos**: `"json"` (estructurado para ingestión) | `"text"` (legible para humanos)
- **Si se omite**: `"json"`

### `file`
- **Tipo Rust**: `Option<PathBuf>`
- **Default**: `null` (logs a stderr)
- **Si se omite**: logs a stderr
- **Ejemplo**: `"/var/log/all4one/agent.log"`
