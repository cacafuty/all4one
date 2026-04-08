# Módulos del agente

El agente es un único binario Rust. Su código fuente se organiza en módulos con
responsabilidades estrictas. **Ningún módulo accede directamente a los datos internos
de otro**: toda comunicación entre módulos ocurre via canales `tokio` o a través del
`ClusterState` compartido.

---

## Estructura de crates

```
all4one/
├── Cargo.toml                  workspace
├── agent/                      binario principal
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs             arranque, inicialización de módulos
│       ├── config/             carga y validación de agent.toml
│       │   ├── mod.rs
│       │   └── schema.rs       structs Config, NodeConfig, RolesConfig, ...
│       ├── node/               identidad del nodo
│       │   └── mod.rs
│       ├── discovery/          mDNS + seeds
│       │   ├── mod.rs
│       │   ├── mdns.rs
│       │   └── seeds.rs
│       ├── gossip/             SWIM, ClusterState, MembershipEvent
│       │   ├── mod.rs
│       │   ├── swim.rs
│       │   └── state.rs
│       ├── raft/               consenso embebido (Fase 2+)
│       │   ├── mod.rs
│       │   ├── store.rs        implementa openraft::RaftStorage
│       │   └── network.rs      implementa openraft::RaftNetwork via gRPC
│       ├── scheduler/          placement algorithm, JobQueue
│       │   ├── mod.rs
│       │   ├── placement.rs
│       │   └── queue.rs
│       ├── executor/           ciclo de vida de procesos
│       │   ├── mod.rs
│       │   ├── docker.rs
│       │   ├── jar.rs
│       │   ├── python.rs
│       │   ├── executable.rs
│       │   └── wasm.rs
│       ├── storage/            chunks locales (Fase 2+)
│       │   ├── mod.rs
│       │   ├── chunks.rs
│       │   └── index.rs
│       ├── lifecycle/          heat score + tiering (Fase 3+)
│       │   └── mod.rs
│       ├── certificates/       PKI interna (Fase 2+)
│       │   └── mod.rs
│       ├── api_rest/           axum HTTP server
│       │   ├── mod.rs
│       │   ├── jobs.rs
│       │   ├── nodes.rs
│       │   ├── cluster.rs
│       │   ├── storage.rs
│       │   └── middleware.rs
│       ├── grpc_server/        tonic gRPC server
│       │   ├── mod.rs
│       │   ├── agent_service.rs
│       │   └── raft_service.rs
│       └── grpc_client/        pool de conexiones gRPC salientes
│           └── mod.rs
├── common/                     tipos compartidos
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── types.rs            NodeId, JobId, ChunkId, FileId, ...
│       ├── job.rs              JobSpec, JobStatus, JobEvent, ...
│       ├── node.rs             NodeProfile, NodeInfo, ClusterState, ...
│       ├── storage.rs          ChunkMetadata, FileMetadata, StoragePolicy, ...
│       └── proto_conversions.rs From/Into entre tipos Rust y tipos proto
└── proto/
    ├── agent.proto             AgentService
    └── raft.proto              RaftService
```

---

## Módulo `config`

**Responsabilidad**: cargar y validar `agent.toml` al arranque.

```rust
// config/schema.rs — representación completa del fichero de configuración
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub node: NodeConfig,
    pub roles: RolesConfig,
    pub network: NetworkConfig,
    pub discovery: DiscoveryConfig,
    pub security: SecurityConfig,
    pub executor: ExecutorConfig,
    pub storage: StorageConfig,
    pub gossip: GossipConfig,
    pub raft: RaftConfig,
    pub capabilities: CapabilitiesConfig,
    pub logging: LoggingConfig,
}
```

- Usa `serde` + `toml` para deserialización.
- Expone `Config` como struct inmutable disponible globalmente via `Arc<Config>`.
- Valida al arranque que los ejecutables referenciados en `capabilities` existen en disco.
- **Sin dependencias de otros módulos.**
- Si `agent.toml` no existe o contiene campos inválidos, el proceso termina con
  código de salida 1 y un mensaje de error descriptivo.

---

## Módulo `node`

**Responsabilidad**: gestionar la identidad persistente del nodo.

```rust
// node/mod.rs
pub fn node_id() -> NodeId;          // lee o genera UUID de {data_dir}/node-id
pub fn profile() -> NodeProfile;     // combina Config + capabilities verificadas
```

- El UUID se genera en el primer arranque y se persiste en `{data_dir}/node-id`.
- Si el fichero existe pero contiene un UUID inválido, el arranque falla con error descriptivo.
- `profile()` es una función pura que no tiene estado mutable — construye el
  `NodeProfile` leyendo la config y verificando capabilities en tiempo de llamada.

