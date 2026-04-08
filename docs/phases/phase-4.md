# Fase 4 — "Transparencia total ante procesos"

**Objetivo**: los procesos existentes (scripts Python con boto3, aplicaciones Java,
binarios nativos) acceden al clúster sin modificación de código, usando sus
interfaces habituales: FUSE como sistema de ficheros, S3 API, SDK nativos,
o LD_PRELOAD para interposición transparente.

---

## Módulos añadidos en Fase 4

| Módulo / Componente | Descripción                                                    |
|---------------------|----------------------------------------------------------------|
| FUSE driver         | Monta el clúster como sistema de ficheros local                |
| LD_PRELOAD shim     | Intercepta llamadas libc en Linux sin modificar binarios       |
| SDK Rust            | Crate `all4one-sdk` publicada en crates.io                     |
| SDK Java            | Librería Maven con autoconfiguración Spring Boot               |
| S3 API (puerto 9000)| Compatible con boto3, aws CLI, s3cmd                           |
| Auth Bearer token   | API keys almacenadas en Raft, middleware axum                  |

---

## Alcance detallado

### FUSE

**Librería**: `fuser` (MIT) en Linux y macOS. `WinFsp` (LGPL, enlace dinámico) en Windows.

**Montaje**:
```bash
# Linux / macOS
all4one-agent mount --bucket datasets --mountpoint /mnt/datasets

# Windows
all4one-agent mount --bucket datasets --mountpoint Z:
```

**Operaciones implementadas en v1**:

| Operación POSIX | Implementación                                               |
|-----------------|--------------------------------------------------------------|
| `lookup`        | Consulta `FileMetadata` en Raft por bucket+key               |
| `getattr`       | Devuelve tamaño, timestamps desde `FileMetadata`             |
| `readdir`       | Lista objetos del bucket con prefijo del directorio          |
| `open`          | Registra handle de fichero, inicia prefetch de chunks        |
| `read`          | Descarga chunks necesarios para el rango solicitado          |
| `write`         | Bufferiza en memoria, sube al clúster en `release`           |
| `create`        | Crea entrada en `FileMetadata`, buffer vacío                 |
| `unlink`        | `DELETE /v1/storage/{bucket}/{key}` via módulo storage       |
| `rename`        | Copia metadata en Raft, elimina entrada anterior             |
| `mkdir`         | Crea prefijo virtual (solo en metadata, no hay directorios reales) |
| `rmdir`         | Elimina todos los objetos con ese prefijo                    |

**Append y seek**: soportados via reescritura del chunk afectado.
Un append al final del fichero reescribe el último chunk con los bytes adicionales.

**POSIX locks distribuidos**: no implementados en v1.
**Decisión pendiente**: especificar el protocolo de distributed locking para v2
(candidatos: Raft-based advisory locks, o lease-based locks con TTL).

**WinFsp LGPL**: enlace dinámico. Verificar con asesor legal antes de distribución
en entornos donde la LGPL sea problemática.

### LD_PRELOAD shim (solo Linux)

El shim intercepta llamadas de libc mediante `LD_PRELOAD`:

```bash
LD_PRELOAD=/usr/lib/all4one/libvolatile.so \
VOLATILE_INTERCEPT_PATH=/data/model \
VOLATILE_TARGET=volatile://models/llama3-70b.gguf \
VOLATILE_ENDPOINT=192.168.1.100:7946 \
  ./mi-programa-existente
```

**Funciones interceptadas**: `open`, `openat`, `read`, `write`, `close`, `stat`,
`fstat`, `lstat`, `lseek`, `mmap` (solo lectura secuencial, no mmap completo).

**Limitación**: solo funciona con binarios enlazados dinámicamente a glibc.
Binarios estáticos o con musl no son compatibles.

