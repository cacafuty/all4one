# Fase 0 - Preparacion del entorno de desarrollo

Fase 0 no entrega funcionalidad de produccion.
El objetivo es dejar un entorno reproducible para construir y validar All4One en Fase 1.

## Alcance general

Esta fase define una base comun y luego rutas por entorno.

Base comun para cualquier entorno:

- Git operativo.
- Rust estable (1.78 o superior) via rustup.
- protoc disponible (3.21 o superior).
- grpcurl disponible para pruebas gRPC.
- Docker operativo (si el nodo ejecutara jobs Docker o si usas cross con contenedores).
- Targets de Rust para los binarios esperados.

Targets esperados en Fase 1:

- linux-x86_64
- linux-aarch64
- windows-x86_64-gnu
- darwin-arm64 (recomendado en CI/macOS nativo)

Resultado esperado al cerrar Fase 0:

- Puedes compilar localmente para tu arquitectura nativa.
- Puedes validar prerequisitos con [scripts/check-env.sh](scripts/check-env.sh).
- Tienes definido como construir los targets no nativos segun tu entorno.

---

## Matriz por entorno

### Entorno A - SteamOS (principal)

Usa este track si desarrollas en Steam Deck / SteamOS.

1. Instala dependencias con privilegios root (gcc, docker y toolchains de linker):

```bash
sudo bash scripts/install-root-deps-steamos.sh
```

1. Instala Rust (usuario normal):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
```

1. Instala targets de Rust:

```bash
rustup target add aarch64-unknown-linux-gnu
rustup target add x86_64-pc-windows-gnu
rustup target add aarch64-apple-darwin
```

1. Instala utilidades de desarrollo:

```bash
cargo install cross --git https://github.com/cross-rs/cross
docker info
grpcurl --version || echo "instalar grpcurl"
```

1. Verifica entorno:

```bash
./scripts/check-env.sh
```

Notas SteamOS:

- Las actualizaciones del sistema pueden restaurar configuraciones del sistema base.
- Si Docker o toolchains desaparecen tras una actualizacion, vuelve a ejecutar [scripts/install-root-deps-steamos.sh](scripts/install-root-deps-steamos.sh).

### Entorno B - Linux estandar (Ubuntu, Debian, Fedora, Arch no-SteamOS)

Usa este track para estaciones Linux tradicionales.

1. Instala paquetes del sistema equivalentes:

- compilador/linker C (gcc o clang)
- pkg-config
- openssl dev headers
- docker
- toolchains de cross para ARM64 y MinGW (si haras cross sin contenedor)

1. Instala Rust con rustup y targets:

```bash
rustup target add aarch64-unknown-linux-gnu
rustup target add x86_64-pc-windows-gnu
rustup target add aarch64-apple-darwin
```

1. Instala cross y valida Docker:

```bash
cargo install cross --git https://github.com/cross-rs/cross
docker info
```

1. Ejecuta verificacion:

```bash
./scripts/check-env.sh
```

### Entorno C - macOS (ARM64)

Usa este track para desarrollo local en Mac.

1. Instala Xcode Command Line Tools y Homebrew.
1. Instala Rust con rustup.
1. Instala utilidades (git, protoc, grpcurl, docker si aplica).
1. Compila nativamente para darwin-arm64.

```bash
rustup target add aarch64-apple-darwin
cargo build -p agent
```

1. Para linux-aarch64 y windows-gnu, se recomienda build en CI o host Linux dedicado.

### Entorno D - Windows x86_64 (nativo o WSL2)

Usa este track para desarrollo en Windows.

Ruta recomendada: WSL2 para experiencia Linux y tooling consistente.

1. Instala Rust (rustup), Git y protoc.
1. Si usas Docker Desktop, valida acceso desde WSL2.
1. Para compilacion windows-gnu desde Linux/WSL2, instala MinGW correspondiente.
1. Para linux-aarch64, usa cross con Docker o CI.

En Windows nativo, asegura rutas y shells consistentes para scripts del proyecto.

---

## Reglas de decision para cross-compilation

- linux-aarch64:
  - Opcion 1: cross + Docker (mas portable).
  - Opcion 2: linker local aarch64 (mas rapido en iteracion).
- windows-x86_64-gnu:
  - Opcion 1: cross + Docker.
  - Opcion 2: linker MinGW local.
- darwin-arm64 desde Linux:
  - Preferir CI en runner macOS.
  - Evitar osxcross local salvo necesidad explicita.

---

## Verificacion final (obligatoria)

Antes de iniciar tareas de implementacion de Fase 1:

```bash
./scripts/check-env.sh
```

Si falla alguna verificacion, corrige dependencias en el track de tu entorno y repite.

---

## Relacion con otras guias (sin duplicar contenido)

- Para puesta en marcha del cluster y primer job: [docs/guides/getting-started.md](docs/guides/getting-started.md).
- Para configuracion detallada de nodos y ejemplos de agent.toml: [docs/guides/node-setup.md](docs/guides/node-setup.md).

Esta pagina se centra solo en preparar el entorno de desarrollo y compilacion.
