# Scheduler y algoritmo de placement

El módulo scheduler recibe jobs via la API REST y decide en qué nodo ejecutarlos.
No existe un scheduler centralizado: cada nodo con `roles.scheduler = true` puede
recibir y colocar jobs de forma independiente.

---

## Estructuras de datos

```rust
// scheduler/queue.rs

/// Cola de jobs pendientes ordenada por prioridad.
/// BTreeMap garantiza que High se sirve antes que Normal, y Normal antes que Low.
pub struct JobQueue {
    inner: BTreeMap<Priority, VecDeque<PendingJob>>,
    mutex: Arc<Mutex<()>>,
}

pub struct PendingJob {
    pub spec:       JobSpec,
    pub queued_at:  DateTime<Utc>,
    pub attempts:   u32,
    pub next_retry: Option<DateTime<Utc>>,
}

#[derive(Ord, PartialOrd, Eq, PartialEq, Clone, Debug, Serialize, Deserialize)]
pub enum Priority {
    Low    = 0,
    Normal = 1,
    High   = 2,
}
```

---

## Algoritmo de placement

### Entrada

Un `JobSpec` recibido via `POST /v1/jobs`.

### Paso 1: snapshot del estado

```rust
let snapshot: ClusterState = cluster_state.read().await.clone();
```

Se trabaja sobre una copia inmutable para que los pasos de filtrado y puntuación
sean consistentes y no bloqueen el lock durante la evaluación.

### Paso 2: filtrado de candidatos

Se aplican los filtros en este orden exacto. Un nodo que falle cualquier filtro
queda excluido:

```
candidatos = snapshot.nodes.values()

  .filter(|n| n.status == NodeStatus::Online)
  // Solo nodos en línea. SUSPECTED, OFFLINE y DRAINING se excluyen.

  .filter(|n| satisfies_capabilities(n.profile.capabilities, spec.constraints.requires_capabilities))
  // Docker disponible si spec.runtime == Docker
  // Python disponible con versión >= mínima si spec.runtime == Python
  // Java disponible con versión >= mínima si spec.runtime == Jar
  // WASM disponible si spec.runtime == Wasm
  // Plataforma ejecutable correcta si spec.runtime == Executable
  // GPU con CUDA >= cuda_min si spec.resources.gpu == true

  .filter(|n| n.resources.cpu_cores_available >= spec.resources.cpu_cores)
  // Recursos CPU disponibles en este momento (actualizado via gossip heartbeat)

  .filter(|n| n.resources.ram_mb_available >= spec.resources.memory_mb)
  // Recursos RAM disponibles

  .filter(|n| n.profile.tier as u8 >= spec.constraints.tier_min as u8)
  // tier_min: 0 → cualquier tier, 1 → Tier 1 o Tier 0, 2 → solo Tier 0 no existe
  // Nota: tier_min=0 en JobConstraints significa "cualquier tier es válido"

  .filter(|n| within_availability_window(n))
  // Tier 0 (Always): siempre pasa
  // Tier 1 (Cron "0 9-18 * * 1-5"): pasa si la hora actual está dentro de la ventana
  // Tier 2 (Manual): pasa si está Online (ya fue verificado arriba)
  // Tier 1/2 (Learned, Fase 5): pasa si el modelo predice disponibilidad

  .filter(|n| if let Some(forced_id) = spec.constraints.node_id {
      n.id == forced_id
  } else {
      true
  })
  // Forzado a nodo específico (solo para debug)

  .filter(|n| if spec.constraints.same_network {
      same_subnet(n.address, client_source_addr)
  } else {
      true
  })
  // Si same_network=true, el nodo debe estar en la misma subred /24 que el cliente
  // que hizo el POST /v1/jobs

  .filter(|n| if let Some(min_gbps) = spec.constraints.network_min_gbps {
      n.profile.resources.network_bandwidth_mbps >= min_gbps * 1000
  } else {
      true
  })
  // Ancho de banda mínimo requerido (relevante para inference_group)

  .collect::<Vec<_>>()
```

### Paso 3: cola si no hay candidatos

```rust
if candidatos.is_empty() {
    job_queue.push(PendingJob { spec, queued_at: Utc::now(), attempts: 0, next_retry: None });
    return Ok(JobStatus {
        job_id: spec.id.unwrap_or_else(Uuid::new_v4).into(),
        status: JobState::Queued,
        assigned_to: None,
        created_at: Utc::now(),
        // resto de campos None
        ..Default::default()
    });
}
```

### Paso 4: puntuación de candidatos

