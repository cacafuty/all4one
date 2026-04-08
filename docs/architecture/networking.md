# Protocolos de red

El agente usa **tres canales de comunicación distintos** que conviven en la misma
máquina: la API REST para clientes externos, gRPC para la comunicación interna
entre agentes, y UDP para el protocolo SWIM de membresía.

---

## Mapa de puertos

```
                   ┌──────────────────────────────────────────┐
                   │              AGENTE all4one               │
                   │                                          │
Clientes externos  │  7946 TCP ──► api_rest (axum)            │
(CLI, SDK, boto3)  │                                          │
                   │  7947 TCP ──► grpc_server (tonic)        │◄── otros agentes
Otros agentes  ────│  7947 UDP ──► gossip SWIM                │◄── otros agentes
                   │                                          │
Prometheus     ────│  9090 TCP ──► /metrics                   │
                   │                                          │
boto3 / AWS CLI────│  9000 TCP ──► S3-compatible API (Fase 4) │
                   └──────────────────────────────────────────┘
```

| Puerto | Protocolo | Dirección  | Descripción                                    |
|--------|-----------|------------|------------------------------------------------|
| 7946   | TCP/HTTP  | Entrada    | API REST para clientes externos                |
| 7947   | TCP/HTTP2 | Bidireccional | gRPC entre agentes y mensajes Raft          |
| 7947   | UDP       | Bidireccional | SWIM gossip (mismo puerto, protocolo distinto) |
| 9000   | TCP/HTTP  | Entrada    | S3-compatible API (Fase 4)                     |
| 9090   | TCP/HTTP  | Entrada    | Prometheus metrics (scraping externo)          |

> El puerto 7947 comparte número entre TCP (gRPC) y UDP (SWIM). Son protocolos
> distintos gestionados por sockets distintos — el kernel los discrimina por
> el tipo de socket.

---

## API REST (puerto 7946)

**Stack**: `axum` (MIT) sobre `tokio`.

**Propósito**: interfaz para clientes externos — CLI, SDK Rust, SDK Java, boto3,
navegador web. No es el canal de comunicación entre agentes.

**Seguridad por fase**:

```
Fase 1 (mode=dev):
  Header requerido en cada request: X-All4One-Secret: <shared_secret>
  Si falta o no coincide → 401 Unauthorized

Fase 4 (mode=prod):
  Header requerido: Authorization: Bearer <api_key>
  Las api_keys se gestionan en Raft y tienen ámbitos (scopes)
```

**Formato de respuesta**: `application/json` salvo `GET /v1/storage/{bucket}/{key}`
(que devuelve `application/octet-stream`) y `GET /metrics` (Prometheus format).

**Cabecera de correlación**: todos los endpoints devuelven
`X-Request-Id: <uuid>` para correlación en logs.

**Aceptación de body**: `POST /v1/jobs` acepta tanto `application/yaml`
como `application/json`.

Ver [especificación completa de la API REST](../api/rest-api.md).

---

## gRPC (puerto 7947 TCP)

**Stack**: `tonic` (MIT) + `prost` (Apache 2.0) + Protocol Buffers 3.

**Propósito**: comunicación interna entre agentes. Nunca expuesto directamente
a clientes externos.

**Por qué gRPC y no REST entre agentes**: ver [ADR-005](../decisions/005-grpc-internal.md).

### Definición de servicios

#### `proto/agent.proto`

