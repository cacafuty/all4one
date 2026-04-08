# Lifecycle engine

El lifecycle engine gestiona automáticamente el tier de cada objeto almacenado
en el clúster, moviéndolo entre Hot, Warm, Cold y Archive según su patrón de acceso.

**Solo activo en el líder Raft.** Corre como tarea `tokio` cada 6 horas.

---

## Heat score

El heat score cuantifica cuán "caliente" es un objeto — con qué frecuencia y
por cuántos usuarios distintos se accede a él. Es un número entre `0.0` y `1.0`.

### Fórmula

```
heat_score = Σ(señal × peso), normalizado a [0.0, 1.0]

Señal 1: accesos_7d / 100                                × 0.35
Señal 2: accesos_30d / 1000                              × 0.20
Señal 3: e^(-días_desde_último_acceso / 7)               × 0.25
Señal 4: min(usuarios_distintos_30d / 50, 1.0)           × 0.10
Señal 5: 1.0 si accesos distribuidos, 0.1 si pico único  × 0.10
```

La señal 5 distingue un pico de uso puntual (un solo día con muchos accesos)
de un acceso sostenido distribuido en el tiempo. Se calcula como:

```rust
fn distributed_score(access_timestamps: &[DateTime<Utc>]) -> f32 {
    // Divide los últimos 30 días en ventanas de 7 días (4 ventanas)
    // Si hay accesos en al menos 3 de las 4 ventanas → 1.0 (distribuido)
    // Si los accesos se concentran en 1 ventana → 0.1 (pico único)
    let windows_with_access = (0..4)
        .filter(|&w| {
            let start = Utc::now() - Duration::days(30 - w * 7);
            let end   = Utc::now() - Duration::days(30 - (w + 1) * 7);
            access_timestamps.iter().any(|t| *t >= end && *t < start)
        })
        .count();
    if windows_with_access >= 3 { 1.0 } else { 0.1 }
}
```

### Ejemplo de cálculo

Un objeto con:
- 45 accesos en los últimos 7 días
- 320 accesos en los últimos 30 días
- Último acceso hace 2 días
- 18 usuarios distintos en 30 días
- Accesos distribuidos (4/4 ventanas tienen accesos)

```
Señal 1: (45/100) × 0.35  = 0.45 × 0.35 = 0.1575
Señal 2: (320/1000) × 0.20 = 0.32 × 0.20 = 0.0640
Señal 3: e^(-2/7) × 0.25  = 0.754 × 0.25 = 0.1886
Señal 4: (18/50) × 0.10   = 0.36 × 0.10  = 0.0360
Señal 5: 1.0 × 0.10       = 0.10

heat_score = 0.1575 + 0.0640 + 0.1886 + 0.0360 + 0.10 = 0.546 → Warm
```

---

## Transiciones de tier

### Umbrales

```
heat_score > 0.8  → Hot
heat_score 0.4–0.8 → Warm
heat_score 0.1–0.4 → Cold
heat_score < 0.1  → Archive
```

### Periodo de gracia

Las transiciones **hacia abajo** (Hot→Warm, Warm→Cold, Cold→Archive) tienen
un periodo de gracia de **7 días**. Un objeto no baja de tier hasta que su
heat score permanece por debajo del umbral durante 7 días consecutivos.

El campo `last_tier_change` en `FileMetadata` registra la última transición.

```rust
fn should_transition_down(metadata: &FileMetadata, new_tier: DataTier) -> bool {
    let days_since_change = (Utc::now() - metadata.last_tier_change).num_days();
    days_since_change >= 7
}
```

Las transiciones **hacia arriba** (Archive→Cold, Cold→Warm, Warm→Hot) son
**inmediatas** en el siguiente ciclo de 6 horas (sin periodo de gracia).

### Ciclo de ejecución

```
Cada 6 horas en el líder Raft:

1. Lee el BlockMap completo de Raft
   → lista de todos los FileMetadata del clúster

2. Para cada objeto:
   a. Recalcula heat_score con los datos más recientes
   b. Determina el tier objetivo según los umbrales
   c. Si tier_objetivo != tier_actual:
      - Transición hacia arriba → encola MigrationJob inmediatamente
      - Transición hacia abajo → verifica periodo de gracia de 7 días
        Si elapsed >= 7 días → encola MigrationJob
        Si elapsed < 7 días  → actualiza heat_score en Raft, no migra

3. Actualiza FileMetadata.heat_score en Raft para cada objeto evaluado

4. Los MigrationJobs se encolan en el scheduler con prioridad Low
   (no interfieren con jobs de usuario)
```

---

## Hints de acceso

Los hints se especifican en `PUT /v1/storage/` via el header
`X-All4One-Access-Hint` o en el job spec vía `DataMount.access_hint`.

Se procesan en el **siguiente ciclo de 6 horas** (no son inmediatos salvo
`ARCHIVE_IMMEDIATELY`).

| Hint                  | Efecto                                                                     |
|-----------------------|----------------------------------------------------------------------------|
| `read_once`           | El objeto va a Cold en el siguiente ciclo, ignorando el heat score actual  |
| `frequent`            | Mantiene Hot durante 30 días ignorando el heat score                       |
| `archive_immediately` | Archiva en el siguiente ciclo **sin periodo de gracia**                    |
| `normal`              | Comportamiento estándar por heat score (default)                           |