**Variables de entorno**:
- `VOLATILE_INTERCEPT_PATH`: path local a interceptar (ej. `/data/model`)
- `VOLATILE_TARGET`: URI en el clúster (ej. `volatile://models/llama3.gguf`)
- `VOLATILE_ENDPOINT`: IP:puerto del nodo REST (ej. `192.168.1.100:7946`)
- `VOLATILE_SECRET`: shared secret o API key para autenticación

### SDK Rust

Crate `all4one-sdk` publicada en crates.io.

```toml
# Cargo.toml
[dependencies]
all4one-sdk = "0.1"
```

```rust
use all4one_sdk::{All4OneClient, ClientConfig, JobSpec};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = All4OneClient::new(ClientConfig {
        endpoint: "http://192.168.1.100:7946".to_string(),
        api_key: "my-api-key".to_string(),
        timeout_secs: 30,
    });

    // Subir datos
    let data = std::fs::read("dataset.tar")?;
    let resp = client.put("datasets", "training/dataset.tar", data.into(), Default::default()).await?;
    println!("Subido: etag={}", resp.etag);

    // Enviar job
    let spec: JobSpec = serde_yaml::from_str(include_str!("job.yaml"))?;
    let status = client.submit_job(spec).await?;
    println!("Job: {}", status.job_id);

    // Streaming de output
    let mut stream = client.job_logs(status.job_id).await?;
    while let Some(event) = stream.next().await {
        println!("{:?}", event);
    }

    Ok(())
}
```

**API completa del SDK Rust**:
```rust
impl All4OneClient {
    pub async fn put(bucket, key, data: Bytes, options: PutOptions) -> Result<PutResponse>;
    pub async fn get(bucket, key) -> Result<Bytes>;
    pub async fn head(bucket, key) -> Result<ObjectMetadata>;
    pub async fn delete(bucket, key) -> Result<()>;
    pub async fn list(bucket, prefix, options: ListOptions) -> Result<ListResponse>;
    pub async fn submit_job(spec: JobSpec) -> Result<JobStatus>;
    pub async fn job_status(id: JobId) -> Result<JobStatus>;
    pub async fn cancel_job(id: JobId) -> Result<JobStatus>;
    pub async fn job_logs(id: JobId) -> Result<impl Stream<Item = JobEvent>>;
}
```

### SDK Java

Artefacto Maven: `io.all4one:all4one-sdk:0.1.0`

```xml
<dependency>
    <groupId>io.all4one</groupId>
    <artifactId>all4one-sdk</artifactId>
    <version>0.1.0</version>
</dependency>
```

```java
import io.all4one.All4OneClient;
import io.all4one.model.JobSpec;
import io.all4one.model.JobStatus;

All4OneClient client = All4OneClient.builder()
    .endpoint("http://192.168.1.100:7946")
    .apiKey("my-api-key")
    .build();

// Subir fichero
client.put("datasets", "training/data.tar", Files.readAllBytes(path))
      .thenAccept(resp -> System.out.println("ETag: " + resp.getEtag()));

// Enviar job
JobSpec spec = JobSpec.fromYaml(new File("job.yaml"));
CompletableFuture<JobStatus> future = client.submitJob(spec);
```

**Autoconfiguración Spring Boot**:
```java
// Añadir @EnableAll4One a la clase de configuración
@Configuration
@EnableAll4One
public class AppConfig {}
```

```properties
# application.properties
all4one.endpoint=http://192.168.1.100:7946
all4one.api-key=my-api-key
all4one.timeout-seconds=30
```

Spring Boot inyecta automáticamente `All4OneClient` como bean.

### S3 API compatible (puerto 9000)

El agente expone en el puerto 9000 una API compatible con AWS S3 (Signature V4).
Los mismos endpoints que `/v1/storage/` pero con el protocolo S3 estándar.

```python
import boto3

s3 = boto3.client(
    "s3",
    endpoint_url="http://192.168.1.100:9000",
    aws_access_key_id="all4one-key-id",
    aws_secret_access_key="all4one-secret",
    region_name="all4one-local",
)

# Funciona con la API S3 estándar
s3.upload_file("dataset.tar", "datasets", "training/dataset.tar")
response = s3.get_object(Bucket="datasets", Key="training/dataset.tar")
```

