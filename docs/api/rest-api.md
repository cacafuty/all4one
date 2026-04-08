# API REST — Especificación completa

**Base URL**: `http(s)://{nodo}:7946`
**Formato de respuesta**: `application/json` salvo donde se indique.
**Cabecera de correlación**: todos los endpoints devuelven `X-Request-Id: {uuid}`.

## Autenticación

| Fase | Mecanismo                                            |
|------|------------------------------------------------------|
| 1    | `X-All4One-Secret: {shared_secret}` si `mode=dev`   |
| 4+   | `Authorization: Bearer {api_key}`                    |

Si la autenticación falla: `401 Unauthorized` con body `ErrorResponse`.

## Formato de errores

Todos los errores usan este schema:

```json
{
  "code": "JOB_NOT_FOUND",
  "message": "Job 'a1b2c3d4-...' does not exist",
  "details": null,
  "request_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479"
}
```

Códigos de error posibles: `JOB_NOT_FOUND`, `JOB_ALREADY_FINISHED`,
`NODE_NOT_FOUND`, `NO_NODES_AVAILABLE`, `INVALID_JOB_SPEC`, `QUORUM_LOST`,
`OBJECT_NOT_FOUND`, `OBJECT_TOO_LARGE`, `INVALID_BUCKET_OR_KEY`,
`NETWORK_REQUIREMENTS_NOT_MET`, `UNAUTHORIZED`.

---

## Jobs

### POST /v1/jobs

Envía un job al scheduler. Si el `id` ya existe, devuelve el estado actual
sin relanzar.

**Request**

```
POST /v1/jobs HTTP/1.1
Content-Type: application/yaml
X-All4One-Secret: mysecret

runtime: docker
source: "python:3.11-slim"
command: ["python", "-c", "print('hello')"]
resources:
  cpu_cores: 1
  memory_mb: 256
```

**Responses**

`202 Accepted` — job creado, en cola o ya scheduled:
```json
{
  "job_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "status": "scheduled",
  "assigned_to": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
  "created_at": "2026-04-08T10:30:00Z",
  "started_at": null,
  "finished_at": null,
  "exit_code": null,
  "attempts": 0,
  "error": null
}
```

`200 OK` — el `id` ya existía, devuelve estado actual sin relanzar.
Body: mismo schema que `202`.

`400 Bad Request`:
```json
{
  "code": "INVALID_JOB_SPEC",
  "message": "Field 'resources.cpu_cores' must be >= 1",
  "details": { "field": "resources.cpu_cores", "value": 0 },
  "request_id": "b2c3d4e5-f6a7-8901-bcde-f01234567890"
}
```

`422 Unprocessable Entity`:
```json
{
  "code": "NO_NODES_AVAILABLE",
  "message": "No nodes satisfy constraints: tier_min=1, python_min=3.11, memory_mb=8192",
  "details": { "online_nodes": 2, "candidates_after_filter": 0 },
  "request_id": "c3d4e5f6-a7b8-9012-cdef-012345678901"
}
```

`503 Service Unavailable`:
```json
{
  "code": "QUORUM_LOST",
  "message": "Raft quorum is not available. Retry when more nodes are online.",
  "details": { "quorum_nodes": 3, "online_quorum_nodes": 1 },
  "request_id": "d4e5f6a7-b8c9-0123-defa-123456789012"
}
```

---

### GET /v1/jobs/{job_id}

**Request**
```
GET /v1/jobs/a1b2c3d4-e5f6-7890-abcd-ef1234567890 HTTP/1.1
X-All4One-Secret: mysecret
```

**Responses**

`200 OK`:
```json
{
  "job_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "status": "completed",
  "assigned_to": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
  "created_at": "2026-04-08T10:30:00Z",
  "started_at": "2026-04-08T10:30:02Z",
  "finished_at": "2026-04-08T10:31:15Z",
  "exit_code": 0,
  "attempts": 1,
  "error": null
}
```

`404 Not Found`:
```json
{
  "code": "JOB_NOT_FOUND",
  "message": "Job 'a1b2c3d4-e5f6-7890-abcd-ef1234567890' does not exist",
  "details": null,
  "request_id": "e5f6a7b8-c9d0-1234-efab-234567890123"
}
```

---

### DELETE /v1/jobs/{job_id}

Cancela un job. Si está corriendo: SIGTERM, espera 30s, luego SIGKILL.

**Request**
```
DELETE /v1/jobs/a1b2c3d4-e5f6-7890-abcd-ef1234567890 HTTP/1.1
X-All4One-Secret: mysecret
```

**Responses**

