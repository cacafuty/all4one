# Almacenamiento distribuido

El módulo storage gestiona el almacenamiento de datos como objetos divididos en
chunks distribuidos entre los nodos con `roles.storage = true`. Activo desde Fase 2.

---

## Modelo de datos

```
Objeto lógico (FileMetadata en Raft)
  bucket: "datasets"
  key:    "training/imagenet-2024.tar"
  size:   150 GB
  chunks: [ChunkId-0, ChunkId-1, ..., ChunkId-2399]  ← 2400 chunks de 64MB
                │
                ▼
Chunk físico (ChunkMetadata en Raft + fichero en disco)
  id:              ChunkId (UUID v4)
  file_id:         FileId del objeto padre
  index:           0..2399
  size_bytes:      67108864  (64 MB, salvo el último)
  sha256:          [u8; 32]
  placements:      [NodeId-A, NodeId-B, NodeId-C]
  tier:            Hot
  erasure_scheme:  Replicated3x
```

---

## Chunking

```
Tamaño default:   64 MB  (configurable por bucket en agent.toml)
Tamaño mínimo:    1 MB
Tamaño máximo:    512 MB
Último chunk:     puede ser menor que el tamaño configurado
```

---

## Tiers de datos y esquemas de protección

| Tier    | Esquema      | Descripción                              | Compresión      |
|---------|--------------|------------------------------------------|-----------------|
| Hot     | Replicated3x | 3 réplicas completas, sin erasure coding | Ninguna o LZ4   |
| Warm    | RS(4,2)      | 4 datos + 2 paridad, tolera 2 fallos     | Zstd nivel 3    |
| Cold    | RS(6,3)      | 6 datos + 3 paridad, tolera 3 fallos     | Zstd nivel 19   |
| Archive | RS(8,4)      | 8 datos + 4 paridad, tolera 4 fallos     | Zstd nivel 22   |

**Por qué Replicated3x en Hot y no RS**: con replicación 3x cualquier réplica
sirve la lectura completa — latencia mínima, sin overhead de decodificación RS.
Con RS(4,2) se necesitan 4 de 6 fragmentos para reconstruir, añadiendo latencia
de coordinación. El coste en almacenamiento (3x vs. 1.5x) se acepta para datos calientes.

---

## Compresión

La compresión se aplica **antes** del erasure coding:

```
Escritura:
  datos_raw → compresión → erasure coding → chunks → disco en nodos

Lectura:
  chunks desde disco → decodificación erasure → descompresión → datos_raw
```

**Detección de tipo para Hot**: se leen los primeros 512 bytes (magic bytes) para
detectar formatos ya comprimidos (JPEG, PNG, MP4, ZIP, GZIP, ZSTD, LZ4, BZIP2, XZ).
Si el objeto ya está comprimido, se omite LZ4 y se almacena sin comprimir.

**Índice de contenido en Archive**: se almacena un índice separado en
`{bucket}/{key}/.index` con la lista de ficheros internos y sus offsets, para
listar el contenido de TAR/ZIP sin descomprimir ni restaurar desde Archive.

---

## Placement de chunks

Reglas aplicadas en orden de prioridad:

```
Regla 1: nunca todas las réplicas/fragmentos en nodos del mismo tier.
  Replicated3x → preferencia Tier0+Tier1+Tier1, o Tier0+Tier0+Tier1.
  Nunca Tier1+Tier1+Tier1 si hay nodos Tier0 disponibles.
  RS(4,2) → los 6 fragmentos distribuidos entre al menos 2 tiers.

Regla 2: al menos una réplica en un nodo con quorum_participant=true.

Regla 3: preferir nodos con mayor ventana de disponibilidad restante.

Regla 4: consistent hashing como distribución base.
  hash(chunk_id) mod num_nodos → nodo primario.
  Nodos secundarios: siguientes en el anillo de consistent hashing.
  Minimiza redistribución cuando nodos entran/salen.
```

---

## Escritura de un objeto (PUT)

```
Cliente → PUT /v1/storage/datasets/training/imagenet.tar
               │
               ▼
api_rest recibe el stream de bytes
               │
               ▼
storage.put_object(bucket, key, stream, policy)
               │
    Para cada chunk de 64MB:
    ├── 1. Calcula SHA-256
    ├── 2. Compresión según tier
    ├── 3. Erasure coding (Reed-Solomon)
    ├── 4. Placement algorithm → lista de nodos destino
    ├── 5. grpc_client.transfer_chunk() a cada nodo destino
    └── 6. Genera ChunkMetadata
               │
               ▼
Raft.apply(PutChunkMap { file_id, metadata: FileMetadata })
  → replicado en quórum antes de responder al cliente
               │
               ▼
200 OK { bucket, key, size_bytes, etag, tier, replicas, created_at }
```

El `etag` es el SHA-256 hexadecimal del objeto completo (antes de chunking).

---

## Lectura de un objeto (GET)

```
Cliente → GET /v1/storage/datasets/training/imagenet.tar
               │
               ▼
Consulta FileMetadata en Raft → lista de ChunkIds ordenados por index
               │
               ▼
Para cada chunk (en paralelo, buffer de 4 chunks):
  1. ChunkMetadata → placements: [NodeId-A, NodeId-B, NodeId-C]
  2. Si algún NodeId == self → lee localmente (prioridad)
  3. Si no → grpc_client.get_chunk(NodeId-A)
  4. Verifica SHA-256
  5. Si corrupción → intenta siguiente placement
  6. Decodifica erasure coding (si aplica)
  7. Descomprime (si aplica)
               │
               ▼
Streaming de bytes al cliente a medida que llegan chunks
```

