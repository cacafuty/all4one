# Fase 2 — "Los datos viven en el clúster"

**Objetivo**: persistencia de datos distribuida, consenso Raft para estado
crítico, y comunicaciones cifradas con mTLS. Un dataset subido al clúster
sobrevive a la caída de un nodo.

---

## Módulos añadidos en Fase 2

| Módulo         | Estado   | Notas                                           |
|----------------|----------|-------------------------------------------------|
| `raft`         | Nuevo    | openraft, log replicado, líder/seguidor/candidato |
| `storage`      | Nuevo    | chunks, SHA-256, RS erasure coding, índice sled |
| `certificates` | Nuevo    | PKI interna, CA Ed25519, mTLS, CRL              |

## Módulos extendidos en Fase 2

| Módulo         | Extensión                                                    |
|----------------|--------------------------------------------------------------|
| `api_rest`     | Añade endpoints `/v1/storage/`, `/v1/admin/tokens`, `/v1/admin/nodes/{id}` |
| `grpc_server`  | Añade `RaftService`. Activa mTLS con `ServerTlsConfig`.      |
| `grpc_client`  | Activa mTLS con `ClientTlsConfig`.                          |
| `scheduler`    | `RegisterJob` via Raft antes de lanzar (deduplicación fuerte) |

---

## Alcance detallado

### Qué está incluido

**Raft (openraft)**:
- Elección de líder, replicación de log, snapshots.
- Log contiene: `BlockMap`, `JobRegistry`, `ClusterConfig`, `TokenStore`, `CRL`.
- `apply_command(RaftCommand)` garantiza escritura en quórum antes de responder.
- Solo nodos con `quorum_participant = true` participan.
- `GET /v1/cluster/status` expone `raft_leader` y `quorum_healthy`.

**Storage**:
- Chunking (64 MB default), SHA-256 por chunk.
- Erasure coding Reed-Solomon (RS(4,2) en Warm, RS(6,3) en Cold, RS(8,4) en Archive).
- Replicación 3x para Hot.
- Compresión Zstd (nivel 3 en Warm, nivel 19 en Cold, nivel 22 en Archive).
- Detección de tipo por magic bytes para saltar compresión en Hot.
- Placement con reglas de tier.
- Índice local en `sled`.
- Scrubbing semanal.
- Drenado anticipado con migración de chunks en riesgo.
- Cifrado en reposo opcional (AES-256-GCM con HKDF).

**S3 API completa**:
- `PUT`, `GET`, `HEAD`, `DELETE` `/v1/storage/{bucket}/{key}`
- `GET /v1/storage/{bucket}` (listado con paginación)
- Multipart upload completo.
- Headers `X-All4One-Policy`, `X-All4One-Min-Replicas`, `X-All4One-Access-Hint`.

**PKI interna y mTLS**:
- `all4one-agent init-cluster` genera CA Ed25519 + certificado autofirmado (TTL 10 años).
- `all4one-agent generate-token` genera token de enrolamiento (un solo uso, TTL 1 hora).
- `all4one-agent enroll --token TOKEN --endpoint IP:7947` enrola el nodo.
- Proceso de enrolamiento completo (CSR → firma → cert de 90 días → CA pública).
- Rate limiting en endpoint Join: 5 intentos/IP/hora.
- Renovación automática de certificados 7 días antes de expirar.
- `all4one-agent revoke --node NODE_ID` añade a la CRL en Raft.
- mTLS activo en todas las conexiones gRPC (salvo el endpoint `Join` que acepta sin cert de cliente).

### Qué NO está incluido en Fase 2

- Lifecycle engine (tiering automático) — Fase 3.
- FUSE / S3 API en puerto 9000 — Fase 4.
- Autenticación Bearer token real — Fase 4.
- Android — Fase 5.
- GPU — Fase 5.

---

## Flujo de enrolamiento completo

