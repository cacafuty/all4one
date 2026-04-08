# Visión general de la arquitectura

## Premisa fundamental

All4One convierte hardware heterogéneo existente en un clúster unificado de cómputo
y almacenamiento. El **principio central** es que no existe un nodo obligatorio de
orquestación: cualquier nodo puede recibir jobs y coordinar, y el clúster funciona
con lo que haya disponible en cada momento.

Esta premisa tiene consecuencias directas en cada decisión de diseño:

- El agente es un **único binario** sin runtime externo.
- El consenso es **embebido** (openraft), no una dependencia externa (etcd).
- El descubrimiento es **mDNS + seeds**, sin servidor de nombres centralizado.
- El scheduling es **distribuido**: el primer nodo que recibe el job lo coloca.

---

## Componente único: el agente

El sistema completo se reduce a **un único binario Rust** instalado en cada
dispositivo. No hay coordinadores separados, no hay metaservidores, no hay proxies.

```
┌─────────────────────────────────────────────────────────────────────┐
│                         AGENTE all4one                              │
│                                                                     │
│  ┌──────────┐  ┌──────────┐  ┌────────────────────────────────┐   │
│  │  config  │  │   node   │  │         discovery              │   │
│  │ (toml)   │  │ (uuid)   │  │   mdns ◄──────────► seeds      │   │
│  └──────────┘  └──────────┘  └────────────────────────────────┘   │
│                                                                     │
│  ┌──────────────────────────────────────────────────────────────┐  │
│  │                    gossip (SWIM/UDP:7947)                     │  │
│  │          ClusterState: HashMap<NodeId, NodeInfo>              │  │
│  │          tokio::broadcast::Sender<MembershipEvent>            │  │
│  └──────────────────────────────────────────────────────────────┘  │
│                                                                     │
│  ┌──────────────────────────────────────────────────────────────┐  │
│  │                  raft (openraft, Fase 2+)                     │  │
│  │    BlockMap │ JobRegistry │ ClusterConfig │ CRL               │  │
│  └──────────────────────────────────────────────────────────────┘  │
│                                                                     │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────────┐ │
│  │  scheduler   │  │   executor   │  │  storage (Fase 2+)        │ │
│  │  JobQueue    │  │  docker.rs   │  │  chunks + index           │ │
│  │  placement   │  │  jar.rs      │  │  SHA-256 + erasure        │ │
│  │  algorithm   │  │  python.rs   │  │  scrubbing                │ │
│  │              │  │  wasm.rs     │  │                            │ │
│  └──────────────┘  └──────────────┘  └──────────────────────────┘ │
│                                                                     │
│  ┌──────────────────────────────────────────────────────────────┐  │
│  │            lifecycle (Fase 3+, solo líder Raft)               │  │
│  │            heat score + transiciones de tier                  │  │
│  └──────────────────────────────────────────────────────────────┘  │
│                                                                     │
│  ┌─────────────────────────┐  ┌───────────────────────────────┐   │
│  │  api_rest (axum :7946)  │  │  grpc_server (tonic :7947)    │   │
│  │  grpc_client (pool)     │  │  certificates (Fase 2+)        │   │
│  └─────────────────────────┘  └───────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Tiers de nodos

Los tiers no son niveles de calidad sino **patrones de disponibilidad temporal**.
El scheduler es consciente de estas ventanas y planifica en consecuencia.

```
Tier 0 — 24/7, columna vertebral
  ┌────────────────────────────────────────────────────────────────┐
  │  Servidores rack │ NAS │ Raspberry Pi dedicado                 │
  │  • Metadata crítica SIEMPRE aquí                               │
  │  • Al menos una réplica de cada dato aquí                      │
  │  • Participan en quórum Raft                                   │
  │  • Nunca Tier 2 para réplicas que estén solo en Tier 0         │
  └────────────────────────────────────────────────────────────────┘

Tier 1 — disponibilidad predecible con horario (cron)
  ┌────────────────────────────────────────────────────────────────┐
  │  PCs de oficina │ portátiles de empresa                        │
  │  • El scheduler planifica dentro de su ventana                 │
  │  • Participan en quórum Raft                                   │
  │  • Pueden hospedar réplicas secundarias                        │
  └────────────────────────────────────────────────────────────────┘

Tier 2 — oportunista, sin garantías
  ┌────────────────────────────────────────────────────────────────┐
  │  Portátiles personales │ móviles Android                       │
  │  • Solo cómputo efímero y storage no crítico                   │
  │  • NO participan en quórum Raft                                │
  │  • DrainNotice automático al 20% batería (Android)             │
  └────────────────────────────────────────────────────────────────┘