Si el objeto está en **Archive**: devuelve `202 Accepted` con `restore_job_id`.
Ver [lifecycle engine](lifecycle.md) para el flujo de restauración asíncrona.

---

## Integridad y scrubbing

### Verificación en tiempo real

```rust
// put_chunk — rechaza si el SHA-256 no coincide
pub async fn put_chunk(id: ChunkId, data: Bytes, expected_sha256: [u8; 32]) -> Result<()> {
    let computed = sha256::digest(&data);
    if computed != expected_sha256 {
        return Err(StorageError::Sha256Mismatch { chunk_id: id });
    }
    let path = format!("{}/chunks/{}", self.storage_path, id);
    tokio::fs::write(&path, &data).await?;
    self.index.insert(id, ChunkIndexEntry {
        sha256: expected_sha256,
        size_bytes: data.len() as u64,
    })?;
    Ok(())
}

// get_chunk — devuelve error si el chunk está corrupto en disco
pub async fn get_chunk(id: ChunkId) -> Result<Bytes> {
    let entry = self.index.get(id)?
        .ok_or(StorageError::ChunkNotFound { chunk_id: id })?;
    let path = format!("{}/chunks/{}", self.storage_path, id);
    let data = tokio::fs::read(&path).await?;
    let computed = sha256::digest(&data);
    if computed != entry.sha256 {
        return Err(StorageError::ChunkCorrupted {
            chunk_id: id,
            node_id: self.node_id,
        });
    }
    Ok(Bytes::from(data))
}
```

### Scrubbing periódico

Tarea `tokio` semanal, throttled al 10% del ancho de banda de disco:

```
Para cada chunk local:
  1. Lee el chunk del disco
  2. Calcula SHA-256
  3. Compara con el índice local
  4. Si no coincide → chunk corrupto:
     a. Incrementa contador de fallos del nodo
     b. Si 3 fallos en ventana de 1 hora:
          → gossip.suspect(self.node_id)
          → re-replicación de TODOS sus chunks a otros nodos
     c. Solicita copia válida del clúster y reemplaza el chunk corrupto
```

---

## Drenado anticipado

```
T-30min: DrainNotice propagado via gossip a todos los nodos
              │
T-25min: storage identifica chunks en riesgo:
         → chunks cuya única copia online está en este nodo
         → chunks que violarían reglas de placement sin este nodo
         Encola MigrationJobs con prioridad High en el scheduler
              │
T-10min: nodo deja de aceptar put_chunk remoto (devuelve 503)
              │
T-0min:  nodo se desconecta
         storage en otros nodos verifica que migración completó
         Si quedan chunks sin migrar → re-replicación de emergencia
```

---

## Multipart upload

Para objetos > 100 MB se recomienda; para > 5 GB se requiere:

```bash
# 1. Iniciar
curl -X POST http://node:7946/v1/storage/datasets/bigfile.tar/uploads \
  -H "X-All4One-Secret: mysecret"
# → { "upload_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479" }

# 2. Subir cada part (mínimo 5 MB salvo el último, máximo 10.000 parts)
curl -X PUT \
  http://node:7946/v1/storage/datasets/bigfile.tar/uploads/f47ac10b-58cc-4372-a567-0e02b2c3d479/parts/1 \
  -H "Content-Type: application/octet-stream" \
  --data-binary @part1.bin
# → { "part_number": 1, "etag": "a3f8c2d1b4e5f6a7b8c9d0e1f2a3b4c5" }

# 3. Completar
curl -X POST \
  http://node:7946/v1/storage/datasets/bigfile.tar/uploads/f47ac10b-58cc-4372-a567-0e02b2c3d479/complete \
  -H "Content-Type: application/json" \
  -d '{"parts": [{"part_number": 1, "etag": "a3f8c2d1b4e5f6a7b8c9d0e1f2a3b4c5"}]}'

# Cancelar si se abandona
curl -X DELETE \
  http://node:7946/v1/storage/datasets/bigfile.tar/uploads/f47ac10b-58cc-4372-a567-0e02b2c3d479
```

**Decisión pendiente**: TTL para multipart uploads abandonados — cuántas horas
sin actividad antes de limpiar automáticamente los parts temporales.

---

## Índice local (sled)

```
{storage_path}/
├── chunks/
│   ├── f47ac10b-58cc-4372-a567-0e02b2c3d479   ← fichero plano por chunk
│   └── ...
└── index.db                                    ← sled database

Esquema sled:
  Clave:  chunk_id como bytes del UUID (16 bytes)
  Valor:  ChunkIndexEntry serializado con bincode:
            sha256:     [u8; 32]
            size_bytes: u64
            path:       String
```

La fuente de verdad del BlockMap global es Raft. El índice local es una
vista optimizada de los chunks que residen en este nodo específico.

**Decisión pendiente**: evaluar subdirectorios por los primeros 2 caracteres del
UUID si el número de chunks por nodo supera habitualmente 100.000 ficheros.

---

## Cifrado en reposo (opcional)

Si `storage.encryption = true` en `agent.toml`:

```
Escritura:  datos_comprimidos → AES-256-GCM(clave_nodo) → disco
Lectura:    disco → AES-256-GCM decrypt(clave_nodo) → datos_comprimidos

Derivación de clave (en memoria al arrancar):
  clave_nodo = HKDF-SHA256(
      ikm  = bytes(node.key privada),
      salt = bytes(cluster_id),
      info = b"all4one-chunk-encryption-v1"
  )
```

**Decisión pendiente**: procedimiento de re-cifrado de chunks al renovar
el certificado del nodo (la clave deriva del certificado privado actual).
