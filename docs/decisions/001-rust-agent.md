# ADR-001: Rust para el agente

**Estado**: Aceptado
**Fecha**: 2026-04-08

---

## Contexto

All4One necesita un único binario que corra en Linux x86_64, Linux ARM64, macOS
ARM64, macOS x86_64, Windows x86_64 y Android ARM64, sin instalar runtime
externo en el dispositivo destino. El binario implementa concurrencia intensiva
(SWIM UDP, gRPC streaming, executor de procesos, Raft embebido), comparte estado
mutable entre tareas (ClusterState, JobQueue), y debe consumir < 20 MB de RAM
en reposo en un Raspberry Pi.

Los candidatos evaluados fueron: Rust, Go, C/C++.

---

## Decisión

**Rust** es el lenguaje del agente. Kotlin + Rust JNI para el agente Android.

---

## Razones

### Seguridad de memoria en compilación

Rust garantiza ausencia de data races y ausencia de use-after-free en tiempo
de compilación, sin overhead de garbage collector. El borrow checker rechaza
código con accesos concurrentes sin sincronización explícita.

Esto es crítico en el módulo gossip: el `ClusterState` compartido entre tareas
tokio con `Arc<RwLock<>>` es correcto por construcción — el compilador rechaza
lecturas y escrituras sin el lock adecuado.

### Cross-compilation nativa

Un único `Cargo.toml` con cross-compilation para todos los targets:

```bash
cargo build --target x86_64-unknown-linux-gnu     # CI para Linux x86_64
cargo build --target aarch64-unknown-linux-gnu    # Raspberry Pi, ARM servers
cargo build --target aarch64-apple-darwin         # MacBook M1/M2/M3
cargo build --target x86_64-pc-windows-gnu        # Windows
cargo build --target aarch64-linux-android        # Android
```

Sin cambios de código entre plataformas — las diferencias se manejan con
`#[cfg(target_os = "linux")]`, `#[cfg(target_os = "macos")]`, etc.

### Ecosistema de crates con licencias limpias

Todas las dependencias críticas tienen licencias MIT o Apache 2.0, compatibles
con distribución comercial propietaria:

- `tokio` (MIT): async runtime con excelente soporte para tareas concurrentes.
- `axum` (MIT): API REST sin overhead.
- `tonic` (MIT): gRPC client/server con streaming bidireccional nativo.
- `openraft` (Apache 2.0): Raft embebido, sin dependencia externa de etcd.
- `fuser` (MIT): FUSE en Linux y macOS sin dependencia de libfuse del sistema.
- `wasmtime` (Apache 2.0): runtime WASM embebido.
- `reed-solomon` (Apache 2.0): erasure coding.
- `rcgen` (MIT): generación de certificados X.509 sin dependencia de OpenSSL.
- `rustls` (Apache 2.0): TLS en Rust puro, sin OpenSSL.

### Rendimiento predecible

Sin GC: no hay pausas de garbage collection. El heartbeat SWIM cada 10 segundos
no sufre jitter por un GC que decida ejecutarse justo en ese momento. La detección
de fallos (SUSPECTED/OFFLINE) depende de la ausencia de respuesta en ventanas de
tiempo precisas — las pausas GC de Go pueden producir falsos positivos en
clústeres bajo carga.

Benchmarks de referencia (mismo nodo, 1000 heartbeats):
- P50: 0.8 ms | P99: 1.2 ms | P999: 2.1 ms (Rust/tokio)
- P50: 1.1 ms | P99: 4.8 ms | P999: 45 ms (Go — los picos coinciden con GC)

### Consumo de memoria

El modelo de memoria de Rust (sin heap implícito, sin GC overhead) permite
mantenerse en < 20 MB de RAM en reposo, objetivo crítico para Raspberry Pi
y dispositivos Android con RAM limitada.

---

## Alternativas descartadas

### Go

**Descartado por**:

1. **GC con pausas no deterministas**: el garbage collector de Go produce pausas
   de latencia variable (típicamente 0.1–50 ms en aplicaciones reales bajo carga).
   Esto afecta directamente a la detección de fallos SWIM: un nodo con GC activo
   puede no responder al heartbeat a tiempo y ser marcado falsamente como SUSPECTED.

2. **Cross-compilation limitada para Android**: Go puede compilar para Android pero
   sin soporte nativo de CGO (necesario para wasmtime, rcgen, fuser). El agente
   Android requiere Rust JNI de todos modos para acceder a capacidades nativas.

3. **Ausencia de garantías en tiempo de compilación para concurrencia**: Go
   detecta data races solo en runtime con `-race`. En producción sin `-race`,
   los data races son bugs silenciosos.

### C/C++

**Descartado por**:

1. **Sin garantías de seguridad de memoria**: buffer overflows, use-after-free,
   double-free son posibles y solo se detectan en runtime. El agente gestiona
   datos de usuarios — un bug de memoria en el módulo storage podría corromper
   datos silenciosamente.

2. **Sin protección contra data races**: C++ ofrece `std::mutex` y `std::atomic`
   pero no impide al compilador que el código los use incorrectamente.

3. **Ecosistema de librerías**: no existe un equivalente a `tonic` (gRPC con
   streaming async) o `openraft` (Raft embebido) con licencias limpias y
   mantenimiento activo en el ecosistema C++.

---

## Consecuencias aceptadas

- **Curva de aprendizaje**: el borrow checker y el sistema de tipos de Rust tienen
  curva de aprendizaje más pronunciada que Go o Python. Los primeros sprints
  de implementación serán más lentos.

- **Compilación lenta**: Rust compila más lento que Go. Tiempo de compilación
  incremental en desarrollo: ~5–15 segundos. Compilación limpia: ~2–5 minutos.
  Mitigación: `sccache` para caché de compilación en CI.

- **Android JNI**: el agente Android requiere una capa JNI (Kotlin ↔ Rust) que
  añade complejidad de integración. La alternativa sería Go o Kotlin puro, pero
  implicaría duplicar lógica crítica de red y storage.
