# IA e inferencia

All4One expone una API compatible con OpenAI en `/v1/` y orquesta la inferencia
sobre los nodos del clúster según las capacidades disponibles. El sistema no
implementa el runtime de inferencia — delega en llama.cpp, ollama o vLLM.

---

## API compatible con OpenAI

Los endpoints de inferencia (Fase 5) están en el mismo puerto 7946 que la API REST:

```
POST /v1/chat/completions   → compatible con OpenAI ChatCompletion
POST /v1/completions        → compatible con OpenAI Completion
POST /v1/embeddings         → compatible con OpenAI Embeddings
GET  /v1/models             → lista modelos disponibles en el clúster
```

Los clientes que ya usen la API de OpenAI pueden apuntar a All4One cambiando
únicamente el `base_url`:

```python
import openai

client = openai.OpenAI(
    base_url="http://192.168.1.100:7946/v1",
    api_key="all4one-api-key-here",
)

response = client.chat.completions.create(
    model="llama3-70b-q4_k_m",
    messages=[{"role": "user", "content": "Explica el protocolo SWIM"}],
)
print(response.choices[0].message.content)
```

---

## Caso 1: modelo en un único nodo

El caso más simple: un modelo cabe en la RAM/VRAM de un nodo.

```
Cliente → POST /v1/chat/completions { model: "llama3-8b-q4_k_m", ... }
               │
               ▼
scheduler busca nodo con:
  - capabilities.executables o capabilities.docker (según runtime)
  - ram_mb_available >= 4500 (Llama 3 8B en Q4_K_M)
  - preferencia por nodo con el modelo ya cargado en RAM
               │
               ▼
executor lanza:
  Runtime Executable: llama-server --model /ruta/modelo.gguf --port 8080
  Runtime Docker:     ollama serve (con modelo precargado)
               │
               ▼
api_rest hace proxy de la petición al puerto 8080 del nodo seleccionado
               │
               ▼
Respuesta OpenAI al cliente (o SSE si stream=true)
```

### Caché de modelos

Los modelos GGUF se almacenan en el clúster como objetos en `volatile://models/`.
El lifecycle engine los gestiona con `policy: manual, pin: true` para que nunca
se archiven automáticamente.

El scheduler **prioriza nodos con el modelo ya cargado en RAM** añadiendo un
filtro de puntuación adicional: si `llama-server` ya está corriendo con ese
modelo en un nodo, ese nodo recibe un bonus de +0.30 en el score (equivalente
al peso completo de la señal de ventana).

---

## Caso 2: modelo distribuido (llama.cpp RPC)

Para modelos que no caben en un único nodo (Llama 3 405B requiere ~200 GB RAM),
llama.cpp implementa dos estrategias de distribución:

### Pipeline parallelism

```
Nodo coordinador (llama-server)          Nodo worker 1           Nodo worker 2
  Capas 0-39                               Capas 40-79             Capas 80-125
       │                                        │                       │
       │──[activaciones capa 39]───────────────►│                       │
       │                                        │──[activaciones 79]───►│
       │◄──────────────────────────────────────────────────[logits]─────│
       │
  Respuesta al cliente
```

- El modelo se divide por **capas** entre los nodos.
- Cada nodo procesa sus capas secuencialmente y pasa las activaciones al siguiente.
- La latencia total = suma de latencias de cada nodo + latencia de red entre ellos.
- **Tolera latencia de red alta** (1-10 GbE) a costa de throughput reducido.
- Ideal para clústeres de oficina con red estándar.

### Tensor parallelism

```
Nodo A                Nodo B                Nodo C
  Parte de capa 0      Parte de capa 0       Parte de capa 0
       │                    │                     │
       └────────── AllReduce ─────────────────────┘
                       │
              Siguiente capa (todos)
```

- Cada capa se **procesa en paralelo** entre todos los nodos.
- `AllReduce` sincroniza tras cada capa → requiere latencia de red < 1 ms.
- Requiere InfiniBand o Ethernet 100 GbE.
- El scheduler **rechaza** el job con `NETWORK_REQUIREMENTS_NOT_MET` si los nodos
  no tienen `network_bandwidth_mbps >= network_min_gbps * 1000`.
- Mayor throughput que pipeline parallelism en redes adecuadas.

### Job spec de inference_group