`200 OK`:
```json
{
  "job_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "status": "cancelled",
  "assigned_to": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
  "created_at": "2026-04-08T10:30:00Z",
  "started_at": "2026-04-08T10:30:02Z",
  "finished_at": "2026-04-08T10:32:05Z",
  "exit_code": null,
  "attempts": 1,
  "error": "cancelled by user"
}
```

`404 Not Found`: ErrorResponse `JOB_NOT_FOUND`

`409 Conflict`:
```json
{
  "code": "JOB_ALREADY_FINISHED",
  "message": "Job 'a1b2c3d4-...' is already in state 'completed'",
  "details": { "current_status": "completed" },
  "request_id": "f6a7b8c9-d0e1-2345-fabc-345678901234"
}
```

---

### GET /v1/jobs/{job_id}/output

Devuelve el output capturado del job (hasta 10 MB). Si está truncado, `truncated: true`.

**Request**
```
GET /v1/jobs/a1b2c3d4-e5f6-7890-abcd-ef1234567890/output HTTP/1.1
X-All4One-Secret: mysecret
```

**Responses**

`200 OK`:
```json
{
  "job_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "stdout": "Processing batch 1/100\nProcessing batch 2/100\n...",
  "stderr": "",
  "truncated": false
}
```

`404 Not Found`: ErrorResponse `JOB_NOT_FOUND`

---

### GET /v1/jobs/{job_id}/output/stream

Stream en tiempo real del output del job via Server-Sent Events.
Si el job ya terminó, emite todos los eventos históricos y cierra la conexión.

**Request**
```
GET /v1/jobs/a1b2c3d4-e5f6-7890-abcd-ef1234567890/output/stream HTTP/1.1
Accept: text/event-stream
X-All4One-Secret: mysecret
```

**Response** `200 OK` (`Content-Type: text/event-stream`):
```
event: stdout
data: {"line": "Processing batch 1/100"}

event: stdout
data: {"line": "Processing batch 2/100"}

event: stderr
data: {"line": "Warning: low memory"}

event: completed
data: {"exit_code": 0}
```

Si el job falla:
```
event: failed
data: {"error": "OOMKilled: container exceeded memory limit"}
```

`404 Not Found`: ErrorResponse `JOB_NOT_FOUND`

---

### GET /v1/jobs

Lista jobs con filtros opcionales.

**Request**
```
GET /v1/jobs?status=running&limit=20&offset=0 HTTP/1.1
X-All4One-Secret: mysecret
```

**Query params**:
- `status`: `queued|scheduled|running|completed|failed|cancelled` (opcional)
- `node_id`: UUID del nodo asignado (opcional)
- `limit`: default `20`, máximo `100`
- `offset`: default `0`

**Response** `200 OK`:
```json
{
  "jobs": [
    {
      "job_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
      "status": "running",
      "assigned_to": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
      "created_at": "2026-04-08T10:30:00Z",
      "started_at": "2026-04-08T10:30:02Z",
      "finished_at": null,
      "exit_code": null,
      "attempts": 1,
      "error": null
    }
  ],
  "total": 1,
  "limit": 20,
  "offset": 0
}
```

---

## Nodos

### GET /v1/nodes

**Response** `200 OK`:
```json
{
  "nodes": [
    {
      "profile": {
        "id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
        "tier": 0,
        "availability": "always",
        "quorum_participant": true,
        "resources": {
          "cpu_cores_total": 8,
          "cpu_cores_available": 6,
          "ram_mb_total": 32768,
          "ram_mb_available": 28000,
          "disk_gb_total": 500,
          "disk_gb_available": 380,
          "network_bandwidth_mbps": 1000
        },
        "capabilities": {
          "docker": true,
          "java": "21.0.1",
          "python": "3.11.2",
          "wasm": true,
          "executables": {
            "linux_x86_64": true,
            "linux_arm64": false,
            "windows_x86_64": false,
            "macos_arm64": false,
            "macos_x86_64": false,
            "android_arm64": false
          },
          "gpu": null
        },
        "platform": "linux_x86_64",
        "reliability_score": 0.97
      },
      "status": "online",
      "last_seen": "2026-04-08T10:35:00Z",
      "drain_at": null,
      "address": "192.168.1.100:7947",
      "rest_address": "192.168.1.100:7946",
      "jobs_running": ["a1b2c3d4-e5f6-7890-abcd-ef1234567890"]
    }
  ],
  "total": 1,
  "online": 1,
  "offline": 0
}
```

---

### GET /v1/nodes/{node_id}

**Response** `200 OK`: mismo schema que un elemento de `nodes[]` en `GET /v1/nodes`.