```protobuf
syntax = "proto3";
package all4one.agent.v1;

import "google/protobuf/timestamp.proto";
import "google/protobuf/empty.proto";

service AgentService {
  // El scheduler envía un job al nodo executor.
  // El stream devuelve JobEvents en tiempo real.
  rpc LaunchJob(LaunchJobRequest) returns (stream JobEvent);

  // Transferencia streaming de un chunk entre nodos storage.
  rpc TransferChunk(stream ChunkData) returns (TransferChunkResponse);

  // Solicitud de un chunk. Streaming para chunks grandes.
  rpc GetChunk(GetChunkRequest) returns (stream ChunkData);

  // Snapshot del estado del clúster para nodos recién incorporados.
  rpc GetClusterState(google.protobuf.Empty) returns (ClusterStateSnapshot);

  // Solicitud de enrolamiento. Único RPC sin mTLS en Fase 2.
  rpc Join(JoinRequest) returns (JoinResponse);

  // Notificación de drenado anticipado.
  rpc Drain(DrainRequest) returns (google.protobuf.Empty);
}

message LaunchJobRequest {
  bytes job_spec_json = 1;  // JobSpec serializado en JSON
}

message JobEvent {
  string job_id = 1;
  oneof event {
    JobStarted    started    = 2;
    JobOutputLine output     = 3;
    JobCheckpoint checkpoint = 4;
    JobCompleted  completed  = 5;
    JobFailed     failed     = 6;
    JobLost       lost       = 7;
  }
}

message JobStarted    { string node_id = 1; }
message JobOutputLine { string stream = 1; string line = 2; } // stream: "stdout"|"stderr"
message JobCheckpoint { string path = 1; }
message JobCompleted  { int32 exit_code = 1; }
message JobFailed     { string error = 1; }
message JobLost       {}

message ChunkData {
  string chunk_id  = 1;
  uint32 index     = 2;   // índice del fragmento en streaming
  bytes  data      = 3;
  bool   last      = 4;   // true en el último mensaje del stream
}

message TransferChunkResponse {
  string chunk_id = 1;
  bool   ok       = 2;
}

message GetChunkRequest {
  string chunk_id = 1;
}

message ClusterStateSnapshot {
  bytes  state_json = 1;  // ClusterState serializado en JSON
  uint64 version    = 2;
}

message JoinRequest {
  string node_id     = 1;
  bytes  csr_pem     = 2;   // Certificate Signing Request PEM
  string token       = 3;   // token de enrolamiento de un solo uso
  bytes  profile_json = 4;  // NodeProfile serializado en JSON
}

message JoinResponse {
  bytes  node_cert_pem = 1;  // certificado firmado PEM
  bytes  ca_cert_pem   = 2;  // certificado público de la CA PEM
  bytes  state_json    = 3;  // ClusterStateSnapshot inicial
}

message DrainRequest {
  string node_id    = 1;
  int64  drain_at   = 2;  // Unix timestamp UTC de desconexión
}
```

#### `proto/raft.proto`

```protobuf
syntax = "proto3";
package all4one.raft.v1;

// Implementación del protocolo Raft estándar (openraft genera
// las estructuras AppendEntriesRequest, etc. internamente).
// Este servicio expone los tres RPCs del protocolo Raft.

service RaftService {
  rpc AppendEntries(AppendEntriesRequest) returns (AppendEntriesResponse);
  rpc RequestVote(RequestVoteRequest) returns (RequestVoteResponse);
  rpc InstallSnapshot(stream SnapshotChunk) returns (InstallSnapshotResponse);
}

// Los mensajes AppendEntriesRequest/Response, RequestVoteRequest/Response
// son los definidos por openraft y se serializan con prost.
// Ver openraft::raft::AppendEntriesRequest<TypeConfig> en la documentación
// de openraft para el schema exacto de cada campo.

message AppendEntriesRequest  { bytes payload = 1; }
message AppendEntriesResponse { bytes payload = 1; }
message RequestVoteRequest    { bytes payload = 1; }
message RequestVoteResponse   { bytes payload = 1; }

message SnapshotChunk {
  bytes  data  = 1;
  bool   last  = 2;
  uint64 index = 3;
}

message InstallSnapshotResponse { bytes payload = 1; }
```

### Seguridad gRPC por fase

```
Fase 1 (mode=dev):
  Interceptor de metadata en cada RPC:
    metadata["x-all4one-secret"] debe coincidir con shared_secret
    Si falta o no coincide → tonic::Status::unauthenticated("invalid secret")
  Sin TLS — las conexiones van en texto plano

Fase 2 (mode=prod):
  ServerTlsConfig con:
    identity: (node.crt, node.key)
    client_ca_root: ca.crt
    client_auth_optional: true  ← solo para el RPC Join
  
  ClientTlsConfig con:
    identity: (node.crt, node.key)
    ca_certificate: ca.crt
  
  En cada handshake mTLS:
    1. Servidor verifica que el cliente tiene cert firmado por la CA del clúster
    2. Cliente verifica que el servidor tiene cert firmado por la CA del clúster
    3. Si alguna verificación falla → conexión rechazada antes de procesar el RPC
    4. Adicionalmente: certificates::is_revoked(peer_node_id) consultado en cada accept
```

### Timeouts gRPC

| Operación              | Timeout      |
|------------------------|--------------|
| Conexión TCP           | 5 segundos   |
| `LaunchJob` (respuesta inicial) | 10 segundos |
| `LaunchJob` (stream completo) | sin límite + `max_duration_minutes` del job |
| `TransferChunk` (64MB) | 60 segundos  |
| `GetChunk` (64MB)      | 60 segundos  |
| `GetClusterState`      | 5 segundos   |
| `Join`                 | 15 segundos  |