```rust
fn score_node(node: &NodeInfo, spec: &JobSpec, cluster_chunk_map: &BlockMap) -> f32 {
    // locality: qué porcentaje de los chunks requeridos por el job
    // están almacenados localmente en este nodo.
    // Si el job no tiene DataMounts, locality = 0.0 para todos.
    let total_chunks: usize = spec.data.iter()
        .map(|mount| resolve_chunk_count(mount, cluster_chunk_map))
        .sum();
    let local_chunks: usize = spec.data.iter()
        .map(|mount| count_local_chunks(mount, node.id, cluster_chunk_map))
        .sum();
    let locality = if total_chunks == 0 {
        0.0f32
    } else {
        local_chunks as f32 / total_chunks as f32
    };

    // ventana: cuánto tiempo de disponibilidad queda vs. duración estimada del job.
    // Para Tier 0 (Always) → ventana_restante = f32::MAX → ratio = 1.0
    let duracion_estimada = spec.resources.max_duration_minutes
        .unwrap_or(60) as f32;  // default 60 min si no se especifica
    let ventana_restante = remaining_window_minutes(node);
    let ventana = (ventana_restante / duracion_estimada).min(1.0f32);

    // recursos: media aritmética de fracción libre de CPU y RAM
    let cpu_ratio = node.resources.cpu_cores_available as f32
        / node.resources.cpu_cores_total.max(1) as f32;
    let ram_ratio = node.resources.ram_mb_available as f32
        / node.resources.ram_mb_total.max(1) as f32;
    let recursos = (cpu_ratio + ram_ratio) / 2.0;

    // tier: preferencia por Tier 0 > Tier 1 > Tier 2
    let tier = (2 - node.profile.tier as u8) as f32 / 2.0;

    // score final ponderado
    locality  * 0.40
    + ventana * 0.30
    + recursos * 0.20
    + tier     * 0.10
}
```

### Paso 5: selección del nodo

```rust
let nodo_elegido = candidatos
    .iter()
    .max_by(|a, b| {
        let score_a = score_node(a, &spec, &block_map);
        let score_b = score_node(b, &spec, &block_map);
        score_a.partial_cmp(&score_b).unwrap_or(Ordering::Equal)
    })
    .expect("candidatos no está vacío — verificado en Paso 3");
```

### Paso 6: registro en Raft (Fase 2+)

```rust
// Solo si Raft está activo. En Fase 1 se omite este paso.
if let Some(raft) = &self.raft {
    let job_status = JobStatus {
        job_id: spec.id.into(),
        status: JobState::Scheduled,
        assigned_to: Some(nodo_elegido.id),
        created_at: Utc::now(),
        ..Default::default()
    };
    raft.apply_command(RaftCommand::RegisterJob { job: job_status }).await?;
}
```

Si el nodo actual no es el líder Raft, `apply_command` devuelve
`Err(RaftError::NotLeader { leader_id })` y el handler REST debe redirigir
la petición al líder con `307 Temporary Redirect`.

### Paso 7: lanzamiento

```rust
if nodo_elegido.id == self.node_id {
    // Lanzar localmente
    self.executor.launch(spec).await?;
} else {
    // Delegar al nodo remoto via gRPC
    let stream = self.grpc_client
        .launch_job(nodo_elegido.id, LaunchJobRequest { spec_json: serde_json::to_vec(&spec)? })
        .await?;
    // Suscribirse al stream de JobEvents y propagar via gossip
    tokio::spawn(relay_job_events(stream, self.gossip_tx.clone()));
}
```

### Paso 8: respuesta

```rust
Ok(JobStatus {
    job_id: spec.id.into(),
    status: JobState::Scheduled,
    assigned_to: Some(nodo_elegido.id),
    created_at: Utc::now(),
    ..Default::default()
})
```

---

## Reevaluación de la cola

La `JobQueue` se reevalúa en tres situaciones, todas gestionadas por una tarea
`tokio` que escucha el canal `tokio::broadcast`:

```rust
// scheduler/mod.rs — tarea de reevaluación

async fn queue_evaluator(
    mut membership_rx: broadcast::Receiver<MembershipEvent>,
    mut job_event_rx: broadcast::Receiver<JobEvent>,
    queue: Arc<Mutex<JobQueue>>,
    cluster_state: SharedClusterState,
) {
    loop {
        tokio::select! {
            Ok(event) = membership_rx.recv() => {
                match event {
                    MembershipEvent::NodeJoined(_) => {
                        // Un nodo nuevo puede tener capabilities que antes
                        // no existían en el clúster → reevaluar TODOS los jobs en cola
                        try_drain_queue(&queue, &cluster_state).await;
                    }
                    MembershipEvent::NodeUpdated(_, _) => {
                        // Un nodo liberó recursos → puede haber jobs que ahora caben
                        try_drain_queue(&queue, &cluster_state).await;
                    }
                    _ => {}
                }
            }
            Ok(event) = job_event_rx.recv() => {
                if let JobEventKind::Completed { .. } | JobEventKind::Failed { .. } = event.event {
                    // Un job terminó → recursos liberados en el nodo asignado
                    try_drain_queue(&queue, &cluster_state).await;
                }
            }
        }
    }
}
```