`404 Not Found`: ErrorResponse `NODE_NOT_FOUND`

---

## Clúster

### GET /v1/cluster/status

**Request**
```
GET /v1/cluster/status HTTP/1.1
X-All4One-Secret: mysecret
```

**Response** `200 OK`:
```json
{
  "cluster_id": "9e107d9d-372b-4844-b3e3-e5e5e5e5e5e5",
  "nodes_total": 4,
  "nodes_online": 3,
  "quorum_nodes": 3,
  "quorum_healthy": true,
  "raft_leader": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
  "jobs_queued": 0,
  "jobs_running": 2,
  "storage_total_gb": 1500,
  "storage_used_gb": 420,
  "storage_available_gb": 1080
}
```

---

## Seguridad (Fase 2+)

### POST /v1/admin/tokens

Genera un token de enrolamiento de un solo uso, válido 1 hora.
Requiere autenticación de administrador.

**Request**
```
POST /v1/admin/tokens HTTP/1.1
Authorization: Bearer admin-api-key-here
```

**Response** `200 OK`:
```json
{
  "token": "a3f8c2d1-b4e5-f6a7-b8c9-d0e1f2a3b4c5",
  "expires_at": "2026-04-08T11:30:00Z"
}
```

---

### DELETE /v1/admin/nodes/{node_id}

Revoca un nodo — lo añade a la CRL replicada en Raft. Acceso inmediato revocado.

**Request**
```
DELETE /v1/admin/nodes/b2c3d4e5-f6a7-8901-bcde-f01234567890 HTTP/1.1
Authorization: Bearer admin-api-key-here
```

**Response** `200 OK`:
```json
{
  "node_id": "b2c3d4e5-f6a7-8901-bcde-f01234567890",
  "revoked": true
}
```

---

## Storage (Fase 2+)

### PUT /v1/storage/{bucket}/{key}

Sube un objeto al clúster. Para objetos > 100 MB usar multipart.

**Request**
```
PUT /v1/storage/datasets/training/imagenet-sample.tar HTTP/1.1
Content-Type: application/octet-stream
X-All4One-Secret: mysecret
X-All4One-Policy: auto
X-All4One-Min-Replicas: 2
X-All4One-Access-Hint: frequent

<bytes del objeto>
```

**Headers opcionales de request**:
- `X-All4One-Policy`: `auto | manual | tiered`, default: `auto`
- `X-All4One-Min-Replicas`: número mínimo de réplicas, default según política
- `X-All4One-Access-Hint`: `frequent | normal | read_once | archive_immediately`

**Response** `200 OK`:
```json
{
  "bucket": "datasets",
  "key": "training/imagenet-sample.tar",
  "size_bytes": 1073741824,
  "etag": "a3f8c2d1b4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1",
  "tier": "hot",
  "replicas": 3,
  "created_at": "2026-04-08T10:30:00Z"
}
```

`400 Bad Request`: ErrorResponse `INVALID_BUCKET_OR_KEY`
`413 Payload Too Large`: ErrorResponse `OBJECT_TOO_LARGE`

---

### GET /v1/storage/{bucket}/{key}

**Request**
```
GET /v1/storage/datasets/training/imagenet-sample.tar HTTP/1.1
X-All4One-Secret: mysecret
```

**Response** `200 OK`: bytes del objeto (`Content-Type: application/octet-stream`)

**Response** `202 Accepted` (objeto en Archive, restauración iniciada):
```json
{
  "status": "restoring",
  "restore_job_id": "c3d4e5f6-a7b8-9012-cdef-012345678901",
  "estimated_minutes": 45
}
```

`404 Not Found`: ErrorResponse `OBJECT_NOT_FOUND`

---

### HEAD /v1/storage/{bucket}/{key}

**Response** `200 OK` (sin body). Headers:
```
X-All4One-Size: 1073741824
X-All4One-ETag: a3f8c2d1b4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1
X-All4One-Tier: hot
X-All4One-Replicas: 3
X-All4One-Heat-Score: 0.823
X-All4One-Last-Access: 2026-04-08T09:15:00Z
X-All4One-Policy: auto
```

`404 Not Found`: ErrorResponse `OBJECT_NOT_FOUND`

---

### DELETE /v1/storage/{bucket}/{key}

**Response** `200 OK`:
```json
{}
```

`404 Not Found`: ErrorResponse `OBJECT_NOT_FOUND`

---

### GET /v1/storage/{bucket}

Lista objetos en un bucket.

**Query params**:
- `prefix`: filtro de prefijo (opcional)
- `limit`: default `50`, máximo `1000`
- `continuation_token`: para paginación (opcional)