---

## SWIM Gossip (puerto 7947 UDP)

**Protocolo**: SWIM (Scalable Weakly-consistent Infection-style Membership).
**Implementación base**: `chitchat` (Apache 2.0), adaptada para incluir
piggybacked state de recursos.
**Serialización**: `bincode` (MIT) — formato binario compacto, sin overhead de texto.

### Mensajes

```rust
// Formato de los mensajes SWIM serializados con bincode
#[derive(Serialize, Deserialize)]
pub enum SwimMessage {
    Ping {
        sender: NodeId,
        seq: u64,
        piggybacked: NodeResources,  // recursos actuales del emisor
    },
    Ack {
        sender: NodeId,
        seq: u64,
        piggybacked: NodeResources,
    },
    PingReq {
        sender: NodeId,
        target: NodeId,
        seq: u64,
    },
    Suspect {
        sender: NodeId,
        target: NodeId,
        incarnation: u64,
    },
    Alive {
        sender: NodeId,
        target: NodeId,
        incarnation: u64,
    },
    Dead {
        sender: NodeId,
        target: NodeId,
        incarnation: u64,
    },
}
```

### Ciclo de detección de fallos

```
Cada 10 segundos por nodo activo:

  1. Elige un peer aleatorio P de la lista de miembros conocidos
  2. Envía Ping UDP a P
  3. Espera Ack de P durante 5 segundos
  4. Si no recibe Ack:
     a. Elige K=3 nodos aleatorios Q1, Q2, Q3
     b. Envía PingReq(target=P) a cada Qi
     c. Espera que algún Qi responda con Ack en 25 segundos adicionales
  5. Si ningún PingReq produce Ack para P:
     → Emite Suspect(target=P) via gossip
     → P pasa a estado SUSPECTED en ClusterState
  6. Si P sigue en SUSPECTED durante 60 segundos adicionales sin responder:
     → Emite Dead(target=P)
     → P pasa a estado OFFLINE en ClusterState
     → Se publica MembershipEvent::NodeOffline(P.id)

Cuando un nodo recibe Suspect sobre sí mismo:
  → Incrementa su incarnation number
  → Propaga Alive(self, nueva_incarnation) para contradecir la sospecha
```

### Piggybacking de recursos

Cada mensaje `Ping` y `Ack` incluye los recursos actuales del nodo emisor:

```rust
pub struct NodeResources {
    pub cpu_cores_total: u32,
    pub cpu_cores_available: u32,
    pub ram_mb_total: u64,
    pub ram_mb_available: u64,
    pub disk_gb_total: u64,
    pub disk_gb_available: u64,
    pub network_bandwidth_mbps: u32,
}
```

Esto propaga el estado de recursos por el clúster sin mensajes adicionales.
El scheduler usa `cpu_cores_available` y `ram_mb_available` del `ClusterState`
actualizado por estos piggybacked updates.

### Tiempos de convergencia

Con N nodos y heartbeat de 10 segundos, el protocolo SWIM garantiza que
cualquier cambio de membresía converge a todos los nodos en `O(log N)` ciclos.

| Nodos | Ciclos hasta convergencia | Tiempo aprox. |
|-------|--------------------------|---------------|
| 10    | ~3 ciclos                | ~30 segundos  |
| 50    | ~4 ciclos                | ~40 segundos  |
| 100   | ~5 ciclos                | ~50 segundos  |
| 500   | ~6 ciclos                | ~60 segundos  |

---

## mDNS (descubrimiento en red local)

**Librería**: `mdns-sd` (MIT).
**Servicio anunciado**: `_all4one._tcp.local`

### Registros TXT anunciados

```
_all4one._tcp.local TXT:
  node_id=f47ac10b-58cc-4372-a567-0e02b2c3d479
  tier=0
  port_grpc=7947
  port_rest=7946
  version=0.1.0
```

### Comportamiento al descubrir un nodo

```
1. El módulo discovery.mdns recibe el anuncio mDNS de un nodo nuevo
2. Si el node_id ya está en ClusterState.nodes → ignorar
3. Si no está:
   a. Enviar DiscoveredNode { address, node_id, tier } al módulo gossip
      via tokio::sync::mpsc
   b. gossip intenta gRPC GetClusterState al nuevo nodo para obtener
      su NodeProfile completo
   c. Si GetClusterState tiene éxito → NodeJoined al broadcast
```

---

## Flujo de enrolamiento (Fase 2+)

