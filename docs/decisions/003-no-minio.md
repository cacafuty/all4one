# ADR-003: Sin MinIO

**Estado**: Aceptado
**Fecha**: 2026-04-08

---

## Contexto

El sistema necesita almacenamiento distribuido de objetos con chunking, replicación
y erasure coding. MinIO es la solución open-source más popular para este caso.

---

## Decisión

**MinIO está descartado.** All4One implementa su propio módulo de storage sobre
`reed-solomon` + `zstd` + `sled`, integrado directamente en el agente.

---

## Razones

### Licencia AGPL v3 incompatible con redistribución propietaria

MinIO usa la licencia AGPL v3 (GNU Affero General Public License v3). AGPL v3
requiere que cualquier software que incorpore código AGPL, o que interaccione con
él como servicio de red, distribuya su código fuente completo bajo AGPL v3.

All4One es software propietario vendido con suscripción. Incorporar MinIO o
distribuirlo como parte del producto requeriría:
- Publicar todo el código fuente de All4One bajo AGPL v3 (eliminando la
  posibilidad de modelo de negocio propietario), o
- Comprar una licencia comercial a MinIO, Inc. (coste por nodo que impacta
  el modelo de precios de All4One).

Ambas opciones son incompatibles con el modelo de negocio actual.

### Modelo de datos sin awareness de tiers de disponibilidad temporal

MinIO trata todos los nodos de forma equivalente. No tiene concepto de:
- Nodos con ventanas de disponibilidad (Tier 1: 9:00–18:00 lunes-viernes).
- DrainNotice anticipado con migración de chunks en riesgo.
- Placement preferencial en nodos con mayor ventana de disponibilidad restante.

Añadir estas características a MinIO requeriría un fork profundo, lo que
multiplica el mantenimiento a largo plazo.

### Sin integración con el scheduler de All4One

El módulo storage de All4One necesita integrarse con el scheduler para:
- Informar al scheduler de qué chunks tiene cada nodo localmente (señal
  `locality` en el algoritmo de placement).
- Recibir eventos `NodeOffline` del módulo gossip para iniciar re-replicación.
- Encolar `MigrationJob` en el scheduler cuando el lifecycle engine decide
  mover datos entre tiers.

MinIO tiene su propio mecanismo de rebalanceo interno que no se puede reemplazar
con la lógica de scheduling de All4One sin modificaciones profundas.

---

## Alternativas descartadas

### MinIO con licencia comercial

**Descartado**: añade un coste por nodo que complica el modelo de precios de
All4One y crea una dependencia de proveedor externo. Si MinIO cambia sus
condiciones de licencia comercial, All4One pierde capacidad de negociación.

### MinIO como dependencia externa (cliente S3)

**Descartado**: requiere que el cliente instale y mantenga MinIO por separado,
lo que contradice el principio de "un único binario sin dependencias externas".
Además, el agente seguiría necesitando integrarse con MinIO via cliente S3, sin
acceso a sus internos para el placement y lifecycle engine.

### Ceph

**Descartado**: Ceph es significativamente más complejo de desplegar y gestionar
que All4One. Requiere múltiples daemons (mon, osd, mgr, mds), conocimiento
especializado, y mínimo 3 nodos para funcionar. Contradicción directa con
el objetivo de "funcional en un solo nodo".

---

## Consecuencias aceptadas

### Implementación propia de storage distribuido

All4One debe implementar:
- Chunking y SHA-256 por chunk.
- Reed-Solomon erasure coding (usando el crate `reed-solomon`, Apache 2.0).
- Compresión Zstd por tier.
- Índice de chunks con sled.
- Placement de chunks con reglas de tier.
- Scrubbing periódico.
- Drenado anticipado.

Esta es una cantidad significativa de código de infraestructura. Sin embargo,
todos estos componentes son exactamente los que All4One necesita y nada más —
sin las capas de compatibilidad que MinIO lleva para ser un drop-in replacement
de AWS S3.

### Mantenimiento propio

El módulo storage de All4One no tiene una comunidad externa que aporte
correcciones y mejoras. El equipo de All4One es responsable de su calidad.

**Mitigación**: los algoritmos fundamentales (Reed-Solomon, SHA-256, Zstd,
consistent hashing) están en crates bien mantenidos con licencias limpias.
El código propio es la lógica de coordinación, placement y lifecycle — la parte
que de todos modos tendría que ser personalizada en cualquier solución.