```bash
# En el primer nodo (ya existente), generar token:
all4one-agent generate-token
# OUTPUT: TOKEN=a3f8c2d1-b4e5-f6a7-b8c9-d0e1f2a3b4c5 (expira en 1 hora)

# En el nuevo nodo, ejecutar enroll:
all4one-agent enroll \
  --token a3f8c2d1-b4e5-f6a7-b8c9-d0e1f2a3b4c5 \
  --endpoint 192.168.1.100:7947

# El agente:
# 1. Genera Ed25519 keypair local → {data_dir}/certs/node.key (0600)
# 2. Genera CSR con node_id como CN
# 3. Llama gRPC Join al nodo indicado (sin mTLS — acepta sin cert de cliente)
# 4. Recibe node.crt (90 días) + ca.crt
# 5. Almacena certs en {data_dir}/certs/
# 6. Reinicia conexiones gRPC con mTLS
# 7. Pasa a ser miembro reconocido del clúster
```

---

## Estructura de carpetas Rust añadida en Fase 2

```
agent/src/
├── raft/
│   ├── mod.rs         # inicialización openraft, apply_command, read_committed
│   ├── store.rs       # implementa RaftStorage (sled como backend del log)
│   ├── network.rs     # implementa RaftNetwork via grpc_client
│   └── commands.rs    # enum RaftCommand y su aplicación al estado
├── storage/
│   ├── mod.rs         # put_chunk, get_chunk, delete_chunk, list_local_chunks
│   ├── chunks.rs      # SHA-256, compresión, erasure coding
│   ├── placement.rs   # reglas de placement de chunks
│   └── index.rs       # índice sled
├── certificates/
│   └── mod.rs         # generate_ca, generate_node_cert, sign_csr, is_revoked
├── api_rest/
│   └── storage.rs     # handlers PUT/GET/HEAD/DELETE /v1/storage/ y multipart
└── grpc_server/
    └── raft_service.rs # implementa RaftService (AppendEntries, RequestVote, InstallSnapshot)

proto/
└── raft.proto         # RaftService (añadido en Fase 2)
```

---

## Criterios de aceptación (Fase 2)

1. **Persistencia de datos**: subir un dataset de 1 GB via `PUT /v1/storage/`.
   Apagar el nodo que recibió la subida. El dataset sigue accesible via `GET`
   desde otro nodo.

2. **Quórum Raft**: con 3 nodos, apagar 1. El clúster sigue operativo
   (`quorum_healthy: true` en `GET /v1/cluster/status`). Apagar un segundo nodo.
   El clúster pasa a `quorum_healthy: false` y rechaza escrituras con `QUORUM_LOST`.

3. **Erasure coding**: subir un objeto en Warm (RS(4,2)). Eliminar manualmente
   los ficheros de 2 chunks en disco (en 2 nodos distintos). El objeto sigue
   siendo recuperable via `GET`.

4. **mTLS**: un nodo con un certificado de otra CA (o uno autofirmado fuera del
   clúster) no puede conectar al clúster — la conexión gRPC se rechaza sin
   procesar ningún mensaje.

5. **Revocación inmediata**: `all4one-agent revoke --node NODE_ID`. Dentro de
   un ciclo de gossip (< 15 segundos), ese nodo ya no puede hacer llamadas gRPC
   al clúster.

6. **Deduplicación de jobs**: enviar el mismo job (con `id` explícito) a dos
   schedulers simultáneamente. Solo se ejecuta una vez.

7. **Drenado**: `all4one-agent drain --in 30m` en un nodo con chunks almacenados.
   A los 25 minutos, los chunks en riesgo se han migrado a otros nodos.
   Al desconectarse, no quedan chunks huérfanos.

8. **RAM del agente en reposo (Fase 2)**: < **30 MB** con Raft activo y 0 jobs.

---

## Dependencias con Fase 1

Fase 2 se construye sobre todos los módulos de Fase 1. Los binarios de Fase 1
no son compatibles con un clúster Fase 2 que tenga Raft activo — todos los nodos
deben actualizarse antes de activar `quorum_participant = true`.