---

## Módulo `discovery`

**Responsabilidad**: detectar otros nodos del clúster al arrancar.

Dos submódulos **independientes** que arrancan concurrentemente como tareas `tokio`:

### Submódulo `mdns`

```rust
// discovery/mdns.rs
pub async fn run(
    node_id: NodeId,
    profile: NodeProfile,
    tx: mpsc::Sender<DiscoveredNode>,
) -> Result<()>
```

- Anuncia `_all4one._tcp.local` con TXT records: `node_id`, `tier`, `port_grpc`,
  `port_rest`, `version`.
- Escucha anuncios de otros nodos.
- Al descubrir un nodo nuevo, lo envía al módulo `gossip` via `mpsc::Sender<DiscoveredNode>`.

### Submódulo `seeds`

```rust
// discovery/seeds.rs
pub async fn run(
    seeds: Vec<SocketAddr>,
    tx: mpsc::Sender<DiscoveredNode>,
    grpc_client: Arc<GrpcClientPool>,
) -> Result<()>
```

- En paralelo con mDNS, intenta conectar a cada seed de la config.
- Al conectar, solicita `GetClusterState` via gRPC y envía todos los nodos
  conocidos al módulo `gossip`.
- Reintenta con backoff exponencial si la conexión falla.

---

## Módulo `gossip`

**Responsabilidad**: mantener la vista de membresía del clúster.

### Protocolo SWIM

```
Cada 10 segundos el nodo elige un peer aleatorio y envía Ping UDP.
Si no responde en 5s → elige K=3 nodos aleatorios y envía PingReq.
Si ningún PingReq recibe Ack en 25s más → marca el nodo como SUSPECTED.
Si el nodo continúa sin responder 60s más → marca como OFFLINE.

Mensajes SWIM (bincode sobre UDP:7947):
  Ping     { sender: NodeId, seq: u64, piggybacked: NodeResources }
  Ack      { sender: NodeId, seq: u64, piggybacked: NodeResources }
  PingReq  { sender: NodeId, target: NodeId, seq: u64 }
  Suspect  { sender: NodeId, target: NodeId, incarnation: u64 }
  Alive    { sender: NodeId, target: NodeId, incarnation: u64 }
  Dead     { sender: NodeId, target: NodeId, incarnation: u64 }
```

### Estado compartido

```rust
// gossip/state.rs
pub struct ClusterState {
    pub nodes: HashMap<NodeId, NodeInfo>,
    pub version: u64,  // incrementado en cada cambio
}

// Acceso concurrente:
type SharedClusterState = Arc<RwLock<ClusterState>>;
```

### Canal de eventos de membresía

```rust
// gossip/mod.rs
pub enum MembershipEvent {
    NodeJoined(NodeInfo),
    NodeUpdated(NodeId, NodeResources),
    NodeSuspected(NodeId),
    NodeOffline(NodeId),
    NodeDraining(NodeId, DateTime<Utc>),
}

// Publicado via:
type MembershipBroadcast = tokio::broadcast::Sender<MembershipEvent>;
// Suscriptores: scheduler, storage
```

### Scrubbing de nodos caídos

Una tarea `tokio` independiente evalúa cada 5 segundos los nodos en estado
`SUSPECTED` y aplica las transiciones de tiempo:

```
SUSPECTED (30s sin respuesta directa ni indirecta)
    │
    └──► OFFLINE (60s adicionales sin respuesta)
              │
              └──► emite NodeOffline(node_id)
```

---

## Módulo `raft` (Fase 2+)

**Responsabilidad**: consenso distribuido para estado crítico.

- Solo activo si `quorum_participant = true` en `agent.toml`.
- Implementado sobre `openraft` (Apache 2.0).
- Los mensajes Raft viajan por gRPC via `RaftService` en `proto/raft.proto`.

### Log replicado

El log Raft contiene comandos tipados (`RaftCommand`). El estado resultante tras
aplicar todos los comandos del log es:

```
Estado replicado en Raft:
  ├── BlockMap:      HashMap<FileId, FileMetadata>  — mapa de chunks del clúster
  ├── JobRegistry:   HashMap<JobId, JobStatus>       — jobs activos y su estado
  ├── ClusterConfig: ClusterConfig                  — configuración oficial
  ├── TokenStore:    HashMap<Uuid, TokenRecord>     — tokens de enrolamiento
  └── CRL:           HashSet<NodeId>                — nodos revocados
```

### Interfaz pública

```rust
// raft/mod.rs
pub async fn apply_command(cmd: RaftCommand) -> Result<()>;
pub async fn read_committed() -> ClusterState;
pub fn is_leader() -> bool;
```