**Response** `200 OK`:
```json
{
  "bucket": "datasets",
  "prefix": "training/",
  "objects": [
    {
      "key": "training/imagenet-sample.tar",
      "size_bytes": 1073741824,
      "tier": "hot",
      "last_modified": "2026-04-08T10:30:00Z"
    },
    {
      "key": "training/cifar10.tar",
      "size_bytes": 170498073,
      "tier": "warm",
      "last_modified": "2026-03-01T08:00:00Z"
    }
  ],
  "truncated": false,
  "continuation_token": null
}
```

---

### Multipart upload

#### POST /v1/storage/{bucket}/{key}/uploads

**Response** `200 OK`:
```json
{ "upload_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479" }
```

#### PUT /v1/storage/{bucket}/{key}/uploads/{upload_id}/parts/{part_number}

Mínimo 5 MB por part salvo el último. Máximo 10.000 parts.

**Response** `200 OK`:
```json
{ "part_number": 1, "etag": "a3f8c2d1b4e5f6a7b8c9d0e1f2a3b4c5" }
```

#### POST /v1/storage/{bucket}/{key}/uploads/{upload_id}/complete

**Request body**:
```json
{
  "parts": [
    { "part_number": 1, "etag": "a3f8c2d1b4e5f6a7b8c9d0e1f2a3b4c5" },
    { "part_number": 2, "etag": "b4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9" }
  ]
}
```

**Response** `200 OK`: mismo schema que `PUT /v1/storage/{bucket}/{key}`.

#### DELETE /v1/storage/{bucket}/{key}/uploads/{upload_id}

Cancela el upload y elimina los parts temporales.

**Response** `200 OK`: `{}`

---

### GET /v1/storage/restore/{restore_job_id}

Consulta el estado de una restauración desde Archive.

**Response** `202 Accepted` (en progreso):
```json
{
  "restore_job_id": "c3d4e5f6-a7b8-9012-cdef-012345678901",
  "status": "restoring",
  "progress": 0.65,
  "estimated_minutes_remaining": 16
}
```

**Response** `200 OK` (listo para descarga):
```json
{
  "restore_job_id": "c3d4e5f6-a7b8-9012-cdef-012345678901",
  "status": "ready",
  "download_url": "http://192.168.1.100:7946/v1/storage/datasets/old-archive.tar",
  "expires_at": "2026-04-09T10:30:00Z"
}
```

---

## Inferencia IA (Fase 5)

### GET /v1/models

**Response** `200 OK`:
```json
{
  "models": [
    {
      "id": "llama3-8b-q4_k_m",
      "name": "Llama 3 8B Q4_K_M",
      "size_gb": 4.5,
      "tier": "hot",
      "nodes_with_cache": 2
    },
    {
      "id": "llama3-70b-q4_k_m",
      "name": "Llama 3 70B Q4_K_M",
      "size_gb": 35.0,
      "tier": "hot",
      "nodes_with_cache": 0
    }
  ]
}
```

### POST /v1/chat/completions

Body y response compatibles con OpenAI ChatCompletion API.

**Request**
```json
{
  "model": "llama3-8b-q4_k_m",
  "messages": [
    { "role": "system", "content": "Eres un asistente técnico." },
    { "role": "user", "content": "¿Qué es el protocolo SWIM?" }
  ],
  "temperature": 0.7,
  "stream": false
}
```

**Response** `200 OK`:
```json
{
  "id": "chatcmpl-abc123",
  "object": "chat.completion",
  "created": 1712569800,
  "model": "llama3-8b-q4_k_m",
  "choices": [
    {
      "index": 0,
      "message": {
        "role": "assistant",
        "content": "SWIM (Scalable Weakly-consistent Infection-style Membership) es un protocolo..."
      },
      "finish_reason": "stop"
    }
  ],
  "usage": {
    "prompt_tokens": 42,
    "completion_tokens": 150,
    "total_tokens": 192
  }
}
```

**Response** `202 Accepted` (modelo cargándose):
```json
{ "status": "loading", "estimated_seconds": 30 }
```

---

## Métricas y salud

### GET /metrics

`200 OK` — Prometheus text exposition format. Ver [networking.md](../architecture/networking.md).

### GET /health

**Response** `200 OK`:
```json
{
  "status": "ok",
  "node_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
  "uptime_seconds": 86400,
  "cluster_connected": true,
  "quorum_healthy": true
}
```

**Response** `503 Service Unavailable`:
```json
{
  "status": "degraded",
  "node_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
  "issues": [
    "Raft quorum lost: 1/3 quorum nodes online",
    "Storage scrubbing found 2 corrupted chunks"
  ]
}
```