## Dependencias de crates adicionales (Fase 2)

```toml
openraft     = { version = "0.9", features = ["serde"] }
reed-solomon = "0.6"
zstd         = "0.13"
rcgen        = "0.13"
rustls       = "0.23"
rustls-pemfile = "2"
tokio-rustls = "0.26"
sled         = "0.34"
```

---

## Lista de tareas ordenadas

Cada tarea depende de las anteriores. Los tests verifican la tarea de forma
aislada antes de avanzar.

---

### Tarea 1 — Definir proto/raft.proto y RaftService en grpc_server

**Qué hacer**: añadir `proto/raft.proto` con `RaftService` (AppendEntries,
RequestVote, InstallSnapshot). Generar código con prost. Implementar el
esqueleto de `grpc_server/raft_service.rs` que delega en el módulo `raft`.

**Test**:
```bash
cargo build --workspace
# Sin errores. El código proto generado compila.

grpcurl -plaintext -H "x-all4one-secret: s" \
  localhost:7947 list
# Incluye: all4one.raft.v1.RaftService
```

---

### Tarea 2 — Módulo `raft`: store y network con openraft

**Qué hacer**: implementar `raft/store.rs` (openraft `RaftStorage` sobre sled)
y `raft/network.rs` (openraft `RaftNetwork` via `grpc_client`). Inicializar
el nodo Raft en `main.rs` solo si `quorum_participant = true`. El estado inicial
del log contiene un `ClusterConfig` vacío.

**Test**:
```bash
# Con 3 nodos quorum_participant=true en la misma red:
# Esperar a que se elija un líder (< 1 segundo tras conectar):
curl -s -H "X-All4One-Secret: s" http://nodo1:7946/v1/cluster/status | \
  python3 -c "import sys,json; d=json.load(sys.stdin); print(d['raft_leader'], d['quorum_healthy'])"
# <UUID del líder>  true

# Matar el líder — nuevo líder elegido en < 500ms:
pkill -f "config-lider.toml"
sleep 1
curl -s -H "X-All4One-Secret: s" http://nodo2:7946/v1/cluster/status | \
  python3 -c "import sys,json; d=json.load(sys.stdin); print(d['raft_leader'], d['quorum_healthy'])"
# <UUID diferente>  true
```

---

### Tarea 3 — RaftCommand: RegisterJob y deduplicación de jobs

**Qué hacer**: implementar `RaftCommand::RegisterJob` y `UpdateJobState` en
`raft/commands.rs`. El scheduler aplica `RegisterJob` antes de lanzar el job.
Si el `job_id` ya existe en el `JobRegistry`, devuelve el estado actual
(deduplicación fuerte).

**Test**:
```bash
# Enviar el mismo job (con id explícito) a dos schedulers simultáneamente:
JOB_YAML='id: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
runtime: docker
source: alpine:3.19
command: ["sleep", "10"]
resources: {cpu_cores: 1, memory_mb: 128}'

curl -s -X POST http://nodo1:7946/v1/jobs \
  -H "Content-Type: application/yaml" -H "X-All4One-Secret: s" \
  -d "$JOB_YAML" &
curl -s -X POST http://nodo2:7946/v1/jobs \
  -H "Content-Type: application/yaml" -H "X-All4One-Secret: s" \
  -d "$JOB_YAML" &
wait

sleep 5
# Solo debe existir UNA instancia del contenedor en el clúster:
# En cada nodo executor, verificar que el job corre exactamente una vez.
curl -s -H "X-All4One-Secret: s" http://nodo1:7946/v1/jobs | \
  python3 -c "import sys,json; jobs=json.load(sys.stdin)['jobs']; \
  ids=[j['job_id'] for j in jobs]; print(len(ids), len(set(ids)))"
# 1 1  (1 job, sin duplicados)
```

---

### Tarea 4 — Módulo `certificates`: generación de CA y certificados

