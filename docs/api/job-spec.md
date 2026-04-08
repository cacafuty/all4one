# Especificación del Job

Un `JobSpec` es la unidad de trabajo que se envía al scheduler via
`POST /v1/jobs`. Se acepta en formato YAML o JSON.

---

## Schema completo

```yaml
# Identificador del job. Si se omite, el scheduler genera un UUID v4.
# Si se especifica y ya existe un job con ese id, se devuelve el estado
# actual sin relanzar (idempotencia explícita).
id: "a1b2c3d4-e5f6-7890-abcd-ef1234567890"   # tipo: Option<JobId (UUID v4)>, default: null

# Runtime que ejecutará el job.
# Valores válidos: docker | jar | python | executable | wasm | inference_group
runtime: docker   # tipo: Runtime (enum), requerido

# Imagen Docker, path al JAR, módulo Python, path al ejecutable,
# path al módulo WASM, o URI del modelo (inference_group).
source: "python:3.11-slim"   # tipo: String, requerido

# Comando a ejecutar dentro del proceso.
# Para Docker: equivalente a CMD [].
# Para Python: argumentos a pasar al intérprete después del módulo.
# Para Executable/JAR/WASM: argumentos del proceso.
command:   # tipo: Vec<String>, default: []
  - "python"
  - "-c"
  - "print('hello')"

# Variables de entorno inyectadas en el proceso.
env:   # tipo: HashMap<String, String>, default: {}
  MY_VAR: "valor"
  BATCH_SIZE: "32"

# Directorio de trabajo dentro del proceso.
workdir: "/workspace"   # tipo: Option<String>, default: "/workspace"

# Recursos requeridos para ejecutar el job.
resources:
  cpu_cores: 2           # tipo: u32, requerido. Núcleos de CPU a reservar.
  memory_mb: 1024        # tipo: u64, requerido. RAM en MB a reservar.
  gpu: false             # tipo: bool, default: false
  cuda_min: "12.0"       # tipo: Option<String>, default: null. Versión CUDA mínima.
  max_duration_minutes: 60  # tipo: Option<u32>, default: null (sin límite)

# Datos a montar en el proceso desde el clúster.
data:   # tipo: Vec<DataMount>, default: []
  - source: "volatile://datasets/training/imagenet"  # URI en el clúster
    mount: "/data/imagenet"   # path dentro del proceso donde se monta
    mode: read                # tipo: MountMode. Valores: read | write | readwrite

# Restricciones de placement.
constraints:
  tier_min: 0             # tipo: Tier (0|1|2), default: 0 (cualquier tier)
  requires_capabilities:  # tipo: CapabilityRequirements, default: {}
    docker: false
    python_min: "3.10"    # tipo: Option<String>, default: null
    java_min: null        # tipo: Option<String>, default: null
    wasm: false
    gpu: false
    cuda_min: null        # tipo: Option<String>, default: null
    platform: null        # tipo: Option<Platform>, default: null (cualquier plataforma)
  deadline: null          # tipo: Option<DateTime<Utc>>, default: null
  node_id: null           # tipo: Option<NodeId>, default: null. Solo para debug.
  with_job: null          # tipo: Option<JobId>, default: null. Afinidad: mismo nodo.
  not_with_job: null      # tipo: Option<JobId>, default: null. Anti-afinidad.
  same_network: false     # tipo: bool, default: false
  network_min_gbps: null  # tipo: Option<u32>, default: null

# Política de reintentos.
retry:
  max_attempts: 1        # tipo: u32, default: 1 (sin reintentos)
  idempotent: false      # tipo: bool, default: false. Si false, no reintenta en Lost.
  backoff_seconds: 30    # tipo: u32, default: 30

# Prioridad en la cola del scheduler.
# Valores válidos: low | normal | high
priority: normal   # tipo: Priority (enum), default: normal

# Política de red del proceso.
network:
  internet_access: false  # tipo: bool, default: false
  cluster_access: false   # tipo: bool, default: false
```

---

## Ejemplos completos

### Ejemplo 1: job Docker — inferencia batch con GPU

```yaml
id: "b7e3f1a2-9c4d-5e6f-7a8b-9c0d1e2f3a4b"
runtime: docker
source: "pytorch/pytorch:2.3.0-cuda12.1-cudnn8-runtime"
command:
  - "python"
  - "/workspace/infer.py"
  - "--input=/data/pending"
  - "--output=/data/results"
  - "--batch-size=64"
env:
  CUDA_VISIBLE_DEVICES: "0"
  HF_HOME: "/models"
workdir: "/workspace"
resources:
  cpu_cores: 4
  memory_mb: 16384
  gpu: true
  cuda_min: "12.1"
  max_duration_minutes: 120
data:
  - source: "volatile://datasets/pending-images"
    mount: "/data/pending"
    mode: read
  - source: "volatile://results/batch-2024-04"
    mount: "/data/results"
    mode: readwrite
  - source: "volatile://models/resnet50.pth"
    mount: "/models/resnet50.pth"
    mode: read
constraints:
  tier_min: 1
  requires_capabilities:
    gpu: true
    cuda_min: "12.1"
  deadline: "2026-04-08T23:59:00Z"
retry:
  max_attempts: 2
  idempotent: true
  backoff_seconds: 60
priority: high
network:
  internet_access: false
  cluster_access: false
```

### Ejemplo 2: job JAR — procesamiento de datos con Java

