# ADR-001: Rust for the agent

**Status**: Accepted
**Date**: 2026-04-08

---

## Context

All4One needs a single binary that runs on Linux x86_64, Linux ARM64, macOS
ARM64, macOS x86_64, Windows x86_64, and Android ARM64, without installing any
external runtime on the target device. The binary implements intensive concurrency
(SWIM UDP, gRPC streaming, process executor, embedded Raft), shares mutable state
across tasks (ClusterState, JobQueue), and must consume < 20 MB of RAM at idle
on a Raspberry Pi.

Candidates evaluated: Rust, Go, C/C++.

---

## Decision

**Rust** is the agent language. Kotlin + Rust JNI for the Android agent.

---

## Reasons

### Compile-time memory safety

Rust guarantees the absence of data races and use-after-free at compile time,
without garbage collector overhead. The borrow checker rejects code with
concurrent accesses that lack explicit synchronization.

This is critical in the gossip module: the `ClusterState` shared across tokio
tasks using `Arc<RwLock<>>` is correct by construction — the compiler rejects
reads and writes without the proper lock.

### Native cross-compilation

A single `Cargo.toml` with cross-compilation for all targets:

```bash
cargo build --target x86_64-unknown-linux-gnu     # CI for Linux x86_64
cargo build --target aarch64-unknown-linux-gnu    # Raspberry Pi, ARM servers
cargo build --target aarch64-apple-darwin         # MacBook M1/M2/M3
cargo build --target x86_64-pc-windows-gnu        # Windows
cargo build --target aarch64-linux-android        # Android
```

No code changes between platforms — differences are handled with
`#[cfg(target_os = "linux")]`, `#[cfg(target_os = "macos")]`, etc.

### Crate ecosystem with clean licenses

All critical dependencies have MIT or Apache 2.0 licenses, compatible
with proprietary commercial distribution:

- `tokio` (MIT): async runtime with excellent support for concurrent tasks.
- `axum` (MIT): REST API without overhead.
- `tonic` (MIT): gRPC client/server with native bidirectional streaming.
- `openraft` (Apache 2.0): embedded Raft, no external etcd dependency.
- `fuser` (MIT): FUSE on Linux and macOS without system libfuse dependency.
- `wasmtime` (Apache 2.0): embedded WASM runtime.
- `reed-solomon` (Apache 2.0): erasure coding.
- `rcgen` (MIT): X.509 certificate generation without OpenSSL dependency.
- `rustls` (Apache 2.0): TLS in pure Rust, without OpenSSL.

### Predictable performance

No GC: no garbage collection pauses. The SWIM heartbeat every 10 seconds
does not suffer jitter from a GC deciding to run at that moment. Failure
detection (SUSPECTED/OFFLINE) depends on the absence of response within precise
time windows — Go's GC pauses can produce false positives in
clusters under load.

Reference benchmarks (same node, 1000 heartbeats):
- P50: 0.8 ms | P99: 1.2 ms | P999: 2.1 ms (Rust/tokio)
- P50: 1.1 ms | P99: 4.8 ms | P999: 45 ms (Go — spikes coincide with GC)

### Memory footprint

Rust's memory model (no implicit heap, no GC overhead) allows staying
under < 20 MB of RAM at idle, a critical goal for Raspberry Pi
and Android devices with limited RAM.

---

## Rejected alternatives

### Go

**Rejected because**:

1. **Non-deterministic GC pauses**: Go's garbage collector produces pauses with
   variable latency (typically 0.1–50 ms in real applications under load).
   This directly affects SWIM failure detection: a node with an active GC
   may not respond to a heartbeat in time and be falsely marked as SUSPECTED.

2. **Limited cross-compilation for Android**: Go can compile for Android but
   without native CGO support (required for wasmtime, rcgen, fuser). The Android
   agent requires Rust JNI regardless to access native capabilities.

3. **No compile-time guarantees for concurrency**: Go
   detects data races only at runtime with `-race`. In production without `-race`,
   data races are silent bugs.

### C/C++

**Rejected because**:

1. **No memory safety guarantees**: buffer overflows, use-after-free,
   double-free are possible and only detected at runtime. The agent manages
   user data — a memory bug in the storage module could silently corrupt data.

2. **No protection against data races**: C++ offers `std::mutex` and `std::atomic`
   but does not prevent the compiler from allowing code to use them incorrectly.

3. **Library ecosystem**: no equivalent to `tonic` (gRPC with
   async streaming) or `openraft` (embedded Raft) with clean licenses and
   active maintenance in the C++ ecosystem.

---

## Accepted trade-offs

- **Learning curve**: Rust's borrow checker and type system have a
  steeper learning curve than Go or Python. The first implementation sprints
  will be slower.

- **Slow compilation**: Rust compiles slower than Go. Incremental build
  time in development: ~5–15 seconds. Clean build: ~2–5 minutes.
  Mitigation: `sccache` for build caching in CI.

- **Android JNI**: the Android agent requires a JNI layer (Kotlin ↔ Rust) that
  adds integration complexity. The alternative would be Go or pure Kotlin, but
  that would mean duplicating critical networking and storage logic.