- `apply_command` bloquea hasta que el comando se replica en el quórum.
- Solo el nodo que lo llama necesita ser el líder; en caso contrario devuelve
  `Err(RaftError::NotLeader { leader_id: NodeId })` para que el llamador redirija.

---

## Módulo `scheduler`

**Responsabilidad**: recibir jobs y decidir en qué nodo ejecutarlos.

Ver [algoritmo de placement](scheduler.md) para la especificación completa.

```rust
// scheduler/mod.rs
pub async fn submit_job(spec: JobSpec) -> Result<JobStatus>;
pub async fn cancel_job(id: JobId) -> Result<JobStatus>;
pub async fn job_status(id: JobId) -> Result<JobStatus>;
```

- Suscrito a `MembershipEvent` via `tokio::broadcast::Receiver`.
- Mantiene `JobQueue` como `BTreeMap<Priority, VecDeque<PendingJob>>`
  protegida con `Arc<Mutex<>>`.

---

## Módulo `executor`

**Responsabilidad**: ejecutar jobs localmente y gestionar su ciclo de vida.

### Trait `Runtime`

```rust
// executor/mod.rs
#[async_trait]
pub trait Runtime: Send + Sync {
    async fn launch(
        job: &JobSpec,
        mounts: &[ResolvedMount],
    ) -> Result<ProcessHandle>;
    
    async fn kill(handle: &ProcessHandle) -> Result<()>;
}
```

Implementaciones: `DockerRuntime`, `JarRuntime`, `PythonRuntime`,
`ExecutableRuntime`, `WasmRuntime`.

### Ciclo de vida de un job

```
executor.launch(spec)
    │
    ├── 1. Resuelve DataMount[] (obtiene chunks del clúster si necesario)
    ├── 2. Aplica límites vía cgroups v2 (Linux) / equivalente por plataforma
    ├── 3. Lanza proceso según runtime
    ├── 4. Captura stdout/stderr (límite 10MB, truncado con aviso al log)
    └── 5. Publica JobEvent al canal interno → gossip → clúster
```

### Límites de recursos por plataforma

| Plataforma      | CPU         | Memoria     | Red         |
|-----------------|-------------|-------------|-------------|
| Linux           | cgroups v2  | cgroups v2  | tc/netns    |
| macOS           | `task_policy_set` | `setrlimit(RLIMIT_AS)` | Sin equivalente a `tc` — la red no se limita en macOS en v1 |
| Windows         | `JobObjectCpuRateControlInformation` | `JobObjectExtendedLimitInformation.ProcessMemoryLimit` | Sin equivalente a `tc` — la red no se limita en Windows en v1 |
| Android         | —           | —           | —           |

### Captura de output

```rust
// executor/mod.rs — límite de output
const MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024; // 10 MB

// Si se supera el límite:
// 1. Se para de capturar output adicional
// 2. Se añade la línea: "[OUTPUT TRUNCATED: exceeded 10MB limit]"
// 3. El proceso continúa ejecutándose
// 4. JobStatus.error incluye "output_truncated: true"
```

---

## Módulo `storage` (Fase 2+)

**Responsabilidad**: almacenar y servir chunks localmente.

```rust
// storage/mod.rs
pub async fn put_chunk(id: ChunkId, data: Bytes) -> Result<()>;
pub async fn get_chunk(id: ChunkId) -> Result<Bytes>;
pub async fn delete_chunk(id: ChunkId) -> Result<()>;
pub async fn list_local_chunks() -> Result<Vec<ChunkId>>;
```

- Verifica SHA-256 en `put_chunk` (rechaza si no coincide) y en `get_chunk`
  (devuelve error si el chunk está corrupto).
- Almacena chunks en `{storage_path}/chunks/{chunk_id}` (fichero plano por chunk).
- Mantiene índice local en `{storage_path}/index.db` usando `sled` (MIT).
- Suscrito a `MembershipEvent` para detectar nodos caídos y encolar re-replicación.
- **Scrubbing periódico** como tarea `tokio` independiente, throttled al 10%
  del ancho de banda de disco, ejecuta semanalmente.

---

## Módulo `lifecycle` (Fase 3+)

**Responsabilidad**: gestionar el tiering automático de datos.

- Solo activo en el **líder Raft**.
- Corre como tarea `tokio` cada **6 horas**.
- Ver [lifecycle engine](lifecycle.md) para la especificación completa del
  algoritmo de heat score y transiciones.

---

## Módulo `certificates` (Fase 2+)

**Responsabilidad**: gestionar la PKI interna del clúster.