**Decisión pendiente**: mapeo exacto entre buckets S3 y buckets de All4One para
el caso de rutas virtuales vs. path-style hosting en la S3 API.

### Autenticación Bearer token

En Fase 4, `security.mode = "prod"` activa autenticación Bearer token para
clientes externos (la autenticación entre agentes sigue siendo mTLS).

```bash
# Crear API key (requiere autenticación de admin)
curl -X POST http://node:7946/v1/admin/api-keys \
  -H "Authorization: Bearer admin-master-key" \
  -d '{"name": "ci-pipeline", "scopes": ["jobs:write", "storage:read"]}'
# → { "key_id": "ak_...", "secret": "sk_...", "scopes": [...] }

# Usar API key
curl http://node:7946/v1/jobs \
  -H "Authorization: Bearer sk_..."
```

**Decisión pendiente**: especificación completa del sistema de scopes (qué
operaciones permiten cada scope, cómo se almacenan en Raft, TTL de las keys).

---

## Estructura de carpetas Rust añadida en Fase 4

```
agent/src/
├── fuse/
│   ├── mod.rs          # inicializa fuser, monta el filesystem
│   ├── fs.rs           # implementa el trait fuser::Filesystem
│   └── cache.rs        # caché de chunks recientes para reducir latencia
├── s3_api/
│   ├── mod.rs          # servidor axum en puerto 9000
│   ├── auth.rs         # AWS Signature V4 verification
│   └── handlers.rs     # mapeo S3 → módulo storage
└── api_rest/
    └── auth.rs         # middleware Bearer token

sdk/                    # crate separado
├── Cargo.toml
└── src/
    └── lib.rs          # All4OneClient, ClientConfig, PutOptions, ...
```

---

## Criterios de aceptación (Fase 4)

1. **FUSE**: montar un bucket en `/mnt/test`. Un script Python que usa
   `open("/mnt/test/data.csv", "r")` lee el fichero del clúster sin ninguna
   modificación de código.

2. **boto3**: script Python estándar que llama `s3.upload_file()` y
   `s3.download_file()` al puerto 9000 funciona sin modificaciones.

3. **LD_PRELOAD**: un binario nativo existente con `VOLATILE_INTERCEPT_PATH`
   configurado lee y escribe en el clúster transparentemente.

4. **SDK Java + Spring Boot**: aplicación Spring Boot con `@EnableAll4One`
   puede subir y descargar objetos inyectando `All4OneClient`.

5. **Auth Bearer**: request sin `Authorization` header devuelve `401 Unauthorized`.
   Request con key revocada devuelve `401 Unauthorized`.

---

## Dependencias con fases anteriores

Requiere Fase 2 (storage distribuido, mTLS) y Fase 3 (lifecycle engine opcional
pero recomendado para gestionar modelos que usa FUSE).

---

## Lista de tareas ordenadas

---

### Tarea 1 — Autenticación Bearer token (API keys en Raft)

**Qué hacer**: añadir `RaftCommand::CreateApiKey` y `RevokeApiKey`. Implementar
`POST /v1/admin/api-keys` y el middleware axum que valida el `Authorization: Bearer`
header en todos los endpoints de `/v1/` cuando `security.mode = "prod"`.

**Test**:
```bash
# Crear API key de admin:
KEY=$(curl -s -X POST http://nodo1:7946/v1/admin/api-keys \
  -H "Authorization: Bearer master-admin-key" \
  -d '{"name": "test-key", "scopes": ["jobs:write", "storage:read"]}' | \
  python3 -c "import sys,json; print(json.load(sys.stdin)['secret'])")

# Usar la key:
curl -s -o /dev/null -w "%{http_code}" \
  -H "Authorization: Bearer $KEY" \
  http://nodo1:7946/v1/jobs
# 200

# Sin header → 401:
curl -s -o /dev/null -w "%{http_code}" http://nodo1:7946/v1/jobs
# 401

# Key revocada → 401:
curl -s -X DELETE http://nodo1:7946/v1/admin/api-keys/$KEY_ID \
  -H "Authorization: Bearer master-admin-key"
curl -s -o /dev/null -w "%{http_code}" \
  -H "Authorization: Bearer $KEY" http://nodo1:7946/v1/jobs
# 401
```

