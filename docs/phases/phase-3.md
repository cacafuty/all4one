# Fase 3 — "Los datos se gestionan solos"

**Objetivo**: el sistema optimiza automáticamente el tier de cada objeto según
su patrón de acceso, comprimiendo datos fríos y archivando los que ya no se usan.
El operador no tiene que gestionar el storage manualmente.

---

## Módulos añadidos en Fase 3

| Módulo      | Estado | Notas                                                    |
|-------------|--------|----------------------------------------------------------|
| `lifecycle` | Nuevo  | Heat score, transiciones de tier, políticas, hints       |

## Módulos extendidos en Fase 3

| Módulo     | Extensión                                                            |
|------------|----------------------------------------------------------------------|
| `storage`  | Restauración asíncrona desde Archive, scrubbing con métricas         |
| `api_rest` | `GET /v1/storage/restore/{restore_job_id}`                           |

---

## Alcance detallado

### Qué está incluido

**Lifecycle engine**:
- Heat score con las 5 señales (accesos_7d, accesos_30d, decaimiento temporal,
  usuarios distintos, distribución de accesos).
- Ciclo de recálculo cada 6 horas en el líder Raft.
- Transiciones automáticas: Hot ↔ Warm ↔ Cold ↔ Archive.
- Periodo de gracia de 7 días para transiciones hacia abajo.
- Transiciones hacia arriba inmediatas en el siguiente ciclo.
- Políticas por objeto: Manual, Auto, Tiered.
- Hints: `READ_ONCE`, `FREQUENT`, `ARCHIVE_IMMEDIATELY`, `NORMAL`.
- Pin: objetos con `pin=true` nunca se mueven.

**Compresión por tier** (ya disponible en Fase 2 para nuevas escrituras,
pero en Fase 3 se aplica a objetos existentes al migrar de tier):
- Hot → Warm: se aplica Zstd-3 al migrar.
- Warm → Cold: se re-comprime con Zstd-19.
- Cold → Archive: se re-comprime con Zstd-22, se genera índice `.index`.

**Restauración asíncrona desde Archive**:
- `GET /v1/storage/{bucket}/{key}` sobre objeto Archive devuelve `202 Accepted`
  con `restore_job_id`.
- `GET /v1/storage/restore/{restore_job_id}` para consultar progreso.
- El objeto restaurado permanece en Hot durante `storage.restore_ttl_hours` (default 24h).

**Scrubbing periódico** (semanal, throttled al 10% del ancho de banda de disco):
- Verifica SHA-256 de cada chunk local.
- Si 3 fallos en 1 hora → sospecha del nodo + re-replicación.
- Métricas de scrubbing en Prometheus.

**Políticas por bucket** (configurables en subida):
- `X-All4One-Policy: auto` → el lifecycle engine gestiona replicas y tier.
- `X-All4One-Policy: manual` → tier fijo, sin intervención del lifecycle engine.
- `X-All4One-Policy: tiered` → transición por tiempo (hot_days, warm_days, then).

### Qué NO está incluido en Fase 3

- FUSE, LD_PRELOAD, SDK — Fase 4.
- Autenticación completa (Bearer token, OAuth2) — Fase 4.
- Android, GPU, multi-tenant — Fase 5.

---

## Estructura de carpetas Rust añadida en Fase 3

```
agent/src/
└── lifecycle/
    ├── mod.rs          # tarea tokio, ciclo de 6 horas
    ├── heat_score.rs   # cálculo de las 5 señales
    ├── transitions.rs  # lógica de transición de tier con periodo de gracia
    ├── policies.rs     # evaluación de Manual/Auto/Tiered/Pin
    └── migration.rs    # generación de MigrationJob para el scheduler
```

---

## Criterios de aceptación (Fase 3)

1. **Ciclo de lifecycle**: tras 6 horas sin accesos, un objeto subido como Hot
   pasa a Warm (si su heat_score cae por debajo de 0.8 durante 7 días).
   Verificable vía `HEAD /v1/storage/{bucket}/{key}` → `X-All4One-Tier: warm`.

