# ADR-006: PKI interna con mTLS para confianza entre nodos

**Estado**: Aceptado
**Fecha**: 2026-04-08

---

## Contexto

Los agentes de All4One se comunican entre sí via gRPC en la red local del cliente.
Cualquier proceso en esa red podría intentar conectarse al puerto 7947 y enviar
comandos (lanzar jobs, transferir chunks, votar en Raft). El sistema necesita
garantizar que solo los nodos legítimos del clúster de una organización
pueden comunicarse entre sí.

Además, en el modelo cloud (Fase 5), múltiples organizaciones comparten la misma
infraestructura. La separación entre organizaciones debe ser criptográficamente
sólida — no basar en la separación de red (que puede comprometerse).

---

## Decisión

**mTLS con PKI interna al clúster.** Cada clúster tiene su propia CA raíz (Ed25519).
Todos los nodos de la misma organización tienen certificados firmados por esa CA.
Cada conexión gRPC realiza mutual TLS — tanto el cliente como el servidor
presentan su certificado y verifican el del otro contra la CA compartida.

---

## Razones

### La identidad se demuestra criptográficamente en cada conexión

Con mTLS, la pregunta "¿es este nodo legítimo?" se responde en el handshake TLS,
antes de procesar ningún mensaje de la capa de aplicación. Si el certificado del
conectante:
- No está firmado por la CA de esta organización → conexión rechazada.
- Está en la CRL (revocado) → conexión rechazada.
- Ha expirado → conexión rechazada.

No hay código de autenticación en la capa de aplicación que se pueda bypassar
con un mensaje malformado o una secuencia de llamadas inesperada.

### Separación criptográfica entre organizaciones

Dos organizaciones tienen CAs distintas. Un nodo de la organización A no puede
conectarse al clúster de la organización B aunque estén en la misma red física.
El handshake TLS falla antes de que se procese ningún dato.

Esto es especialmente importante para el modelo cloud (Fase 5), donde nodos de
distintas organizaciones pueden estar en el mismo datacenter o incluso en el
mismo servidor físico (VMs).

### Revocación inmediata via CRL en Raft

La CRL (Certificate Revocation List) está replicada en el log Raft. Cuando el
administrador ejecuta `all4one-agent revoke --node NODE_ID`:

1. `RaftCommand::AddToCRL { node_id }` se aplica en el quórum.
2. Todos los nodos del clúster reciben la actualización via Raft replication.
3. En el siguiente handshake gRPC con el nodo revocado, `certificates::is_revoked()`
   devuelve `true` y la conexión se rechaza.

El tiempo desde la revocación hasta la efectividad es el tiempo de replicación
Raft (típicamente < 500 ms en una red LAN), no el TTL del certificado (90 días).

### Renovación automática elimina operaciones manuales

7 días antes de la expiración del certificado (TTL 90 días), el módulo
`certificates` detecta la proximidad y solicita un nuevo certificado al líder
Raft usando el certificado actual como autenticación. El operador no necesita
intervenir — el proceso es automático y transparente.

Si el certificado ya expiró, el nodo se desconecta del clúster y debe
re-enrolarse manualmente con token. Esto es el comportamiento correcto: un nodo
que lleva > 90 días sin conectarse (y por tanto no pudo renovar) necesita
verificación manual antes de readmitirse.

### Enrolamiento con token de un solo uso

El único punto de entrada para un nuevo nodo es el endpoint `AgentService.Join`,
que requiere un token de un solo uso con TTL de 1 hora. Esto garantiza que:

1. Solo el administrador puede añadir nodos (debe generar el token).
2. Un token robado tiene ventana de uso de 1 hora.
3. Un token capturado en tránsito no se puede reutilizar (un solo uso).

El rate limiting (5 intentos/IP/hora) en el endpoint `Join` previene fuerza bruta.

---

## Alternativas descartadas

### Shared secret por clúster (Fase 1 como solución definitiva)

**Descartado**: el shared secret es válido para modo desarrollo pero no escala
como mecanismo de producción por:

1. **Sin revocación individual**: si se compromete el secreto, hay que cambiarlo
   en todos los nodos simultáneamente (operación de mantenimiento compleja).

2. **Sin identidad de nodo**: cualquier proceso que conozca el secreto puede
   impersonar cualquier nodo. Con mTLS, cada nodo tiene una identidad única
   (su NodeId como Common Name en el certificado).

3. **Sin separación entre organizaciones**: si dos clústeres de organizaciones
   distintas usan el mismo secreto (por error de configuración), pueden
   comunicarse entre sí.

### TLS unidireccional (solo el servidor presenta certificado)

**Descartado**: TLS unidireccional verifica que el servidor es quien dice ser,
pero no verifica la identidad del cliente. Cualquier proceso en la red podría
conectarse al puerto 7947 y enviar mensajes gRPC. La seguridad dependería
únicamente de la lógica de autorización en la capa de aplicación (más atacable
que el rechazo en capa de transporte).

### OAuth2 / JWT tokens para inter-agente

**Descartado**: JWT requiere un servicio de emisión de tokens (Authorization Server)
que es un SPOF adicional. Los tokens JWT tienen TTL cortos que requieren
renovación frecuente con llamadas al Authorization Server. Con mTLS, no hay
servicio de autenticación externo — la CA es un par de ficheros en disco.

---

## Consecuencias aceptadas

### La clave privada de la CA es crítica

Si la clave privada de la CA (`{data_dir}/certs/ca.key`) se pierde o compromete:
- **Pérdida**: todos los nodos deben re-enrolarse con una nueva CA. El proceso
  requiere regenerar la CA en el primer nodo y enrolar todos los demás con tokens nuevos.
- **Compromiso**: un atacante con la clave privada de la CA puede crear
  certificados para cualquier nodo y unirse al clúster.

**Mitigaciones**:
- La clave privada de la CA debe residir en un nodo Tier 0 (siempre disponible,
  físicamente seguro).
- Permisos 0600 — solo el usuario del agente puede leerla.
- **Decisión pendiente**: backup cifrado de la CA (con qué mecanismo de cifrado,
  dónde almacenar el backup, proceso de restauración).

### Complejidad operativa al añadir nodos

Añadir un nodo requiere el proceso de enrolamiento con token (4 pasos). Es más
complejo que simplemente copiar un fichero de configuración con el secreto
compartido.

**Mitigación**: el proceso está completamente automatizado en el comando
`all4one-agent enroll` — el operador solo necesita copiar y pegar el token y
la IP del endpoint. Tiempo total del proceso: < 30 segundos.

### WinFsp LGPL en Windows

El módulo FUSE en Windows usa WinFsp con licencia LGPL. El enlace dinámico con
LGPL es generalmente compatible con distribución propietaria, pero verificar
con asesor legal antes de distribución en entornos con restricciones legales
específicas. Ver [Fase 4](../phases/phase-4.md).