```yaml
runtime: inference_group
source: "volatile://models/llama3-405b-q4_k_m.gguf"
command: []
resources:
  cpu_cores: 0
  memory_mb: 0
  gpu: false
  max_duration_minutes: 10080   # 7 días — el servidor corre hasta que se cancela
partitioning:
  strategy: pipeline             # o "tensor"
  min_nodes: 3
  max_nodes: 8
constraints:
  tier_min: 1
  network_min_gbps: 10
  same_network: true
network:
  internet_access: false
  cluster_access: true
priority: normal
```

El scheduler calcula automáticamente la distribución de capas según la RAM
disponible en cada nodo candidato:

```rust
// scheduler/placement.rs — distribución de capas para pipeline parallelism
fn distribute_layers(
    model_layers: u32,           // total de capas del modelo (405B → 126 capas)
    nodes: &[NodeInfo],          // nodos candidatos ordenados por RAM disponible
) -> Vec<(NodeId, Range<u32>)> {
    let total_ram: u64 = nodes.iter().map(|n| n.resources.ram_mb_available).sum();
    let mut offset = 0u32;
    nodes.iter().map(|node| {
        let fraction = node.resources.ram_mb_available as f64 / total_ram as f64;
        let layers = (model_layers as f64 * fraction).round() as u32;
        let range = offset..(offset + layers).min(model_layers);
        offset += layers;
        (node.id, range)
    }).collect()
}
```

---

## Rendimiento aproximado (pipeline parallelism, Llama 3 70B, 3 nodos)

| Red      | Tokens/s | Uso recomendado                            |
|----------|----------|--------------------------------------------|
| 1 GbE    | ~0.15    | Batch nocturno — inviable para interactivo |
| 10 GbE   | ~1.5     | Aceptable para uso no interactivo          |
| 25 GbE   | ~4       | Bueno para uso interactivo                 |
| 100 GbE  | ~15      | Excelente                                  |

---

## Caso 3: escalado horizontal

Múltiples instancias del mismo modelo en nodos distintos, para mayor throughput
con modelos pequeños que caben en un solo nodo:

```
                    ┌──────────────────────┐
Cliente 1 ─────────►│                      ├──► Nodo A (llama3-8b instancia 1)
Cliente 2 ─────────►│  scheduler / balancer├──► Nodo B (llama3-8b instancia 2)
Cliente 3 ─────────►│                      ├──► Nodo C (llama3-8b instancia 3)
                    └──────────────────────┘
```

El scheduler lanza instancias adicionales cuando la cola de peticiones supera
un umbral configurable (`inference.scale_up_queue_depth`, default: 5) y las
detiene cuando la demanda baja (`inference.scale_down_idle_minutes`, default: 10).

---

## Tamaños de modelo (referencia)

Formato Q4_K_M (GGUF), RAM aproximada:

| Modelo           | RAM       | Nodos mínimos (16 GB RAM/nodo) |
|------------------|-----------|-------------------------------|
| Mistral 7B       | ~4 GB     | 1                             |
| Llama 3 8B       | ~4.5 GB   | 1                             |
| Gemma 2 27B      | ~15 GB    | 1–2                           |
| Llama 3 70B      | ~35 GB    | 3 (con 16 GB por nodo)        |
| Llama 3 405B     | ~200 GB   | 13 (con 16 GB por nodo)       |

---

## Runtimes de inferencia soportados

| Runtime    | Licencia    | Caso de uso                                    |
|------------|-------------|------------------------------------------------|
| llama.cpp  | MIT         | CPU y GPU, pipeline/tensor parallelism, GGUF   |
| ollama     | MIT         | Gestión simplificada de modelos, API REST local |
| vLLM       | Apache 2.0  | Alto throughput con GPU, continuous batching   |

El job spec indica el runtime en el campo `source`:
- `llama.cpp`: `source: "volatile://models/modelo.gguf"`, `runtime: executable`
- `ollama`: `source: "ollama/llama3:8b"`, `runtime: docker`
- `vLLM`: `source: "vllm/vllm-openai:latest"`, `runtime: docker`

---

## Streaming SSE (stream: true)

Cuando el cliente especifica `"stream": true` en la petición, la respuesta es
un stream Server-Sent Events compatible con el formato OpenAI:

```
data: {"id":"chatcmpl-abc123","object":"chat.completion.chunk","choices":[{"delta":{"content":"Hola"},"index":0}]}

data: {"id":"chatcmpl-abc123","object":"chat.completion.chunk","choices":[{"delta":{"content":" mundo"},"index":0}]}

data: [DONE]
```

El agente actúa como proxy transparente del stream generado por llama-server
u ollama hacia el cliente HTTP, sin buffering adicional.