**Qué hacer**: implementar `certificates/mod.rs` con `generate_ca()`,
`generate_node_cert()`, `sign_csr()`, `is_revoked()`. Implementar el comando
`all4one-agent init-cluster` que genera la CA y el token de bootstrap.

**Test**:
```bash
# Generar CA:
all4one-agent init-cluster --data-dir /var/lib/all4one
# Crea: /var/lib/all4one/certs/ca.key (permisos 0600)
#        /var/lib/all4one/certs/ca.crt
ls -la /var/lib/all4one/certs/
# ca.key: -rw------- (0600)
# ca.crt: -rw-r--r--

# Verificar que ca.crt es un certificado válido Ed25519:
openssl x509 -in /var/lib/all4one/certs/ca.crt -noout -text | grep -E "Issuer|Public Key"
# Public Key Algorithm: ED25519
```

---

### Tarea 5 — Enrolamiento: comando enroll y endpoint Join

**Qué hacer**: implementar `all4one-agent generate-token` (escribe token en Raft
via `RaftCommand::AddToken`). Implementar `all4one-agent enroll --token --endpoint`
que genera keypair + CSR, llama gRPC `Join`, almacena los certs. Implementar
el RPC `Join` en `grpc_server/agent_service.rs` con rate limiting y firma del CSR.

**Test**:
```bash
# Generar token en nodo existente:
TOKEN=$(all4one-agent generate-token --data-dir /var/lib/all4one | grep TOKEN | cut -d= -f2)

# Enrolar nuevo nodo:
all4one-agent enroll \
  --token $TOKEN \
  --endpoint 192.168.1.100:7947 \
  --data-dir /var/lib/all4one-nuevo
# Crea: node.key (0600), node.crt, ca.crt en /var/lib/all4one-nuevo/certs/

# Verificar que node.crt está firmado por la CA del clúster:
openssl verify -CAfile /var/lib/all4one/certs/ca.crt \
  /var/lib/all4one-nuevo/certs/node.crt
# /var/lib/all4one-nuevo/certs/node.crt: OK

# El token ya no es reutilizable (un solo uso):
all4one-agent enroll \
  --token $TOKEN \
  --endpoint 192.168.1.100:7947 \
  --data-dir /tmp/otro-nodo
# Error: token already consumed
```

---

### Tarea 6 — Activar mTLS en grpc_server y grpc_client

**Qué hacer**: en `mode=prod`, configurar `ServerTlsConfig` con `client_ca_root`
en `grpc_server`. Configurar `ClientTlsConfig` con el cert del nodo en
`grpc_client`. El endpoint `Join` acepta conexiones sin certificado de cliente
(client_auth_optional para ese RPC específico).

**Test**:
```bash
# Nodo sin cert válido NO puede conectar:
# (usando un certificado autofirmado diferente a la CA del clúster)
grpcurl -cert /tmp/fake.crt -key /tmp/fake.key -cacert /tmp/fake-ca.crt \
  192.168.1.100:7947 all4one.agent.v1.AgentService/GetClusterState
# Error: transport: authentication handshake failed

# Nodo con cert válido SÍ puede conectar:
grpcurl -cert /var/lib/all4one/certs/node.crt \
        -key  /var/lib/all4one/certs/node.key \
        -cacert /var/lib/all4one/certs/ca.crt \
  192.168.1.100:7947 all4one.agent.v1.AgentService/GetClusterState
# { "stateJson": "...", "version": "N" }
```

---

### Tarea 7 — Módulo `storage`: chunks, SHA-256 e índice sled

**Qué hacer**: implementar `storage/chunks.rs` con `put_chunk` (SHA-256,
escritura en disco) y `get_chunk` (verificación SHA-256). Implementar
`storage/index.rs` con el índice sled. Implementar `list_local_chunks`.

