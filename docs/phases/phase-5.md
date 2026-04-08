# Fase 5 — "Plataformas y madurez"

**Objetivo**: cobertura completa de plataformas (Android), soporte GPU para ML,
inferencia distribuida con llama.cpp RPC, multi-tenant para el modelo cloud,
y UI web de administración.

---

## Componentes añadidos en Fase 5

| Componente                  | Descripción                                                      |
|-----------------------------|------------------------------------------------------------------|
| Agente Android              | App Kotlin + Rust JNI, Tier 2, storage + executable + WASM      |
| Soporte GPU + CUDA          | Executor detecta GPU, jobs con `gpu: true` se asignan correctamente |
| inference_group             | Pipeline y tensor parallelism via llama.cpp RPC                  |
| Checkpointing con CRIU      | Linux — jobs con checkpoint/restore para migración en caliente   |
| Modo Learned                | Inferencia de patrón de disponibilidad tras 2 semanas de historial |
| UI web de administración    | SPA para gestión de jobs, nodos, storage y métricas             |
| Multi-tenant                | Namespaces de bucket + quotas + metering para modelo cloud       |

---

## Agente Android

**Stack**: Kotlin + Rust JNI. La lógica de red y storage está en Rust compilado
para `aarch64-linux-android`. La app Kotlin gestiona el ciclo de vida del
servicio Android y la interfaz de usuario.

**Condiciones de activación**: el agente Android solo está activo cuando:
- Pantalla apagada
- Cargando (cable conectado)
- Conectado a WiFi

Si alguna condición deja de cumplirse, el agente envía `DrainNotice` automático.

**Batería**: DrainNotice automático cuando el nivel de batería cae al 20%.

**Capacidades**: storage + ejecutables ARM64 nativos + WASM. Sin Docker, sin JAR,
sin quórum Raft. Siempre Tier 2. Sin root requerido.

**Limitaciones de Android sin root**:
- Sin cgroups: los límites de CPU/RAM del JobSpec no se aplican.
- Sin acceso a `/proc/net`: la detección de ancho de banda es aproximada.
- Storage en `/data/user/0/io.all4one.agent/files/`.

**Estructura del proyecto Android**:
```
android/
├── app/
│   ├── src/main/
│   │   ├── java/io/all4one/agent/
│   │   │   ├── AgentService.kt     # foreground service
│   │   │   ├── MainActivity.kt     # UI de estado
│   │   │   ├── BatteryReceiver.kt  # DrainNotice al 20%
│   │   │   └── WifiReceiver.kt     # activa/desactiva según WiFi
│   │   └── jniLibs/arm64-v8a/
│   │       └── liball4one.so       # Rust compilado para Android
│   └── Cargo.toml                  # crate Rust con cfg(target_os = "android")
└── Cargo.toml
```

---

## Soporte GPU + CUDA

**Detección al arranque** (si `capabilities.gpu_enabled = true`):

```rust
// capabilities/gpu.rs
pub fn detect_gpu() -> Option<GpuProfile> {
    // Intenta cargar NVML (Nvidia Management Library)
    if let Ok(nvml) = nvml_wrapper::Nvml::init() {
        let device = nvml.device_by_index(0)?;
        return Some(GpuProfile {
            cuda_version: Some(nvml.sys_cuda_driver_version()?.to_string()),
            vram_mb: device.memory_info()?.total / 1024 / 1024,
            vendor: GpuVendor::Nvidia,
        });
    }
    // Intenta ROCm para AMD
    // Intenta Metal para Apple Silicon
    None
}
```

**Placement con GPU**: el scheduler añade un filtro adicional en el Paso 2:
```rust
.filter(|n| if spec.resources.gpu {
    n.profile.capabilities.gpu.is_some()
    && if let Some(ref cuda_min) = spec.resources.cuda_min {
        n.profile.capabilities.gpu.as_ref()
          .and_then(|g| g.cuda_version.as_ref())
          .map(|v| version_gte(v, cuda_min))
          .unwrap_or(false)
    } else {
        true
    }
} else {
    true
})
```

---

## inference_group con pipeline y tensor parallelism

Ver especificación completa en [ai-inference.md](../architecture/ai-inference.md).

El scheduler implementa en Fase 5 la distribución de capas para inference_group:

