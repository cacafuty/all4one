# ADR-004: llama.cpp RPC para inferencia distribuida

**Estado**: Aceptado
**Fecha**: 2026-04-08

---

## Contexto

All4One necesita soportar inferencia de LLMs grandes (70B–405B parámetros)
que no caben en la RAM de un solo nodo. El clúster típico de un cliente Business
tiene varios nodos con 16–64 GB de RAM cada uno. La solución debe tolerar la
latencia de red de oficina (1–10 GbE) y tener licencia compatible con
distribución propietaria.

---

## Decisión

**llama.cpp RPC** (MIT) como runtime de inferencia distribuida con pipeline
parallelism como estrategia principal para redes de oficina. Tensor parallelism
disponible como opción de configuración para clústeres con 25 GbE+.

All4One actúa como **orquestador** — el rol de scheduling y placement de los
procesos llama.cpp es de All4One. El runtime de inferencia puede ser llama.cpp,
ollama o vLLM según el caso.

---

## Razones

### Solución existente y probada con licencia MIT

llama.cpp implementa pipeline parallelism (llama-rpc-server) con licencia MIT.
Esto evita meses de implementación de un protocolo de comunicación de tensores
propio, con todos los problemas de correctitud y rendimiento que eso implica.

La alternativa sería implementar desde cero la comunicación de activaciones
entre capas — trabajo equivalente a varios meses de un ingeniero especialista
en sistemas distribuidos y ML.

### Pipeline parallelism tolera latencia de redes de oficina

La estrategia de pipeline parallelism es adecuada para el hardware del cliente
típico de All4One:

```
Modelo Llama 3 70B, 3 nodos con 16 GB RAM:

Pipeline parallelism (tolerante a latencia):
  1 GbE   → ~0.15 tokens/s  (batch nocturno, no interactivo)
  10 GbE  → ~1.5 tokens/s   (uso no interactivo)
  25 GbE  → ~4 tokens/s     (bueno para interactivo)
  100 GbE → ~15 tokens/s    (excelente)

Tensor parallelism (requiere latencia < 1 ms):
  Solo viable con InfiniBand o 100 GbE
  25 GbE puede funcionar si la latencia P99 < 1 ms
```

La mayoría de clientes Business tienen 1–10 GbE. Pipeline parallelism funciona
con estos clientes. Tensor parallelism es una opción para el 10% de clientes
con infraestructura de red superior.

### El scheduler controla el placement, no el runtime

All4One no reimplementa el protocolo de inferencia — solo decide qué nodos
ejecutan qué partes del modelo. La responsabilidad está bien delimitada:

- **All4One scheduler**: elige los nodos según RAM disponible, latencia de red,
  tier, ventana de disponibilidad.
- **llama.cpp RPC**: gestiona la comunicación de activaciones entre capas.

Esto significa que All4One puede soportar otros runtimes (ollama, vLLM) con
el mismo mecanismo de scheduling, simplemente cambiando qué proceso se lanza
en cada nodo.

### Protección de la inversión: rechazo explícito si la red no cumple

El scheduler rechaza el job `inference_group` con tensor parallelism si los
nodos no cumplen `network_min_gbps`, en lugar de dejar que el job corra con
rendimiento degradado sin que el usuario entienda por qué:

```
NETWORK_REQUIREMENTS_NOT_MET: tensor parallelism requires 25 Gbps,
but selected nodes have max 1 Gbps. Use strategy: pipeline instead.
```

Esto protege al usuario de expectativas incorrectas.

---

## Alternativas descartadas

### Implementación propia de tensor parallelism

**Descartada**: requiere implementar AllReduce distribuido con NCCL o comunicación
TCP personalizada, gestión de buffers de tensores entre nodos, sincronización
de gradientes. Trabajo de 6–12 meses con alto riesgo de bugs de rendimiento
sutiles. llama.cpp ya lo hace correctamente con MIT.

### vLLM como único runtime

**Descartado como opción única**: vLLM es excelente para alto throughput con GPU,
pero no soporta CPU-only de forma eficiente y no tiene inferencia distribuida
sin GPU (tensor parallelism requiere NCCL/GPU). La mayoría de clientes en
Fase 5 no tienen clústeres de GPU.

vLLM se soporta como runtime opcional (runtime: docker, source: vllm/...) para
clientes con GPU, pero no es la solución principal.

### Ray Serve / Triton Inference Server

**Descartados**: requieren Python + Ray o Triton como dependencia externa.
Contradicen el principio de "un único binario sin dependencias externas".
Sus licencias (Apache 2.0 para Ray, BSD para Triton) son compatibles, pero
el overhead de instalación y gestión es incompatible con el modelo de
despliegue de All4One.

---

## Consecuencias aceptadas

### Dependencia de llama.cpp como binario externo

Los binarios `llama-server` y `llama-rpc-server` deben estar disponibles en
cada nodo que participe en inference_group. All4One los gestiona como
ejecutables en `volatile://runtimes/llama-cpp/`, descargándolos automáticamente
cuando el scheduler asigna un inference_group a un nodo que no los tiene.

### Rendimiento subóptimo en 1 GbE para modelos grandes

Un modelo 70B en 3 nodos con 1 GbE produce ~0.15 tokens/s — inviable para
uso interactivo. El scheduler incluye advertencias cuando el placement
resultante producirá rendimiento bajo, para que el usuario tome una decisión
informada.

**Decisión pendiente**: umbral exacto de tokens/s para emitir la advertencia
`LOW_INFERENCE_PERFORMANCE` y si el scheduler rechaza el job o solo advierte.