**Test**:
```bash
# Unit test — put y get de un chunk:
cargo test -p agent storage::chunks
# Debe incluir:
# - put_chunk con sha256 correcto → OK
# - put_chunk con sha256 incorrecto → Err(Sha256Mismatch)
# - get_chunk de chunk corrupto (modificar fichero en disco) → Err(ChunkCorrupted)
# - list_local_chunks devuelve el chunk recién almacenado
```

---

### Tarea 8 — Erasure coding y compresión por tier

**Qué hacer**: integrar `reed-solomon` en `storage/chunks.rs`. Implementar la
función `encode(data, tier) -> Vec<Shard>` y `decode(shards, tier) -> Bytes`.
Integrar `zstd` para compresión antes del erasure coding.

**Test**:
```bash
cargo test -p agent storage::erasure
# - encode RS(4,2) de 1MB → 6 shards, cada uno de ~170KB + overhead paridad
# - decode con los 4 shards de datos → datos originales ✓
# - decode eliminando 2 shards cualesquiera → datos originales recuperados ✓
# - decode eliminando 3 shards → Err(InsufficientShards) ✓
# - compress(data, Warm) → zstd nivel 3, descomprimible ✓
# - compress(data, Archive) → zstd nivel 22 ✓
```

---

### Tarea 9 — Placement de chunks y escritura distribuida

**Qué hacer**: implementar `storage/placement.rs` con las 4 reglas de placement.
Implementar `storage.put_object()` que chunkea el objeto, comprime, aplica
erasure coding, elige nodos con placement y transfiere via `grpc_client`.
Implementar `RaftCommand::PutChunkMap` y aplicarlo al completar la subida.

**Test**:
```bash
# Subir un fichero de 200 MB con política hot (3 réplicas):
dd if=/dev/urandom of=/tmp/test200mb.bin bs=1M count=200
curl -s -X PUT http://nodo1:7946/v1/storage/test/test200mb.bin \
  -H "Content-Type: application/octet-stream" \
  -H "X-All4One-Secret: s" \
  -H "X-All4One-Policy: auto" \
  --data-binary @/tmp/test200mb.bin
# { "bucket": "test", "key": "test200mb.bin", "size_bytes": 209715200,
#   "tier": "hot", "replicas": 3, ... }

# Verificar que los chunks están en al menos 2 nodos distintos:
curl -s -H "X-All4One-Secret: s" \
  http://nodo1:7946/v1/storage/test/test200mb.bin \
  -o /tmp/test200mb-descargado.bin
sha256sum /tmp/test200mb.bin /tmp/test200mb-descargado.bin
# Los dos hashes deben coincidir.
```

---

### Tarea 10 — GET con tolerancia a fallos y scrubbing

**Qué hacer**: implementar `storage.get_object()` que lee chunks en paralelo
(buffer de 4), verifica SHA-256, y si un chunk falla intenta el siguiente
placement. Implementar el scrubbing semanal como tarea tokio throttled.

**Test**:
```bash
# Subir objeto hot (3 réplicas).
# Corromper manualmente 1 chunk en disco en uno de los nodos:
CHUNK_PATH=$(ls /var/lib/all4one/storage/chunks/ | head -1)
echo "corrupto" >> /var/lib/all4one/storage/chunks/$CHUNK_PATH

# El objeto sigue siendo descargable (usa las otras 2 réplicas):
curl -s -H "X-All4One-Secret: s" \
  http://nodo1:7946/v1/storage/test/test200mb.bin \
  -o /tmp/recuperado.bin
sha256sum /tmp/test200mb.bin /tmp/recuperado.bin
# Hashes iguales — datos recuperados de réplica sana.
```

---

### Tarea 11 — Multipart upload

**Qué hacer**: implementar los 4 endpoints de multipart upload en `api_rest/storage.rs`.
Los parts se almacenan temporalmente en el nodo que recibe la subida y se
ensamblan al completar.