```rust
// scheduler/inference.rs

pub async fn place_inference_group(
    spec: &JobSpec,
    cluster_state: &ClusterState,
) -> Result<InferenceGroupPlacement> {
    let partitioning = spec.partitioning.as_ref()
        .ok_or(SchedulerError::MissingPartitioning)?;
    
    // Filtrar nodos con suficiente RAM y red
    let candidates = cluster_state.nodes.values()
        .filter(|n| n.status == NodeStatus::Online)
        .filter(|n| n.profile.tier as u8 >= spec.constraints.tier_min as u8)
        .filter(|n| n.profile.resources.network_bandwidth_mbps
                    >= spec.constraints.network_min_gbps.unwrap_or(0) * 1000)
        .filter(|n| n.resources.ram_mb_available > 0)
        .collect::<Vec<_>>();
    
    if candidates.len() < partitioning.min_nodes as usize {
        return Err(SchedulerError::NetworkRequirementsNotMet {
            required_gbps: spec.constraints.network_min_gbps.unwrap_or(0),
            available_nodes: candidates.len(),
        });
    }
    
    let selected = &candidates[..partitioning.max_nodes.min(candidates.len()) as usize];
    let layer_ranges = distribute_layers(model_layer_count(&spec.source), selected);
    
    Ok(InferenceGroupPlacement { nodes: layer_ranges, strategy: partitioning.strategy })
}
```

---

## Checkpointing con CRIU (Linux)

CRIU (Checkpoint/Restore In Userspace) permite guardar el estado completo de
un proceso en disco y restaurarlo más tarde, posiblemente en otro nodo.

**Casos de uso**:
- Migración en caliente: mover un job de un nodo que va a drenar a otro.
- Tolerancia a fallos: si un nodo cae con un job corriendo, restaurar el job
  en otro nodo desde el último checkpoint.

**Flujo de migración con checkpoint**:
```
Nodo origen (drenando)         Nodo destino
       │
       │  CRIU dump del proceso
       │  → archivos checkpoint en volatile://checkpoints/{job_id}/
       │
       │──[gRPC MigrateJob]──────────────────────────────►│
       │                                                   │  CRIU restore
       │                                                   │  del proceso
       │◄──────────────────────────────[JobEvent::Started]─│
       │
 Job eliminado del origen
```

**Limitaciones**: solo Linux. Requiere que el proceso sea checkpointable con CRIU
(sin file descriptors que no se puedan serializar, sin operaciones TCP activas
salvo con `--tcp-established`).

**Decisión pendiente**: lista completa de runtimes que soportan checkpointing
(Docker con CRIU, ejecutables nativos con CRIU, Java con CRIU experimental).

---

## Modo Learned (ventanas de disponibilidad)

Para nodos con `availability = "learned"` en `agent.toml`.

El agente registra cada vez que el nodo está online/offline durante 2 semanas.
Tras ese periodo, infiere un patrón y lo usa para predecir disponibilidad futura.

```rust
// Ejemplo de patrón inferido:
// Lunes–viernes: online 09:00–18:30 con probabilidad 0.92
// Sábado: online 10:00–13:00 con probabilidad 0.45
// Domingo: online < 0.10 de probabilidad

pub struct LearnedAvailability {
    pub history: Vec<(DateTime<Utc>, bool)>,   // online/offline timestamps
    pub pattern: WeeklyPattern,                 // inferido tras 2 semanas
}

pub struct WeeklyPattern {
    pub slots: [[f32; 24]; 7],   // probabilidad por hora y día de semana
}
```

El scheduler usa `pattern.slots[weekday][hour]` como factor en la señal
`ventana` del algoritmo de placement. Solo se confía en el patrón si hay
al menos 14 días de historial.

---

## UI web de administración

SPA React servida en el puerto `7946` bajo la ruta `/admin/`.

**Vistas implementadas**:
- **Dashboard**: nodos online/offline, jobs corriendo, uso de storage, quórum.
- **Jobs**: lista de jobs con filtros, detalle de un job, output en tiempo real.
- **Nodos**: mapa del clúster con tier, recursos y estado de cada nodo.
- **Storage**: explorador de buckets, métricas de uso por tier, heat scores.
- **Administración**: generar tokens, revocar nodos, gestionar API keys.

**Decisión pendiente**: tecnología de bundling (Vite vs. esbuild) y si la UI
se compila dentro del binario del agente (via `include_bytes!`) o se sirve
como ficheros estáticos separados.

---

## Multi-tenant (modelo cloud)

Para el modelo cloud operado por All4One, múltiples organizaciones comparten
la misma infraestructura con aislamiento completo.

**Aislamiento**:
- Namespaces de bucket: `{org_id}/{bucket}/{key}` — una organización no puede
  acceder a los buckets de otra.
- Quotas en Raft: `ClusterConfig` incluye `HashMap<OrgId, OrgQuota>` con
  límites de CPU·hora, GB almacenados y GB transferidos.