```rust
// certificates/mod.rs
pub fn generate_ca() -> Result<CaBundle>;
pub fn generate_node_cert(ca: &CaBundle, node_id: NodeId) -> Result<NodeCert>;
pub fn sign_csr(ca: &CaBundle, csr: &[u8]) -> Result<Vec<u8>>;
pub fn is_revoked(node_id: NodeId) -> bool;
```

- Usa `rcgen` (MIT) para generación de claves Ed25519 y certificados X.509.
- Usa `rustls` (Apache 2.0) para configuración TLS en tonic.
- `is_revoked` consulta la CRL del estado Raft (no tiene estado propio).
- Ver [ADR-006](../decisions/006-pki-mtls.md) y [Fase 2](../phases/phase-2.md)
  para el flujo completo de enrolamiento.

---

## Módulo `api_rest`

**Responsabilidad**: servidor HTTP para clientes externos.

```rust
// api_rest/mod.rs
pub async fn start(
    config: Arc<Config>,
    scheduler: Arc<Scheduler>,
    storage: Arc<StorageModule>,    // Option en Fase 1
    cluster_state: SharedClusterState,
    raft: Option<Arc<RaftModule>>,  // None en Fase 1
) -> Result<()>
```

- Servidor `axum` (MIT) en el puerto **7946**.
- **No contiene lógica de negocio** — delega en los módulos correspondientes
  pasando `Arc<>` de cada módulo en el estado de axum.
- Gestiona serialización JSON y el formato estándar `ErrorResponse`.
- Fase 1: middleware de `shared_secret` si `security.mode = "dev"`.
- Fase 4: middleware de autenticación Bearer token.
- Todos los handlers añaden el header `X-Request-Id: {uuid}` en la respuesta.

---

## Módulo `grpc_server`

**Responsabilidad**: servidor gRPC para comunicación inter-agente.

```rust
// grpc_server/mod.rs
pub async fn start(
    config: Arc<Config>,
    scheduler: Arc<Scheduler>,
    executor: Arc<Executor>,
    storage: Arc<StorageModule>,
    raft: Option<Arc<RaftModule>>,
    certificates: Option<Arc<CertificatesModule>>,
) -> Result<()>
```

- Servidor `tonic` (MIT) en el puerto **7947** (TCP).
- Implementa `AgentService` y `RaftService` de los ficheros `.proto`.
- Fase 1: interceptor de `shared_secret` en metadata gRPC.
- Fase 2: `ServerTlsConfig` con mTLS configurado via `rustls`.

---

## Módulo `grpc_client`

**Responsabilidad**: pool de conexiones gRPC salientes hacia otros nodos.

```rust
// grpc_client/mod.rs
pub async fn launch_job(
    node: NodeId,
    req: LaunchJobRequest,
) -> Result<Streaming<JobEvent>>;

pub async fn transfer_chunk(
    node: NodeId,
    data: ChunkData,
) -> Result<()>;

pub async fn get_chunk(
    node: NodeId,
    id: ChunkId,
) -> Result<Bytes>;

pub async fn get_cluster_state(
    node: NodeId,
) -> Result<ClusterStateSnapshot>;

pub async fn join(
    node: NodeId,
    req: JoinRequest,
) -> Result<JoinResponse>;
```

- Cachea conexiones por `NodeId`. Reconecta automáticamente si la conexión se pierde.
- Fase 1: sin TLS.
- Fase 2: `ClientTlsConfig` con el certificado del nodo y la CA del clúster.
- Timeout de conexión: 5 segundos. Timeout de llamada LaunchJob: 10 segundos.
- Al recibir un `NodeOffline` del módulo gossip, invalida la conexión cacheada.

---

## Comunicación inter-módulo

```
                    tokio::broadcast::Sender<MembershipEvent>
gossip ──────────────────────────────────────────────────────►  scheduler
                                                             ►  storage

                    tokio::sync::mpsc::Sender<DiscoveredNode>
discovery.mdns ─────────────────────────────────────────────►  gossip
discovery.seeds ────────────────────────────────────────────►  gossip

                    tokio::sync::mpsc::Sender<JobEvent>
executor ───────────────────────────────────────────────────►  gossip (para propagación)

                    Arc<RwLock<ClusterState>>
gossip ──────────────────────────────────────────────────────►  scheduler (lectura)
                                                             ►  api_rest (lectura)

                    Arc<RaftModule> (Option)
scheduler ──────────────────────────────────────────────────►  raft.apply_command()
storage ────────────────────────────────────────────────────►  raft.apply_command()
lifecycle ──────────────────────────────────────────────────►  raft.apply_command()
```

**Regla estricta**: ningún módulo importa tipos internos de otro módulo.
Solo se importan los tipos públicos del crate `common` y los `Arc<Módulo>` recibidos
por inyección de dependencias en `main.rs`.