---

### Tarea 2 — SDK Rust: crate all4one-sdk

**Qué hacer**: implementar el crate `sdk/` con `All4OneClient`. Métodos: `put`,
`get`, `head`, `delete`, `list`, `submit_job`, `job_status`, `cancel_job`,
`job_logs`. Publicar en crates.io (o al menos compilar y pasar tests).

**Test**:
```rust
// tests/integration_test.rs en el crate sdk
#[tokio::test]
async fn test_put_get_delete() {
    let client = All4OneClient::new(ClientConfig {
        endpoint: "http://localhost:7946".to_string(),
        api_key: std::env::var("ALL4ONE_API_KEY").unwrap(),
        timeout_secs: 30,
    });

    let data = Bytes::from("hello world");
    let resp = client.put("test", "sdk-test.txt", data.clone(), Default::default()).await.unwrap();
    assert_eq!(resp.size_bytes, 11);

    let downloaded = client.get("test", "sdk-test.txt").await.unwrap();
    assert_eq!(downloaded, data);

    client.delete("test", "sdk-test.txt").await.unwrap();

    let result = client.get("test", "sdk-test.txt").await;
    assert!(result.is_err()); // OBJECT_NOT_FOUND
}
```

---

### Tarea 3 — S3-compatible API en puerto 9000

**Qué hacer**: implementar `s3_api/` como servidor axum separado en el puerto
9000. Implementar AWS Signature V4 verification en `s3_api/auth.rs`. Mapear
las rutas S3 a los métodos del módulo storage.

**Test**:
```bash
# Con boto3 estándar apuntando al puerto 9000:
python3 << 'EOF'
import boto3, hashlib

s3 = boto3.client("s3",
    endpoint_url="http://localhost:9000",
    aws_access_key_id="all4one-key",
    aws_secret_access_key="all4one-secret",
    region_name="us-east-1")

# Upload
s3.put_object(Bucket="test", Key="boto3-test.txt", Body=b"hello from boto3")

# Download y verificar
obj = s3.get_object(Bucket="test", Key="boto3-test.txt")
content = obj["Body"].read()
assert content == b"hello from boto3", f"Got: {content}"

# List
resp = s3.list_objects_v2(Bucket="test", Prefix="boto3")
assert len(resp["Contents"]) == 1

print("OK — boto3 compatible")
EOF
```

---

### Tarea 4 — FUSE driver (Linux)

**Qué hacer**: implementar `fuse/fs.rs` con el trait `fuser::Filesystem`.
Implementar las operaciones: `lookup`, `getattr`, `readdir`, `open`, `read`,
`write`, `create`, `unlink`, `rename`, `mkdir`, `rmdir`. Implementar el comando
`all4one-agent mount --bucket B --mountpoint /mnt/B`.

**Test**:
```bash
# Montar bucket:
mkdir -p /mnt/test
all4one-agent mount --bucket test --mountpoint /mnt/test \
  --endpoint http://localhost:7946 --api-key $KEY &
sleep 1

# Operaciones POSIX básicas:
echo "hola mundo" > /mnt/test/fichero.txt
cat /mnt/test/fichero.txt       # hola mundo
ls /mnt/test/                   # fichero.txt
cp /tmp/test200mb.bin /mnt/test/grande.bin
diff /tmp/test200mb.bin /mnt/test/grande.bin  # sin diferencias
rm /mnt/test/fichero.txt
ls /mnt/test/                   # solo grande.bin

# Script Python sin modificaciones accede al clúster via FUSE:
python3 -c "
with open('/mnt/test/grande.bin', 'rb') as f:
    data = f.read(1024)
print(f'Leídos {len(data)} bytes del clúster via FUSE')
"

# Desmontar:
fusermount -u /mnt/test
```