**Test**:
```bash
# Subir un fichero de 20 MB en 4 parts de 5 MB cada uno:
split -b 5m /tmp/test200mb.bin /tmp/part-
UPLOAD_ID=$(curl -s -X POST http://nodo1:7946/v1/storage/test/multipart-test/uploads \
  -H "X-All4One-Secret: s" | python3 -c "import sys,json; print(json.load(sys.stdin)['upload_id'])")

ETAG1=$(curl -s -X PUT \
  http://nodo1:7946/v1/storage/test/multipart-test/uploads/$UPLOAD_ID/parts/1 \
  -H "Content-Type: application/octet-stream" -H "X-All4One-Secret: s" \
  --data-binary @/tmp/part-aa | python3 -c "import sys,json; print(json.load(sys.stdin)['etag'])")
# Repetir para parts 2, 3, 4...

curl -s -X POST \
  http://nodo1:7946/v1/storage/test/multipart-test/uploads/$UPLOAD_ID/complete \
  -H "Content-Type: application/json" -H "X-All4One-Secret: s" \
  -d "{\"parts\": [{\"part_number\": 1, \"etag\": \"$ETAG1\"}, ...]}"
# { "bucket": "test", "key": "multipart-test", ... }
```

---

### Tarea 12 — Renovación automática de certificados y CRL

**Qué hacer**: implementar en `certificates/mod.rs` la tarea tokio que verifica
la expiración 7 días antes y solicita renovación al líder Raft. Implementar
`all4one-agent revoke --node NODE_ID` que aplica `RaftCommand::AddToCRL`.
Verificar la CRL en cada accept gRPC.

**Test**:
```bash
# Revocar un nodo:
all4one-agent revoke --node <NODE_ID> --endpoint 192.168.1.100:7947

# En < 15 segundos (un ciclo de gossip), el nodo revocado
# no puede hacer llamadas gRPC:
grpcurl -cert /var/lib/all4one-revocado/certs/node.crt \
        -key  /var/lib/all4one-revocado/certs/node.key \
        -cacert /var/lib/all4one-revocado/certs/ca.crt \
  192.168.1.100:7947 all4one.agent.v1.AgentService/GetClusterState
# Error: transport: authentication handshake failed (nodo en CRL)
```

---

### Tarea 13 — Drenado anticipado con migración de chunks

**Qué hacer**: implementar `all4one-agent drain --in Xm`. Al recibir `DrainRequest`
gRPC, el módulo storage identifica chunks en riesgo y encola `MigrationJob` con
prioridad High. A T-10min el nodo rechaza nuevas escrituras.

**Test**:
```bash
# Con objeto almacenado cuyo único chunk está en el nodo a drenar:
all4one-agent drain --in 5m --data-dir /var/lib/all4one-B

# A los 4 minutos (T-1min), los chunks deben haber migrado a otro nodo:
sleep 240
# Apagar nodo B abruptamente:
pkill -f "config-b.toml"

# El objeto sigue accesible desde nodo A:
curl -s -H "X-All4One-Secret: s" \
  http://nodoA:7946/v1/storage/test/test200mb.bin \
  -o /tmp/post-drain.bin
sha256sum /tmp/test200mb.bin /tmp/post-drain.bin
# Hashes iguales.
```

---

### Tarea 14 — Prueba de integración final (Fase 2)

**Test**: ejecutar los 8 criterios de aceptación de la fase.
```bash
# 1. Persistencia al caer el nodo que recibió la subida ✓
# 2. Quórum: 1/3 nodos caídos → operativo; 2/3 caídos → QUORUM_LOST ✓
# 3. Erasure RS(4,2): 2 chunks eliminados → objeto recuperable ✓
# 4. mTLS: cert de otra CA → conexión rechazada en handshake ✓
# 5. Revocación: nodo revocado pierde acceso en < 15s ✓
# 6. Deduplicación: mismo job_id enviado dos veces → ejecutado una vez ✓
# 7. Drenado: chunks migrados antes de T-0 ✓
# 8. RAM en reposo con Raft activo < 30 MB ✓
```