- Nodos dedicados: un nodo puede tener `tenant_id` que restringe qué
  organización puede usarlo.

**Metering** (para facturación):
```rust
pub struct UsageRecord {
    pub org_id:        OrgId,
    pub period_start:  DateTime<Utc>,
    pub cpu_hours:     f64,            // CPU·hora consumidas por jobs
    pub storage_gb_months: f64,        // GB·mes almacenados
    pub transfer_gb:   f64,            // GB transferidos (GET)
}
```

Los `UsageRecord` se escriben en Raft al final de cada job y en el ciclo de
lifecycle, para garantizar que son auditables y no se pueden perder.

---

## Criterios de aceptación (Fase 5)

1. **Android**: agente Android en un Pixel 7 almacena chunks y los sirve al
   clúster mientras carga y la pantalla está apagada. DrainNotice automático
   al 20% de batería.

2. **GPU**: job con `resources.gpu: true` se asigna solo a nodo con GPU
   disponible. Job Docker con imagen CUDA corre con acceso a GPU.

3. **inference_group 70B**: job `inference_group` con `strategy: pipeline` y
   `min_nodes: 3` en clúster de 4 nodos con 10GbE. `POST /v1/chat/completions`
   devuelve respuesta en < 30 segundos.

4. **CRIU migración**: job de larga duración en nodo que inicia drenado migra
   al siguiente nodo sin reiniciarse desde cero — el proceso continúa desde
   el último checkpoint.

5. **Multi-tenant**: organización A no puede listar ni descargar objetos
   de organización B aunque use el mismo clúster físico. `403 Forbidden` al intentarlo.

---

## Dependencias con fases anteriores

Requiere Fase 1–4 completas. La UI web requiere la API REST completa de Fase 4
(auth Bearer token). El multi-tenant requiere Raft de Fase 2.

---

## Lista de tareas ordenadas

---

### Tarea 1 — Soporte GPU: detección y placement

**Qué hacer**: implementar `capabilities/gpu.rs` con detección via NVML
(Nvidia), ROCm (AMD) y Metal (Apple). Añadir el filtro de GPU + CUDA en el
algoritmo de placement del scheduler. Actualizar `NodeCapabilities.gpu` en
el `NodeProfile` anunciado via gossip.

**Test**:
```bash
# En nodo con GPU Nvidia:
all4one-agent capabilities
# gpu: { vendor: "nvidia", cuda_version: "12.1", vram_mb: 8192 }

# Job con gpu=true → asignado solo al nodo con GPU:
JOB_ID=$(curl -s -X POST http://nodo1:7946/v1/jobs \
  -H "Content-Type: application/yaml" -H "Authorization: Bearer $KEY" \
  -d 'runtime: docker
source: nvidia/cuda:12.1-base-ubuntu22.04
command: ["nvidia-smi"]
resources: {cpu_cores: 1, memory_mb: 1024, gpu: true}' | \
  python3 -c "import sys,json; print(json.load(sys.stdin)['job_id'])")

sleep 10
curl -s -H "Authorization: Bearer $KEY" \
  http://nodo1:7946/v1/jobs/$JOB_ID/output | \
  python3 -c "import sys,json; print(json.load(sys.stdin)['stdout'])"
# Muestra la salida de nvidia-smi con la GPU detectada.

# Job con gpu=true en clúster sin GPU → queued:
# (todos los nodos sin GPU)
curl -s -H "Authorization: Bearer $KEY" \
  http://nodo1:7946/v1/jobs/$JOB_ID2 | \
  python3 -c "import sys,json; print(json.load(sys.stdin)['status'])"
# queued
```

---

### Tarea 2 — inference_group: pipeline parallelism con llama.cpp RPC

**Qué hacer**: implementar `scheduler/inference.rs` con `place_inference_group()`
que distribuye las capas del modelo entre nodos candidatos según su RAM.
El executor lanza `llama-server` en el nodo coordinador y `llama-rpc-server`
en los workers, con los rangos de capas calculados por el scheduler.