---

### Tarea 5 — FUSE driver (macOS y Windows)

**Qué hacer**: adaptar `fuse/fs.rs` para macFUSE (macOS) y WinFsp (Windows)
usando los feature flags `#[cfg(target_os = "macos")]` y
`#[cfg(target_os = "windows")]`. El comando `mount` debe funcionar en las
tres plataformas.

**Test**:
```bash
# macOS (requiere macFUSE instalado):
all4one-agent mount --bucket test --mountpoint /Volumes/all4one &
sleep 2
echo "test" > /Volumes/all4one/test.txt
cat /Volumes/all4one/test.txt   # test
diskutil unmount /Volumes/all4one

# Windows (requiere WinFsp instalado):
# all4one-agent mount --bucket test --mountpoint Z:
# echo test > Z:\test.txt
# type Z:\test.txt   → test
```

---

### Tarea 6 — LD_PRELOAD shim (Linux)

**Qué hacer**: implementar `libvolatile.so` como crate `cdylib` que intercepta
`open`, `read`, `write`, `close`, `stat` via `LD_PRELOAD`. Las llamadas a paths
que coincidan con `VOLATILE_INTERCEPT_PATH` se redirigen al agente via HTTP.

**Test**:
```bash
# Compilar la librería:
cargo build -p volatile-shim --release
# → target/release/libvolatile.so

# Subir un fichero al clúster:
curl -s -X PUT http://localhost:7946/v1/storage/test/modelo.bin \
  -H "X-All4One-Secret: s" -H "Content-Type: application/octet-stream" \
  --data-binary @/tmp/test200mb.bin

# Usar el shim con un binario existente (xxd lee el fichero):
LD_PRELOAD=./target/release/libvolatile.so \
VOLATILE_INTERCEPT_PATH=/tmp/modelo.bin \
VOLATILE_TARGET=volatile://test/modelo.bin \
VOLATILE_ENDPOINT=localhost:7946 \
VOLATILE_SECRET=s \
  xxd /tmp/modelo.bin | head -3
# Debe mostrar los primeros bytes del objeto del clúster.
```

---

### Tarea 7 — SDK Java con autoconfiguración Spring Boot

**Qué hacer**: implementar el artefacto Maven `all4one-sdk`. Implementar
`All4OneClient` con `CompletableFuture<T>`. Implementar la autoconfiguración
Spring Boot via `@EnableAll4One` y `application.properties`.

**Test**:
```java
// Test de integración Maven (JUnit 5):
@Test
void testPutAndGet() throws Exception {
    All4OneClient client = All4OneClient.builder()
        .endpoint("http://localhost:7946")
        .apiKey(System.getenv("ALL4ONE_API_KEY"))
        .build();

    byte[] data = "hello from java".getBytes();
    PutResponse resp = client.put("test", "java-test.txt", data).get(10, SECONDS);
    assertEquals(data.length, resp.getSizeBytes());

    byte[] downloaded = client.get("test", "java-test.txt").get(10, SECONDS);
    assertArrayEquals(data, downloaded);
}
```

---

### Tarea 8 — Prueba de integración final (Fase 4)

**Test**: ejecutar los 5 criterios de aceptación de la fase.
```bash
# 1. FUSE: script Python con open() estándar lee del clúster ✓
# 2. boto3: s3.upload_file() y s3.download_file() en puerto 9000 ✓
# 3. LD_PRELOAD: binario nativo lee del clúster sin modificación ✓
# 4. SDK Java + Spring Boot: @EnableAll4One inyecta All4OneClient ✓
# 5. Auth Bearer: sin header → 401, con key revocada → 401 ✓
```
