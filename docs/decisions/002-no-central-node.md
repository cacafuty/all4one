# ADR-002: Sin nodo central de orquestación

**Estado**: Aceptado
**Fecha**: 2026-04-08

---

## Contexto

All4One se vende a empresas con hardware existente heterogéneo — desde Raspberry
Pi hasta servidores rack. El cliente típico no quiere designar ni mantener un
"servidor maestro" dedicado. Muchos clientes tienen hardware con disponibilidad
temporal (portátiles que se apagan, PCs de oficina fuera del horario laboral).

La pregunta de diseño fundamental: ¿quién coordina el clúster?

---

## Decisión

**No existe un nodo obligatorio de orquestación.** Cualquier nodo puede recibir
jobs y coordinar. El clúster funciona con lo que haya disponible en cada momento.
El consenso (Raft) es embebido en cada agente y los nodos que participan en quórum
son los que tengan `quorum_participant = true` — no hay un nodo especial para ello.

---

## Razones

### Clúster mínimo funcional de un solo nodo

Con un nodo central obligatorio, el cliente necesita al menos dos máquinas:
el maestro y un worker. Sin nodo central, un desarrollador puede instalar el
agente en su portátil y tener un clúster funcional para desarrollo local.

En ventas: el demo de "instala esto y en 5 minutos tienes tu clúster" solo es
posible sin nodo central.

### Ausencia de SPOF (Single Point of Failure)

Si el nodo central cae, el sistema entero se detiene. En el modelo distribuido:
- Mientras haya quórum (mayoría de nodos `quorum_participant=true` online),
  el clúster opera normalmente.
- Si se pierde el quórum, el clúster pasa a modo degradado (sin escrituras en
  Raft, pero los jobs ya en ejecución continúan y los schedulers sin Raft
  siguen colocando jobs best-effort).

### La disponibilidad temporal de nodos es un parámetro de diseño

Los nodos Tier 1 y Tier 2 tienen disponibilidad parcial por diseño. El modelo
con nodo central tendría que tratar la ausencia de nodos como casos de error
a gestionar. En el modelo distribuido, la disponibilidad temporal es una
característica que el scheduler conoce y sobre la que planifica:

```
availability = "cron:0 9-18 * * 1-5"
```

El scheduler filtra este nodo fuera de la ventana 18:00–09:00 en lugar de
intentar conectar a un nodo que sabe que no está disponible.

---

## Alternativas descartadas

### Arquitectura maestro-worker (Kubernetes-style)

**Descartada por**:

1. **Dependencia de infraestructura**: el maestro debe ser un servidor siempre
   disponible. Muchos clientes target de Starter/Business no tienen ese servidor
   o no quieren gestionarlo.

2. **SPOF**: la caída del maestro detiene el scheduling. En K8s, el control plane
   es el punto crítico — alta disponibilidad del maestro requiere 3+ nodos
   dedicados (etcd cluster + API server redundante).

3. **Overhead operativo**: los clientes compran All4One para no tener que gestionar
   infraestructura. Un maestro separado es infraestructura a gestionar.

### Coordinador externo (ZooKeeper / etcd)

**Descartado por**:

1. **Dependencia externa**: requiere instalar y mantener ZooKeeper o etcd
   por separado. Contradice el principio de "un único binario sin dependencias".

2. **Licencia etcd**: etcd usa Apache 2.0, pero requiere instalación y gestión
   separada. Añade una pieza de infraestructura que puede fallar independientemente.

3. **Latencia de coordinación**: cada operación que requiera consenso (RegisterJob,
   PutChunkMap) necesita una llamada de red extra al coordinador externo. Con Raft
   embebido, la escritura en el log es local al nodo líder.

---

## Consecuencias aceptadas

### Mayor complejidad en cada agente

Cada agente implementa scheduling, gossip y consenso. Esto hace el agente más
complejo que un simple worker que solo ejecuta órdenes del maestro.

**Mitigación**: los módulos son independientes con interfaces bien definidas.
Un nodo Android solo activa `storage` — no implementa scheduler ni Raft.

### Debugging más complejo

Sin logs centralizados, correlacionar eventos entre nodos requiere:
- IDs de request consistentes (`X-Request-Id` en REST, `correlation_id` en gRPC).
- Timestamps sincronizados (NTP obligatorio en producción).
- Agregación de logs externos (Loki, Elasticsearch) para producción.

**Mitigación**: tracing estructurado en JSON con `request_id` permite correlación
post-hoc. La UI web de administración (Fase 5) agrega métricas de todos los nodos.

### Race condition de scheduling en Fase 1

Sin Raft, si el mismo job con `id` explícito llega a dos schedulers
simultáneamente, puede ejecutarse dos veces.

**Mitigación en Fase 1**: los jobs deben ser idempotentes. Si el `id` ya existe
en el estado local, se devuelve el estado actual sin relanzar.

**Solución definitiva en Fase 2**: `RaftCommand::RegisterJob` garantiza que el
job se registra exactamente una vez en el quórum. El segundo intento recibe
un error de clave duplicada.