---

## Política de reintentos

```rust
// El job se reintenta según su RetryPolicy cuando falla con JobFailed o JobLost.
// NO se reintenta cuando el proceso termina con exit_code != 0 (eso es un Completed).
// JobLost ocurre cuando el nodo ejecutor cae mientras el job está Running.

pub struct RetryPolicy {
    pub max_attempts:   u32,   // default: 1 (sin reintentos)
    pub idempotent:     bool,  // si false, no se reintenta en caso de Lost (datos parciales)
    pub backoff_seconds: u32,  // espera entre reintentos, default: 30
}

fn should_retry(status: &JobStatus, policy: &RetryPolicy) -> bool {
    if status.attempts >= policy.max_attempts {
        return false;
    }
    match status.status {
        JobState::Failed => true,
        JobState::Lost   => policy.idempotent,
        _                => false,
    }
}
```

---

## Afinidad y anti-afinidad

```yaml
# Dos jobs en el mismo nodo (afinidad)
constraints:
  with_job: "a1b2c3d4-e5f6-7890-abcd-ef1234567890"

# Dos jobs en nodos distintos (anti-afinidad)
constraints:
  not_with_job: "a1b2c3d4-e5f6-7890-abcd-ef1234567890"
```

La afinidad se implementa añadiendo un filtro adicional en el Paso 2:

```rust
// Con afinidad: busca el nodo donde ya corre with_job y lo fuerza como único candidato
if let Some(ref_job_id) = spec.constraints.with_job {
    if let Some(ref_status) = job_registry.get(&ref_job_id) {
        if let Some(target_node) = ref_status.assigned_to {
            candidatos.retain(|n| n.id == target_node);
        }
    }
}

// Con anti-afinidad: excluye el nodo donde ya corre not_with_job
if let Some(ref_job_id) = spec.constraints.not_with_job {
    if let Some(ref_status) = job_registry.get(&ref_job_id) {
        if let Some(excluded_node) = ref_status.assigned_to {
            candidatos.retain(|n| n.id != excluded_node);
        }
    }
}
```

---

## Cancelación de un job

```
DELETE /v1/jobs/{job_id}
    │
    ├── Si job está en Queued: lo elimina de la JobQueue → Cancelled
    ├── Si job está en Scheduled o Running:
    │     Si nodo_asignado == self → executor.kill(handle) → SIGTERM → 30s → SIGKILL
    │     Si nodo_asignado != self → grpc_client.cancel_job(nodo_asignado, job_id)
    └── Si job está en Completed/Failed/Cancelled → 409 JOB_ALREADY_FINISHED
```

---

## Race condition en Fase 1

Sin Raft, si el mismo job (con el mismo `id`) llega simultáneamente a dos nodos
schedulers, ambos pueden colocarlo y lanzarlo en nodos distintos.

**Mitigación en Fase 1**: los jobs deben ser idempotentes. Si el `id` ya existe
en el estado local del scheduler, se devuelve el estado actual sin relanzar (200 OK).

**Solución definitiva**: Raft en Fase 2. `RegisterJob` es un comando Raft que solo
tiene éxito una vez — el segundo intento con el mismo `job_id` recibe un error
de clave duplicada del store Raft.

---

## Diagrama de estados del job

```
              submit
                │
                ▼
           ┌─────────┐
           │  QUEUED  │◄──────────────────────────────────┐
           └────┬─────┘  no hay candidatos                │
                │                                         │
    candidatos disponibles                                │
                │                                         │
                ▼                                         │
         ┌───────────┐                                    │
         │ SCHEDULED │                                    │
         └─────┬─────┘                                    │
               │                                          │
     executor confirma arranque                           │
               │                                          │
               ▼                                          │
         ┌─────────┐                                      │
         │ RUNNING  │──── nodo cae ──► LOST ──► retry? ──┘
         └─────┬────┘
               │
    ┌──────────┼──────────────┐
    │          │              │
exit_code=0  exit_code≠0   timeout
    │          │              │
    ▼          ▼              ▼
COMPLETED   COMPLETED     TIMEOUT
            (no es Failed
             — el proceso
             terminó bien)

  DELETE /v1/jobs/{id}
    │
    ▼
CANCELLED
```

> `exit_code != 0` es un `Completed` con código de salida no nulo, no un `Failed`.
> `Failed` indica que el runtime no pudo lanzar o gestionar el proceso (ej. imagen
> Docker no encontrada, OOM killer, etc.).
