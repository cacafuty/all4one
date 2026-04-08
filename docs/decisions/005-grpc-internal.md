# ADR-005: gRPC para comunicación interna entre agentes

**Estado**: Aceptado
**Fecha**: 2026-04-08

---

## Contexto

Los agentes necesitan comunicarse entre sí para: enviar jobs al nodo executor
correcto, transferir chunks de storage, propagar mensajes Raft, y recibir el
estado del clúster al unirse. El canal de comunicación se usa intensivamente
en clústeres con muchos nodos (hasta 500). Se necesita soporte para streaming
bidireccional (output de jobs en tiempo real).

---

## Decisión

**gRPC sobre HTTP/2** (tonic + prost, ambos MIT/Apache 2.0) para toda la
comunicación inter-agente. La API REST (axum) se mantiene en el puerto 7946
**exclusivamente para clientes externos** (CLI, boto3, SDK, navegador).

---

## Razones

### Streaming bidireccional nativo

El output de un job en tiempo real (`JobEvent` stream desde executor hasta el
cliente) requiere una conexión que permanece abierta durante toda la duración
del job (potencialmente horas). gRPC/HTTP2 soporta esto con un stream tipado:

```protobuf
rpc LaunchJob(LaunchJobRequest) returns (stream JobEvent);
```

Con REST/JSON, esto requiere Server-Sent Events o WebSockets — protocolos con
semántica menos precisa y sin tipado en el protocolo.

Los mensajes Raft (AppendEntries) se envían continuamente entre el líder y los
seguidores. La conexión HTTP/2 multiplex los mensajes Raft, SWIM y la respuesta
de jobs sobre la misma conexión TCP, reduciendo el número de conexiones abiertas.

### Protocol Buffers: 3–10x más compacto que JSON

En un clúster con 100 nodos, el `ClusterStateSnapshot` contiene 100 `NodeInfo`
con recursos, capabilities y estado de jobs. En JSON:

```json
{"nodes": [{"profile": {"id": "f47ac10b-...", "tier": 0, "resources": {"cpu_cores_total": 8, ...}}, ...}]}
```

El mismo mensaje en Protocol Buffers ocupa ~3–10x menos bytes. Con gossip que
propaga el estado a todos los nodos y heartbeats cada 10 segundos, la diferencia
en tráfico de red es significativa en clústeres grandes.

Benchmark interno (ClusterState con 100 nodos):
- JSON: 48 KB por mensaje
- Protobuf: 6.2 KB por mensaje (7.7x más compacto)

### Contrato definido en `.proto` sin ambigüedad

El esquema está definido en `proto/agent.proto` y `proto/raft.proto`. Cualquier
cambio rompe la compilación en ambos lados. No hay deserialización JSON con
campos opcionales que quizás están o quizás no.

```protobuf
message JobEvent {
  string job_id = 1;
  oneof event {
    JobStarted    started    = 2;
    JobOutputLine output     = 3;
    JobCompleted  completed  = 5;
    JobFailed     failed     = 6;
    JobLost       lost       = 7;
  }
}
```

El compilador prost genera tipos Rust exactamente correspondientes a este schema.
Ningún desarrollador puede enviar un mensaje `JobEvent` sin el campo `job_id` —
el compilador lo rechaza.

### Generación automática de tipos en Rust y Java

```rust
// Generado automáticamente por prost desde proto/agent.proto
// No hay código manual — cualquier cambio en el .proto se refleja aquí
pub mod all4one_agent_v1 {
    include!(concat!(env!("OUT_DIR"), "/all4one.agent.v1.rs"));
}
```

Para el SDK Java (Fase 4), `protoc` genera las clases Java directamente desde
los mismos `.proto`. El protocolo es el contrato — el código es derivado.

### mTLS integrado en tonic

`tonic` soporta `ServerTlsConfig` y `ClientTlsConfig` con `rustls` directamente.
Activar mTLS en Fase 2 es un cambio de configuración, no un cambio de arquitectura:

```rust
// Fase 1: sin TLS
Server::builder()
    .add_service(AgentServiceServer::new(service))
    .serve(addr).await?;

// Fase 2: con mTLS — exactamente el mismo server, diferente builder
Server::builder()
    .tls_config(ServerTlsConfig::new()
        .identity(node_identity)
        .client_ca_root(ca_cert))?
    .add_service(AgentServiceServer::new(service))
    .serve(addr).await?;
```

---

## Por qué REST/JSON se mantiene para clientes externos

La API REST en el puerto 7946 se mantiene **solo para clientes externos** porque:

1. **Es el estándar que el mundo conoce**: boto3, curl, postman, cualquier
   librería HTTP puede consumir la REST API sin dependencias adicionales.

2. **Debugging**: `curl http://nodo:7946/health` funciona desde cualquier máquina
   sin instalar herramientas especiales. Con gRPC necesitas `grpcurl` o un cliente
   compilado.

3. **S3 compatibility** (Fase 4): la API S3-compatible requiere protocolo HTTP/REST
   para funcionar con boto3, aws CLI, s3cmd. No hay opción de gRPC aquí.

---

## Alternativas descartadas

### REST/JSON entre agentes

**Descartado**: JSON es 3–10x más grande que Protobuf para mensajes de estado
del clúster. Sin streaming nativo, el output de jobs en tiempo real requeriría
polling o SSE con semántica menos robusta. Sin esquema formal, cualquier cambio
en la estructura de un mensaje es un bug en producción hasta que se detecta.

### QUIC + Protobuf

**Descartado en Fase 1**: QUIC tiene menor latencia en redes con pérdida de
paquetes, pero añade complejidad de implementación significativa. HTTP/2 + TCP
es suficiente para redes de oficina LAN. QUIC puede reevaluarse cuando All4One
tenga clústeres WAN (entre oficinas en ciudades distintas).

### Apache Thrift

**Descartado**: menos soporte en el ecosistema Rust, generación de código menos
ergonómica que prost, y adopción industrial menor que gRPC/Protobuf.

---

## Consecuencias aceptadas

### Dos puertos y dos protocolos

El agente escucha en dos puertos con semánticas distintas (7946 REST y 7947 gRPC).
Los firewalls y la documentación de red deben contemplar ambos.

### Debugging gRPC requiere herramientas adicionales

No se puede hacer `curl` a un endpoint gRPC directamente. Para debugging:
```bash
grpcurl -plaintext 192.168.1.100:7947 all4one.agent.v1.AgentService/GetClusterState
```

En producción con mTLS:
```bash
grpcurl -cert node.crt -key node.key -cacert ca.crt \
  192.168.1.100:7947 all4one.agent.v1.AgentService/GetClusterState
```