2. **Transición hacia arriba inmediata**: acceder un objeto Cold repetidamente
   hasta que su heat_score supere 0.8. En el siguiente ciclo de 6 horas,
   el objeto migra a Hot sin periodo de gracia.

3. **Hint ARCHIVE_IMMEDIATELY**: subir objeto con `X-All4One-Access-Hint: archive_immediately`.
   En el siguiente ciclo de lifecycle (máximo 6 horas), el objeto está en Archive.

4. **Hint FREQUENT**: subir objeto con `X-All4One-Access-Hint: frequent`.
   Aunque no se acceda durante 30 días, permanece en Hot.

5. **Restauración desde Archive**: `GET` sobre objeto Archive devuelve `202 Accepted`.
   Polling en `/v1/storage/restore/{id}` hasta `status: ready`. El objeto es
   descargable desde `download_url`. Tiempo de restauración de 100 MB < 5 minutos.

6. **Pin**: objeto con `policy: manual, pin: true` no migra aunque su heat_score
   caiga a 0.0 durante múltiples ciclos.

7. **Métricas de lifecycle**:
   - `all4one_lifecycle_objects_by_tier` refleja correctamente la distribución.
   - `all4one_lifecycle_migrations_total` incrementa cuando ocurre una migración.

---

## Dependencias con fases anteriores

Fase 3 requiere Fase 2 completamente funcional:
- Raft activo (el lifecycle engine solo corre en el líder Raft).
- Storage distribuido con chunks y FileMetadata en Raft.
- Los campos `heat_score`, `last_accessed`, `access_count_7d`, `access_count_30d`,
  `unique_accessors_30d`, `last_tier_change` en `FileMetadata` deben estar
  siendo actualizados por el módulo storage en Fase 2.

**Actualización de métricas de acceso** (implementada en Fase 2, usada en Fase 3):
Cada `GET /v1/storage/{bucket}/{key}` exitoso aplica via Raft:
```rust
RaftCommand::UpdateAccessMetrics {
    file_id,
    accessor_id,   // hash de la IP cliente o api_key
    accessed_at: Utc::now(),
}
```

---

## Lista de tareas ordenadas

---

### Tarea 1 — Actualización de métricas de acceso en storage (prerequisito)

**Qué hacer**: en Fase 2 el módulo storage debe actualizar los campos
`last_accessed`, `access_count_7d`, `access_count_30d` y `unique_accessors_30d`
en `FileMetadata` cada vez que se sirve un `GET /v1/storage/{bucket}/{key}`.
Esto se hace via `RaftCommand::UpdateAccessMetrics`. Sin este paso, el lifecycle
engine no tiene datos sobre los que calcular el heat score.

**Test**:
```bash
# Subir un objeto y acceder a él 5 veces:
for i in $(seq 1 5); do
  curl -s -H "X-All4One-Secret: s" \
    http://nodo1:7946/v1/storage/test/objeto.bin -o /dev/null
done

# HEAD muestra access_count_7d actualizado:
curl -s -I -H "X-All4One-Secret: s" \
  http://nodo1:7946/v1/storage/test/objeto.bin | grep -i "x-all4one"
# X-All4One-Heat-Score: <valor > 0>
# (el heat score se recalcula en el ciclo de lifecycle,
#  pero los contadores deben estar en Raft ya)
```

---

### Tarea 2 — Módulo `lifecycle`: cálculo de heat score

**Qué hacer**: implementar `lifecycle/heat_score.rs` con la función
`calculate(metadata: &FileMetadata) -> f32` que aplica las 5 señales con sus
pesos. El resultado debe estar en `[0.0, 1.0]`.

**Test**:
```rust
// Unit tests en lifecycle/heat_score.rs
#[test]
fn test_heat_score_hot() {
    let metadata = FileMetadata {
        access_count_7d: 80,
        access_count_30d: 500,
        last_accessed: Utc::now() - Duration::hours(2),
        unique_accessors_30d: 30,
        // accesos distribuidos en las últimas 4 semanas
        ..Default::default()
    };
    let score = calculate(&metadata);
    assert!(score > 0.8, "score={}", score);
}

#[test]
fn test_heat_score_archive() {
    let metadata = FileMetadata {
        access_count_7d: 0,
        access_count_30d: 1,
        last_accessed: Utc::now() - Duration::days(60),
        unique_accessors_30d: 1,
        ..Default::default()
    };
    let score = calculate(&metadata);
    assert!(score < 0.1, "score={}", score);
}
```