**Test**:
```bash
# Subir modelo pequeño para test (usar Llama 3 8B):
curl -s -X PUT http://nodo1:7946/v1/storage/models/llama3-8b.gguf \
  -H "Authorization: Bearer $KEY" \
  -H "Content-Type: application/octet-stream" \
  -H "X-All4One-Policy: manual" \
  --data-binary @/tmp/llama3-8b-q4_k_m.gguf

# Lanzar inference_group con 2 nodos (8B es pequeño, usar min_nodes=2):
JOB_ID=$(curl -s -X POST http://nodo1:7946/v1/jobs \
  -H "Content-Type: application/yaml" -H "Authorization: Bearer $KEY" \
  -d 'runtime: inference_group
source: "volatile://models/llama3-8b.gguf"
partitioning: {strategy: pipeline, min_nodes: 2, max_nodes: 2}
constraints: {tier_min: 0, same_network: true}
resources: {cpu_cores: 0, memory_mb: 0}' | \
  python3 -c "import sys,json; print(json.load(sys.stdin)['job_id'])")

# Esperar a que el servidor esté listo (status=running):
sleep 30
curl -s -H "Authorization: Bearer $KEY" \
  http://nodo1:7946/v1/jobs/$JOB_ID | \
  python3 -c "import sys,json; print(json.load(sys.stdin)['status'])"
# running

# Inferencia via API OpenAI compatible:
curl -s -X POST http://nodo1:7946/v1/chat/completions \
  -H "Content-Type: application/json" -H "Authorization: Bearer $KEY" \
  -d '{"model": "llama3-8b", "messages": [{"role": "user", "content": "Di hola"}]}' | \
  python3 -c "import sys,json; print(json.load(sys.stdin)['choices'][0]['message']['content'])"
# Alguna respuesta en español.
```

---

### Tarea 3 — inference_group: rechazo por red insuficiente

**Qué hacer**: el scheduler verifica `network_min_gbps` antes de aceptar un
job `inference_group` con `strategy: tensor`. Si los nodos no cumplen el
requisito, devuelve `422 NETWORK_REQUIREMENTS_NOT_MET`.

**Test**:
```bash
# En clúster con nodos de 1 GbE, intentar tensor parallelism con 25 Gbps mínimo:
curl -s -X POST http://nodo1:7946/v1/jobs \
  -H "Content-Type: application/yaml" -H "Authorization: Bearer $KEY" \
  -d 'runtime: inference_group
source: "volatile://models/llama3-8b.gguf"
partitioning: {strategy: tensor, min_nodes: 2, max_nodes: 4}
constraints: {network_min_gbps: 25, same_network: true}
resources: {cpu_cores: 0, memory_mb: 0}'
# HTTP 422
# { "code": "NETWORK_REQUIREMENTS_NOT_MET",
#   "message": "tensor parallelism requires 25 Gbps, max available: 1 Gbps", ... }
```

---

### Tarea 4 — Modo Learned: inferencia de patrón de disponibilidad

**Qué hacer**: implementar el registro de eventos online/offline en
`node/availability.rs`. Tras 14 días de historial, calcular el
`WeeklyPattern` (probabilidad por hora y día). El scheduler usa el patrón
como factor en la señal `ventana` del placement.

**Test**:
```bash
# Simular 14 días de historial con patrón 9:00-18:00 L-V:
# (inyectar historial via comando de admin para tests)
all4one-agent inject-availability-history \
  --pattern "cron:0 9-18 * * 1-5" \
  --days 14 \
  --data-dir /var/lib/all4one

# Verificar que el patrón inferido coincide:
all4one-agent show-learned-availability --data-dir /var/lib/all4one
# Lunes 09:00 → probabilidad: 0.93
# Lunes 20:00 → probabilidad: 0.04
# Sábado 12:00 → probabilidad: 0.08
```

---

### Tarea 5 — Agente Android (Kotlin + Rust JNI)

**Qué hacer**: implementar la app Android con `AgentService.kt` (foreground
service), `BatteryReceiver.kt` (DrainNotice al 20%), `WifiReceiver.kt`
(activa/desactiva según WiFi + cargando). El módulo Rust compilado como
`liball4one.so` implementa storage y executor ARM64.

**Test**:
```bash
# En dispositivo Android conectado via ADB:
adb install app-debug.apk

# Iniciar servicio y verificar que el nodo aparece en el clúster:
adb shell am start -n io.all4one.agent/.MainActivity
sleep 10
curl -s -H "Authorization: Bearer $KEY" \
  http://nodo1:7946/v1/nodes | \
  python3 -c "import sys,json; nodes=json.load(sys.stdin)['nodes']; \
  [print(n['profile']['platform'], n['profile']['tier']) for n in nodes]"
# android_arm64  2

# Al 20% de batería, el agente envía DrainNotice:
adb shell dumpsys battery set level 19
sleep 5
curl -s -H "Authorization: Bearer $KEY" \
  http://nodo1:7946/v1/nodes | \
  python3 -c "import sys,json; nodes=json.load(sys.stdin)['nodes']; \
  android=[n for n in nodes if 'android' in n['profile']['platform']]; \
  print(android[0]['status'])"
# draining
```