```rust
// FileMetadata incluye el hint activo
pub struct FileMetadata {
    // ... campos anteriores ...
    pub access_hint: Option<AccessHint>,
    pub hint_expires_at: Option<DateTime<Utc>>,  // para FREQUENT (30 días)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AccessHint {
    ReadOnce,
    Frequent,
    ArchiveImmediately,
    Normal,
}
```

---

## Políticas de storage

Las políticas determinan cómo el lifecycle engine gestiona un objeto.
Se especifican al subir el objeto y se almacenan en `FileMetadata.policy`.

### Manual

```yaml
# El tier es fijo. El lifecycle engine no lo mueve.
X-All4One-Policy: manual
# Campos adicionales vía JSON en X-All4One-Policy-Config:
# { "tier": "hot", "replicas": 3, "compression": "none", "pin": true }
```

Con `pin: true`, el objeto nunca se mueve aunque el heat score caiga por debajo
de los umbrales. Recomendado para modelos de IA y datos de referencia.

### Auto

```yaml
# El lifecycle engine gestiona réplicas automáticamente.
X-All4One-Policy: auto
# Campos: min_replicas, max_replicas, archive_after_days
# { "min_replicas": 2, "max_replicas": 4, "archive_after_days": 90 }
```

### Tiered

```yaml
# Política con tiempo explícito por tier.
X-All4One-Policy: tiered
# { "hot_days": 7, "warm_days": 30, "then": "archive" }
# El objeto pasa de Hot a Warm a los 7 días, y de Warm a Archive a los 37 días.
# Ignora el heat score — solo cuenta el tiempo desde la subida.
```

---

## Restauración desde Archive

Cuando se solicita `GET /v1/storage/{bucket}/{key}` sobre un objeto en Archive:

```
Cliente → GET /v1/storage/datasets/old-model.tar
               │
               ▼
storage detecta tier=Archive
               │
               ▼
202 Accepted {
  "status": "restoring",
  "restore_job_id": "b2c3d4e5-f6a7-8901-bcde-f01234567890",
  "estimated_minutes": 45
}
               │
               ▼
lifecycle engine encola RestoreJob con prioridad Normal:
  1. Localiza fragmentos RS(8,4) del objeto en los nodos
  2. Descarga y decodifica los fragmentos
  3. Descomprime Zstd-22
  4. Almacena el objeto restaurado temporalmente como Hot
  5. Actualiza FileMetadata.tier = Hot, heat_score recalculado
               │
               ▼
Cliente polling: GET /v1/storage/restore/{restore_job_id}
  202: { "status": "restoring", "progress": 0.65, "estimated_minutes_remaining": 16 }
  200: { "status": "ready", "download_url": "http://node:7946/v1/storage/...",
         "expires_at": "2026-04-09T12:00:00Z" }
               │
               ▼
Cliente descarga el objeto restaurado desde download_url
```

El objeto restaurado permanece en Hot durante el tiempo configurado en
`storage.restore_ttl_hours` (default: 24 horas). Tras ese tiempo, el lifecycle
engine lo re-archiva si el heat score no justifica mantenerlo caliente.

---

## MigrationJob

El lifecycle engine no mueve datos directamente — encola `MigrationJob` en el
scheduler, que los ejecuta como jobs internos del sistema:

```rust
pub struct MigrationJob {
    pub file_id:    FileId,
    pub from_tier:  DataTier,
    pub to_tier:    DataTier,
    pub reason:     MigrationReason,
}

pub enum MigrationReason {
    HeatScoreTransition { old_score: f32, new_score: f32 },
    HintApplied(AccessHint),
    PolicyChange,
    Restore,
}
```

Los MigrationJobs tienen `priority: Low` para no interferir con jobs de usuario,
salvo `ArchiveImmediately` y `Restore` que tienen `priority: Normal`.

---

## Métricas de lifecycle

Expuestas en `/metrics` (Prometheus):

```
# HELP all4one_lifecycle_objects_by_tier Objects per storage tier
# TYPE all4one_lifecycle_objects_by_tier gauge
all4one_lifecycle_objects_by_tier{tier="hot"}     1240
all4one_lifecycle_objects_by_tier{tier="warm"}    8930
all4one_lifecycle_objects_by_tier{tier="cold"}    45200
all4one_lifecycle_objects_by_tier{tier="archive"} 120000

# HELP all4one_lifecycle_migrations_total Tier migrations performed
# TYPE all4one_lifecycle_migrations_total counter
all4one_lifecycle_migrations_total{from="hot",to="warm"}       342
all4one_lifecycle_migrations_total{from="warm",to="cold"}     1204
all4one_lifecycle_migrations_total{from="cold",to="archive"}   891
all4one_lifecycle_migrations_total{from="archive",to="cold"}    23

# HELP all4one_lifecycle_cycle_duration_seconds Duration of the last lifecycle cycle
# TYPE all4one_lifecycle_cycle_duration_seconds gauge
all4one_lifecycle_cycle_duration_seconds 142.3
```