---

### Tarea 3 — Módulo `lifecycle`: transiciones de tier con periodo de gracia

**Qué hacer**: implementar `lifecycle/transitions.rs`. La función
`evaluate_transition(metadata, new_score) -> Option<DataTier>` devuelve
el tier objetivo si debe cambiar, respetando el periodo de gracia de 7 días
para transiciones hacia abajo y sin periodo para las hacia arriba.

**Test**:
```rust
#[test]
fn test_no_transition_within_grace_period() {
    let metadata = FileMetadata {
        tier: DataTier::Hot,
        heat_score: 0.9,
        last_tier_change: Utc::now() - Duration::days(3), // solo 3 días
        ..Default::default()
    };
    // score cae a 0.5 (Warm), pero solo han pasado 3 días
    let result = evaluate_transition(&metadata, 0.5);
    assert_eq!(result, None); // sin transición — en periodo de gracia
}

#[test]
fn test_transition_after_grace_period() {
    let metadata = FileMetadata {
        tier: DataTier::Hot,
        heat_score: 0.9,
        last_tier_change: Utc::now() - Duration::days(10), // 10 días > 7
        ..Default::default()
    };
    let result = evaluate_transition(&metadata, 0.5);
    assert_eq!(result, Some(DataTier::Warm));
}

#[test]
fn test_upward_transition_immediate() {
    let metadata = FileMetadata {
        tier: DataTier::Cold,
        heat_score: 0.2,
        last_tier_change: Utc::now() - Duration::hours(1), // 1 hora
        ..Default::default()
    };
    // score sube a 0.9 (Hot) — transición inmediata sin importar tiempo
    let result = evaluate_transition(&metadata, 0.9);
    assert_eq!(result, Some(DataTier::Hot));
}
```

---

### Tarea 4 — Módulo `lifecycle`: ciclo de 6 horas en el líder Raft

**Qué hacer**: implementar `lifecycle/mod.rs` con la tarea tokio que corre
cada 6 horas. Solo activa si el nodo es líder Raft. Lee el BlockMap de Raft,
evalúa cada objeto y encola `MigrationJob` para los que deben cambiar de tier.

**Test**:
```bash
# Forzar un ciclo inmediato para pruebas (flag --run-lifecycle-now):
all4one-agent run-lifecycle --data-dir /var/lib/all4one

# Verificar que los objetos con heat_score bajo transicionaron (tras 7 días de gracia):
# En un entorno de test, mockear last_tier_change para que sea hace 10 días.
# Después del ciclo:
curl -s -I -H "X-All4One-Secret: s" \
  http://nodo1:7946/v1/storage/test/objeto-frio.bin | grep "X-All4One-Tier"
# X-All4One-Tier: warm   (si el score cayó a 0.4-0.8)
```

---

### Tarea 5 — Hints de acceso (READ_ONCE, FREQUENT, ARCHIVE_IMMEDIATELY)

**Qué hacer**: implementar el procesamiento de hints en `lifecycle/policies.rs`.
Los hints se leen de `FileMetadata.access_hint` y se aplican en el ciclo.
`ARCHIVE_IMMEDIATELY` encola `MigrationJob` sin periodo de gracia.