```yaml
runtime: jar
source: "volatile://apps/etl-processor-2.1.0.jar"
command:
  - "--config=/config/etl.json"
  - "--input=/data/raw"
  - "--output=/data/processed"
  - "--partitions=8"
env:
  JAVA_OPTS: "-Xmx6g -XX:+UseG1GC"
  LOG_LEVEL: "INFO"
workdir: "/workspace"
resources:
  cpu_cores: 8
  memory_mb: 8192
  gpu: false
  max_duration_minutes: 240
data:
  - source: "volatile://raw-data/april-2026"
    mount: "/data/raw"
    mode: read
  - source: "volatile://processed-data/april-2026"
    mount: "/data/processed"
    mode: readwrite
  - source: "volatile://config/etl-config"
    mount: "/config"
    mode: read
constraints:
  tier_min: 0
  requires_capabilities:
    java_min: "21.0"
  same_network: false
retry:
  max_attempts: 3
  idempotent: true
  backoff_seconds: 120
priority: normal
network:
  internet_access: false
  cluster_access: false
```

### Ejemplo 3: job Python — scraping con acceso a internet

```yaml
runtime: python
source: "volatile://scripts/scraper/main.py"
command:
  - "--sites-file=/config/sites.txt"
  - "--output=/data/scraped"
  - "--concurrency=10"
env:
  PYTHONPATH: "/workspace/lib"
  HTTP_TIMEOUT: "30"
  MAX_RETRIES: "3"
workdir: "/workspace"
resources:
  cpu_cores: 2
  memory_mb: 2048
  gpu: false
  max_duration_minutes: 480
data:
  - source: "volatile://config/scraper"
    mount: "/config"
    mode: read
  - source: "volatile://scraped-data/2026-04"
    mount: "/data/scraped"
    mode: readwrite
constraints:
  tier_min: 1
  requires_capabilities:
    python_min: "3.11"
  deadline: "2026-04-09T06:00:00Z"
retry:
  max_attempts: 2
  idempotent: false
  backoff_seconds: 300
priority: low
network:
  internet_access: true   # este job necesita acceso a internet
  cluster_access: false
```

### Ejemplo 4: inference_group — modelo 70B distribuido en 3 nodos

```yaml
runtime: inference_group
source: "volatile://models/llama3-70b-q4_k_m.gguf"
command: []
env:
  LLAMA_N_CTX: "8192"
  LLAMA_N_PARALLEL: "4"
workdir: "/workspace"
resources:
  cpu_cores: 0    # el scheduler calcula recursos por nodo automáticamente
  memory_mb: 0    # ídem
  gpu: false
  max_duration_minutes: 10080   # 7 días — servidor de larga duración
partitioning:
  strategy: pipeline
  min_nodes: 3
  max_nodes: 5
constraints:
  tier_min: 1
  requires_capabilities:
    platform: null    # cualquier plataforma con suficiente RAM
  network_min_gbps: 10
  same_network: true
retry:
  max_attempts: 1
  idempotent: false
  backoff_seconds: 0
priority: high
network:
  internet_access: false
  cluster_access: true    # los nodos worker necesitan comunicarse entre sí
```

### Ejemplo 5: ejecutable nativo ARM64 — compilación cruzada

```yaml
id: "c3d4e5f6-a7b8-9012-cdef-012345678901"
runtime: executable
source: "volatile://apps/build-server/build-server-linux-arm64"
command:
  - "--project=/src/myapp"
  - "--output=/artifacts"
  - "--target=release"
  - "--jobs=4"
env:
  CARGO_HOME: "/workspace/.cargo"
  RUSTUP_HOME: "/workspace/.rustup"
workdir: "/src"
resources:
  cpu_cores: 4
  memory_mb: 4096
  gpu: false
  max_duration_minutes: 30
data:
  - source: "volatile://source/myapp-main"
    mount: "/src/myapp"
    mode: read
  - source: "volatile://artifacts/myapp"
    mount: "/artifacts"
    mode: write
constraints:
  tier_min: 0
  requires_capabilities:
    platform: linux_arm64
retry:
  max_attempts: 1
  idempotent: true
  backoff_seconds: 0
priority: normal
network:
  internet_access: false
  cluster_access: false
```

---

## Validación del JobSpec

El módulo `api_rest` valida el `JobSpec` antes de pasarlo al scheduler.
Un `JobSpec` inválido devuelve `400 Bad Request` con `code: INVALID_JOB_SPEC`.

Reglas de validación:

| Campo                        | Regla                                                      |
|------------------------------|------------------------------------------------------------|
| `runtime`                    | Valor en el enum válido                                    |
| `source`                     | No vacío                                                   |
| `resources.cpu_cores`        | >= 1 (salvo `inference_group` donde puede ser 0)           |
| `resources.memory_mb`        | >= 128 (salvo `inference_group` donde puede ser 0)         |
| `resources.cuda_min`         | Solo si `resources.gpu = true`                             |
| `data[].source`              | Formato `volatile://bucket/path`                           |
| `data[].mount`               | Path absoluto que comienza por `/`                         |
| `constraints.tier_min`       | Valor 0, 1 o 2                                             |
| `constraints.deadline`       | Si presente, debe ser en el futuro                         |
| `retry.max_attempts`         | >= 1                                                       |
| `retry.backoff_seconds`      | >= 0                                                       |
| `partitioning.min_nodes`     | >= 2 si `runtime = inference_group`                        |
| Tamaño total del YAML/JSON   | <= 1 MB                                                    |