---

### Tarea 6 — Checkpointing con CRIU

**Qué hacer**: implementar `executor/checkpoint.rs` con las funciones
`checkpoint(job_id) -> Result<CheckpointPath>` y
`restore(checkpoint_path) -> Result<ProcessHandle>`. Integrar con el flujo
de drenado: cuando un nodo drena con jobs corriendo, los checkpoints se
almacenan en `volatile://checkpoints/{job_id}/` y se restauran en el
nodo destino.

**Test**:
```bash
# Lanzar job de larga duración que escribe un contador en stdout:
JOB_ID=$(curl -s -X POST http://nodoA:7946/v1/jobs \
  -H "Content-Type: application/yaml" -H "Authorization: Bearer $KEY" \
  -d 'runtime: executable
source: "volatile://bins/counter"
command: []
resources: {cpu_cores: 1, memory_mb: 256}' | \
  python3 -c "import sys,json; print(json.load(sys.stdin)['job_id'])")

# Esperar a que imprima ~10 líneas (contador en 10):
sleep 12

# Iniciar drenado del nodo A → job debe migrarse a nodo B via CRIU:
all4one-agent drain --in 1m --data-dir /var/lib/all4one-A

sleep 65  # esperar a que complete el drenado

# Verificar que el job sigue corriendo en nodo B desde donde lo dejó:
curl -s -H "Authorization: Bearer $KEY" \
  http://nodoB:7946/v1/jobs/$JOB_ID | \
  python3 -c "import sys,json; d=json.load(sys.stdin); \
  print(d['status'], d['assigned_to'])"
# running  <UUID de nodo B>

# El contador continúa desde ~10, no desde 0:
curl -s -H "Authorization: Bearer $KEY" \
  http://nodoB:7946/v1/jobs/$JOB_ID/output | \
  python3 -c "import sys,json; lines=json.load(sys.stdin)['stdout'].strip().split('\n'); \
  print('Primera línea post-migración:', lines[-5])"
# Primera línea post-migración: 11  (o similar — no empieza desde 1)
```

---

### Tarea 7 — Multi-tenant: namespaces y quotas

**Qué hacer**: añadir `OrgId` como campo en `ClusterConfig`. Implementar
aislamiento de buckets (`{org_id}/{bucket}` internamente). Implementar
`ClusterConfig.org_quotas: HashMap<OrgId, OrgQuota>`. El middleware de auth
rechaza acceso a buckets de otra organización con `403 Forbidden`.

**Test**:
```bash
# Org A sube un objeto:
curl -s -X PUT http://nodo1:7946/v1/storage/bucket-a/fichero.txt \
  -H "Authorization: Bearer key-org-a" \
  -H "Content-Type: application/octet-stream" \
  -d "contenido privado org A"

# Org B intentando acceder al bucket de Org A → 403:
curl -s -o /dev/null -w "%{http_code}" \
  -H "Authorization: Bearer key-org-b" \
  http://nodo1:7946/v1/storage/bucket-a/fichero.txt
# 403

# Org A puede acceder a su propio bucket → 200:
curl -s -o /dev/null -w "%{http_code}" \
  -H "Authorization: Bearer key-org-a" \
  http://nodo1:7946/v1/storage/bucket-a/fichero.txt
# 200
```

---

### Tarea 8 — UI web de administración

**Qué hacer**: implementar la SPA React servida en `/admin/` en el puerto 7946.
Vistas: Dashboard, Jobs, Nodos, Storage, Administración.

**Test**:
```bash
# La UI carga sin errores:
curl -s -o /dev/null -w "%{http_code}" \
  http://nodo1:7946/admin/
# 200

# Dashboard muestra nodos online:
# (test manual en navegador — verificar que el conteo de nodos
#  coincide con GET /v1/nodes)

# La UI de Jobs muestra output en tiempo real via SSE:
# (test manual — lanzar un job desde la UI y verificar que
#  las líneas de stdout aparecen a medida que se generan)
```

---

### Tarea 9 — Prueba de integración final (Fase 5)

**Test**: ejecutar los 5 criterios de aceptación de la fase.
```bash
# 1. Android: agente en Pixel 7 almacena chunks; DrainNotice al 20% batería ✓
# 2. GPU: job con gpu:true asignado solo a nodo con GPU; nvidia-smi funciona ✓
# 3. inference_group 8B pipeline: respuesta en < 30s en clúster de 2 nodos ✓
# 4. CRIU migración: job contador continúa desde el mismo número tras migrar ✓
# 5. Multi-tenant: org B recibe 403 al acceder a bucket de org A ✓
```