**Test**:
```bash
# Subir objeto con hint archive_immediately:
curl -s -X PUT http://nodo1:7946/v1/storage/test/objeto-archivar.bin \
  -H "Content-Type: application/octet-stream" \
  -H "X-All4One-Secret: s" \
  -H "X-All4One-Access-Hint: archive_immediately" \
  --data-binary @/tmp/test200mb.bin

# Tras el próximo ciclo (forzar con --run-lifecycle-now):
all4one-agent run-lifecycle --data-dir /var/lib/all4one
curl -s -I -H "X-All4One-Secret: s" \
  http://nodo1:7946/v1/storage/test/objeto-archivar.bin | grep "X-All4One-Tier"
# X-All4One-Tier: archive

# Subir objeto con hint frequent:
curl -s -X PUT http://nodo1:7946/v1/storage/test/objeto-frecuente.bin \
  -H "X-All4One-Access-Hint: frequent" \
  -H "X-All4One-Secret: s" \
  -H "Content-Type: application/octet-stream" \
  --data-binary @/tmp/test200mb.bin
# Sin accesos durante 30 días (mockear en test) → sigue en Hot.
```

---

### Tarea 6 — Restauración asíncrona desde Archive

**Qué hacer**: implementar `GET /v1/storage/{bucket}/{key}` para objetos en
Archive (devuelve 202 + `restore_job_id`). Implementar
`GET /v1/storage/restore/{restore_job_id}` para consultar progreso.
El lifecycle engine lanza el `RestoreJob` con prioridad Normal.

**Test**:
```bash
# Archivar un objeto primero (hint archive_immediately + ciclo forzado).
# Intentar descargarlo:
RESPONSE=$(curl -s -o - -w "\n%{http_code}" \
  -H "X-All4One-Secret: s" \
  http://nodo1:7946/v1/storage/test/objeto-archivar.bin)
HTTP_CODE=$(echo "$RESPONSE" | tail -1)
echo $HTTP_CODE   # 202

RESTORE_ID=$(echo "$RESPONSE" | head -1 | python3 -c "import sys,json; print(json.load(sys.stdin)['restore_job_id'])")

# Polling hasta que esté listo:
while true; do
  STATUS=$(curl -s -H "X-All4One-Secret: s" \
    http://nodo1:7946/v1/storage/restore/$RESTORE_ID | \
    python3 -c "import sys,json; print(json.load(sys.stdin)['status'])")
  echo "Estado: $STATUS"
  [ "$STATUS" = "ready" ] && break
  sleep 5
done

# Descargar desde la URL de restauración y verificar integridad:
DOWNLOAD_URL=$(curl -s -H "X-All4One-Secret: s" \
  http://nodo1:7946/v1/storage/restore/$RESTORE_ID | \
  python3 -c "import sys,json; print(json.load(sys.stdin)['download_url'])")
curl -s -H "X-All4One-Secret: s" $DOWNLOAD_URL -o /tmp/restaurado.bin
sha256sum /tmp/test200mb.bin /tmp/restaurado.bin
# Hashes iguales.
```

---

### Tarea 7 — Métricas de lifecycle en Prometheus

**Qué hacer**: añadir las métricas `all4one_lifecycle_objects_by_tier` y
`all4one_lifecycle_migrations_total` al endpoint `/metrics`.

**Test**:
```bash
curl -s http://nodo1:9090/metrics | grep all4one_lifecycle
# all4one_lifecycle_objects_by_tier{tier="hot"} N
# all4one_lifecycle_objects_by_tier{tier="warm"} N
# all4one_lifecycle_migrations_total{from="hot",to="warm"} N
# all4one_lifecycle_cycle_duration_seconds N

# Después de ejecutar un ciclo con transiciones:
# all4one_lifecycle_migrations_total incrementa correctamente.
```

---

### Tarea 8 — Prueba de integración final (Fase 3)

**Test**: ejecutar los 7 criterios de aceptación de la fase.
```bash
# 1. Objeto sin accesos 7+ días baja de Hot a Warm en el siguiente ciclo ✓
# 2. Objeto con score > 0.8 sube a Hot inmediatamente (sin periodo de gracia) ✓
# 3. archive_immediately: objeto en Archive en el siguiente ciclo ✓
# 4. frequent: objeto permanece Hot 30 días sin accesos ✓
# 5. Restauración desde Archive en < 5 min para 100 MB ✓
# 6. pin=true: objeto no migra aunque score=0.0 durante múltiples ciclos ✓
# 7. Métricas de lifecycle reflejan correctamente la distribución por tier ✓
```