```
Nuevo nodo                    Nodo existente del clúster (líder Raft)
    │                                     │
    │  all4one-agent enroll               │
    │  --token TOKEN --endpoint IP:7947   │
    │                                     │
    │──[gRPC Join sin mTLS]──────────────►│
    │  JoinRequest {                      │  1. Verifica rate limit (5/IP/hora)
    │    node_id,                         │  2. Verifica TOKEN en Raft TokenStore
    │    csr_pem,                         │  3. Invalida TOKEN en Raft
    │    token,                           │  4. Firma CSR con CA privada
    │    profile_json                     │  5. Devuelve cert + CA + state
    │  }                                  │
    │◄──────────────────────────────────[JoinResponse]
    │                                     │
    │  Almacena:                          │
    │    node.key → {data_dir}/certs/     │
    │    node.crt → {data_dir}/certs/     │
    │    ca.crt   → {data_dir}/certs/     │
    │                                     │
    │  Reinicia conexiones gRPC con mTLS  │
    │──[gRPC con mTLS]───────────────────►│
    │  (ya es miembro reconocido)         │
```

---

## S3-compatible API (puerto 9000, Fase 4)

El módulo `api_rest` expone en el puerto 9000 los mismos endpoints de storage
que en el puerto 7946 bajo `/v1/storage/`, pero con headers de autenticación
AWS S3 estándar (AWS Signature Version 4).

Esto hace el sistema compatible con:
- `boto3` (Python)
- `aws s3` CLI
- `s3cmd`
- Cualquier librería con soporte S3

La implementación reutiliza exactamente la misma lógica de storage — solo
el parsing de autenticación y el mapeo de rutas difiere entre los dos puertos.

**Decisión pendiente**: definir el mapeo exacto entre buckets S3 y los buckets
de All4One para el caso de rutas virtuales vs. path-style en la S3 API.

---

## Prometheus metrics (puerto 9090)

El endpoint `GET /metrics` en el puerto 9090 (o en el 7946 — configurable)
expone métricas en formato Prometheus text exposition format.

```
# HELP all4one_node_cpu_used_cores CPU cores currently in use by running jobs
# TYPE all4one_node_cpu_used_cores gauge
all4one_node_cpu_used_cores{node_id="f47ac10b"} 2.5

# HELP all4one_node_ram_used_mb RAM in MB currently used by running jobs
# TYPE all4one_node_ram_used_mb gauge
all4one_node_ram_used_mb{node_id="f47ac10b"} 1024

# HELP all4one_node_disk_used_gb Disk space in GB used by stored chunks
# TYPE all4one_node_disk_used_gb gauge
all4one_node_disk_used_gb{node_id="f47ac10b"} 45.2

# HELP all4one_node_jobs_running Number of jobs currently running on this node
# TYPE all4one_node_jobs_running gauge
all4one_node_jobs_running{node_id="f47ac10b"} 3

# HELP all4one_node_jobs_completed_total Total jobs completed successfully
# TYPE all4one_node_jobs_completed_total counter
all4one_node_jobs_completed_total{node_id="f47ac10b"} 142

# HELP all4one_node_jobs_failed_total Total jobs that failed
# TYPE all4one_node_jobs_failed_total counter
all4one_node_jobs_failed_total{node_id="f47ac10b"} 7

# HELP all4one_node_chunks_stored Number of chunks stored locally
# TYPE all4one_node_chunks_stored gauge
all4one_node_chunks_stored{node_id="f47ac10b"} 891

# HELP all4one_node_chunks_served_bytes_total Total bytes served from local chunks
# TYPE all4one_node_chunks_served_bytes_total counter
all4one_node_chunks_served_bytes_total{node_id="f47ac10b"} 9847562134

# HELP all4one_node_uptime_seconds Seconds since agent started
# TYPE all4one_node_uptime_seconds counter
all4one_node_uptime_seconds{node_id="f47ac10b"} 86400

# HELP all4one_cluster_nodes_online Number of nodes currently online
# TYPE all4one_cluster_nodes_online gauge
all4one_cluster_nodes_online 4

# HELP all4one_cluster_jobs_queued Number of jobs waiting for a node
# TYPE all4one_cluster_jobs_queued gauge
all4one_cluster_jobs_queued 0

# HELP all4one_cluster_storage_used_gb Total GB used across all storage nodes
# TYPE all4one_cluster_storage_used_gb gauge
all4one_cluster_storage_used_gb 180.5
```

La recolección de métricas del clúster (`all4one_cluster_*`) se hace agregando
el `ClusterState` local — no requiere llamadas adicionales a otros nodos.