```

---

## Roles del agente

Cada agente puede ejercer hasta tres roles simultáneamente, activados
independientemente en `agent.toml`:

| Rol       | Activado con          | Responsabilidad                                     |
|-----------|-----------------------|-----------------------------------------------------|
| SCHEDULER | `roles.scheduler=true` | Recibe jobs via REST, ejecuta el algoritmo de placement, delega via gRPC |
| EXECUTOR  | `roles.executor=true` | Ejecuta jobs asignados. Gestiona cgroups, stdout/stderr, ciclo de vida |
| STORAGE   | `roles.storage=true`  | Almacena chunks locales. Sirve lecturas. Participa en réplicas/erasure |

Un nodo Tier 0 típico tiene los tres roles activos. Un nodo Android solo tiene
STORAGE activo.

---

## Flujo de un job de extremo a extremo

```
Cliente (curl / SDK / boto3)
         │
         │  POST /v1/jobs  (YAML/JSON)
         ▼
  ┌─────────────┐
  │  api_rest   │  valida JobSpec, genera JobId, pasa a scheduler
  └──────┬──────┘
         │
         ▼
  ┌─────────────┐
  │  scheduler  │  snapshot de ClusterState
  │             │  filtra candidatos por capabilities/recursos/tier/ventana
  │             │  puntúa por locality/ventana/recursos/tier
  │             │  elige nodo_elegido
  └──────┬──────┘
         │
    ┌────┴─────────────────────────────┐
    │ nodo_elegido == self?             │
    Yes                                No
    │                                  │
    ▼                                  ▼
 executor.launch()          grpc_client.launch_job(nodo_elegido, ...)
    │                                  │
    ▼                                  ▼
 docker/jar/python/          nodo remoto → executor.launch()
 wasm/executable
    │
    ▼
 JobEvent stream (Started → OutputLine* → Completed|Failed)
    │
    ▼
 gossip propaga estado al clúster
```

---

## Flujo de almacenamiento (Fase 2+)

```
Cliente
  │  PUT /v1/storage/bucket/key  (bytes)
  ▼
api_rest → storage module
  │
  ├── chunking (default 64MB por chunk)
  ├── SHA-256 por chunk
  ├── compresión según tier (zstd)
  ├── erasure coding (Reed-Solomon según tier)
  │
  ▼
scheduler de chunks:
  ├── placement rules (nunca todas las réplicas en el mismo tier)
  ├── consistent hashing como base
  └── preferir nodos con mayor ventana restante
  │
  ▼
grpc_client.transfer_chunk() → nodos destino
  │
  ▼
Raft.apply(PutChunkMap) → BlockMap replicado en quórum
  │
  ▼
200 OK { etag, tier, replicas, ... }
```

---

## Descubrimiento y membresía

```
Arranque del agente
        │
        ├──► mdns: anuncia _all4one._tcp.local
        │         escucha anuncios de otros nodos
        │         al descubrir → notifica gossip via mpsc
        │
        └──► seeds: conecta a IPs fijas de la config
                    solicita GetClusterState via gRPC
                    notifica gossip via mpsc

gossip (SWIM):
  • Heartbeat UDP cada 10 segundos
  • Indirect probing con K=3 nodos si no responde directo
  • SUSPECTED tras 30s sin respuesta
  • OFFLINE tras 60s en SUSPECTED
  • Piggybacking: recursos actuales en cada heartbeat
  • Publica MembershipEvent via tokio::broadcast
    └──► scheduler suscrito: reintenta jobs en cola
    └──► storage suscrito: re-replica chunks de nodos caídos
```

---

## Seguridad por fase

```
Fase 1 (dev)
  • mode = "dev" en agent.toml
  • shared_secret en header X-All4One-Secret (REST) y metadata gRPC
  • Sin TLS — comunicaciones en texto plano
  • Aviso explícito en arranque

Fase 2+ (producción)
  • CA Ed25519 interna del clúster
  • mTLS en todas las conexiones gRPC entre agentes
  • Enrolamiento con token de un solo uso (TTL 1 hora)
  • CRL replicada en Raft — revocación inmediata
  • Renovación automática de certificados (7 días antes de expirar)
  • Cifrado en reposo opcional (AES-256-GCM con HKDF del certificado)
```

---

## Límites operativos clave

| Parámetro                            | Valor              |
|--------------------------------------|--------------------|
| Timeout conexión entre nodos         | 5 segundos         |
| Timeout respuesta LaunchJob          | 10 segundos        |
| Timeout transferencia chunk (64MB)   | 60 segundos        |
| Heartbeat SWIM                       | cada 10 segundos   |
| SUSPECTED tras                       | 30 segundos        |
| OFFLINE tras                         | 60 segundos en SUSPECTED |
| Output máximo capturado por job      | 10 MB (truncado)   |
| Tamaño máximo JobSpec YAML           | 1 MB               |
| Nodos máximos en ClusterState        | 500                |
| Tokens de enrolamiento simultáneos   | 100                |
| Rate limit endpoint Join             | 5 intentos/IP/hora |
| Tamaño de chunk default              | 64 MB              |
| Tamaño de chunk mínimo               | 1 MB               |
| Tamaño de chunk máximo               | 512 MB             |

---

## Referencias

- [Módulos del agente en detalle](agent.md)
- [Protocolos de red y puertos](networking.md)
- [Algoritmo de scheduling](scheduler.md)
- [Almacenamiento distribuido](storage.md)
- [Lifecycle engine](lifecycle.md)
- [IA e inferencia](ai-inference.md)
