# Fase 0 — Entorno de desarrollo

Esta fase no produce código de producción. El objetivo es tener la máquina
de desarrollo lista para compilar All4One para todos los targets de Fase 1:
Linux x86_64, Linux ARM64, macOS ARM64 y Windows x86_64.

**Plataforma de desarrollo**: SteamOS (Arch Linux x86_64).
Los pasos marcados con `[TODAS]` aplican a cualquier distribución Linux.

---

## Resumen de herramientas a instalar

| Herramienta         | Versión mínima | Instalación          | Para qué                              |
|---------------------|---------------|----------------------|---------------------------------------|
| Rust (rustup)       | 1.78+         | `$HOME/.cargo`       | compilar el agente                    |
| gcc                 | cualquiera    | pacman (root)        | linker nativo x86_64                  |
| cross               | 0.2+          | `cargo install`      | cross-compilar ARM64 y Windows        |
| Docker              | 20+           | flatpak / manual     | cross usa contenedores; tests locales |
| protoc              | 3.21+         | ya instalado         | generar código desde .proto           |
| grpcurl             | 1.8+          | binario precompilado | probar endpoints gRPC                 |
| git                 | cualquiera    | ya instalado         | control de versiones                  |

---

## Paso 1 — Desactivar el modo read-only de SteamOS e instalar gcc

SteamOS monta `/usr` en modo read-only por defecto. Hay que desactivarlo
temporalmente para instalar `gcc` (el linker que Rust necesita para x86_64).
**Las actualizaciones del sistema de SteamOS restaurarán el modo read-only** —
es necesario repetir este paso tras cada actualización del sistema.

```bash
# Desactivar modo read-only:
sudo steamos-readonly disable

# Inicializar el keyring de pacman (necesario si nunca se ha hecho):
sudo pacman-key --init
sudo pacman-key --populate archlinux

# Instalar gcc y pkg-config:
sudo pacman -Sy --noconfirm gcc pkg-config openssl

# Verificar:
gcc --version
# gcc (GCC) 15.x.x ...

# Reactivar modo read-only (buena práctica — gcc queda instalado):
sudo steamos-readonly enable
```

> **Nota**: `gcc-libs` ya estaba en el sistema. El paquete que faltaba es
> `gcc` (que incluye el compilador y el linker `ld`).

---

## Paso 2 — Instalar Rust via rustup

`rustup` instala Rust completamente en `$HOME/.cargo` y `$HOME/.rustup` —
**no requiere root** y **sobrevive a las actualizaciones de SteamOS**.

```bash
# Instalar rustup:
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- \
  --default-toolchain stable \
  --profile default \
  --no-modify-path \
  -y

# Cargar el entorno en la sesión actual:
source "$HOME/.cargo/env"

# Verificar:
rustc --version
# rustc 1.78.x (o superior)
cargo --version
# cargo 1.78.x (o superior)

# Añadir al shell permanentemente:
echo 'source "$HOME/.cargo/env"' >> ~/.bashrc
# Si usas zsh:
echo 'source "$HOME/.cargo/env"' >> ~/.zshrc
```

---

## Paso 3 — Añadir targets de cross-compilation

Los targets para Linux ARM64 (Raspberry Pi, servidores ARM) y Windows x86_64
se añaden con `rustup target add`. **No requieren root.**

```bash
# Linux ARM64 (Raspberry Pi, servidores ARM):
rustup target add aarch64-unknown-linux-gnu

# macOS ARM64 (cross-compilar desde Linux — requiere osxcross, ver Paso 5):
rustup target add aarch64-apple-darwin

# Windows x86_64:
rustup target add x86_64-pc-windows-gnu

# Verificar targets instalados:
rustup target list --installed
# aarch64-apple-darwin
# aarch64-unknown-linux-gnu
# x86_64-pc-windows-gnu
# x86_64-unknown-linux-gnu  ← el nativo, ya estaba
```

---

## Paso 4 — Instalar `cross` para cross-compilation con Docker

`cross` es una herramienta que lanza la compilación dentro de contenedores
Docker preconfigurados con los linkers y sysroots necesarios. Es la forma
más fiable de cross-compilar en Linux sin tener que instalar toolchains
manualmente.

```bash
# Instalar cross:
cargo install cross --git https://github.com/cross-rs/cross

# Verificar:
cross --version
# cross 0.2.x

# cross requiere Docker corriendo. Verificar Docker:
docker info | head -5
# Si Docker no está corriendo: ver Paso 6.
```

---

## Paso 5 — Linker para Linux ARM64 (sin Docker)

Si prefieres cross-compilar ARM64 sin Docker (más rápido que `cross` para
iteraciones rápidas), instala el linker `aarch64-linux-gnu-gcc`:

```bash
sudo steamos-readonly disable
sudo pacman -Sy --noconfirm aarch64-linux-gnu-gcc
sudo steamos-readonly enable

# Configurar Cargo para usar este linker con el target ARM64:
mkdir -p ~/.cargo
cat >> ~/.cargo/config.toml << 'EOF'
[target.aarch64-unknown-linux-gnu]
linker = "aarch64-linux-gnu-gcc"
EOF

# Probar compilación ARM64:
cd /home/deck/Projects/all4one
cargo build --target aarch64-unknown-linux-gnu -p agent 2>&1 | tail -5
# Finishing ... (sin errores de linker)

# Verificar que el binario es ARM64:
file target/aarch64-unknown-linux-gnu/debug/all4one-agent
# ELF 64-bit LSB pie executable, ARM aarch64
```

---

## Paso 6 — Docker (para `cross` y para tests de jobs Docker)

En SteamOS, Docker se puede instalar via el script oficial o via flatpak.
El método recomendado en SteamOS es el script oficial:

```bash
# Método 1: script oficial de Docker
curl -fsSL https://get.docker.com -o /tmp/get-docker.sh
# Revisar el script antes de ejecutarlo:
head -20 /tmp/get-docker.sh

sudo steamos-readonly disable
sudo sh /tmp/get-docker.sh
sudo steamos-readonly enable

# Añadir el usuario al grupo docker (sin esto necesitas sudo para docker):
sudo usermod -aG docker $USER
# Reiniciar la sesión para que el grupo surta efecto.

# Arrancar Docker:
sudo systemctl enable --now docker

# Verificar:
docker run --rm hello-world
# Hello from Docker!
```

> **Nota**: Las actualizaciones de SteamOS pueden desinstalar Docker al
> restaurar el sistema de ficheros. Si ocurre, repetir la instalación.
> Los datos de los contenedores en `/var/lib/docker` se perderían también —
> para desarrollo esto es aceptable.

---

## Paso 7 — Windows x86_64: linker MinGW

Para compilar el target `x86_64-pc-windows-gnu` sin Docker:

```bash
sudo steamos-readonly disable
sudo pacman -Sy --noconfirm mingw-w64-gcc
sudo steamos-readonly enable

# Configurar Cargo:
cat >> ~/.cargo/config.toml << 'EOF'
[target.x86_64-pc-windows-gnu]
linker = "x86_64-w64-mingw32-gcc"
EOF

# Probar compilación Windows:
cargo build --target x86_64-pc-windows-gnu -p agent 2>&1 | tail -5
# Finishing ...

file target/x86_64-pc-windows-gnu/debug/all4one-agent.exe
# PE32+ executable (console) x86-64, for MS Windows
```

---

## Paso 8 — macOS ARM64 (cross-compilación opcional)

La cross-compilación para macOS desde Linux requiere `osxcross` con el SDK
de macOS. Es el target más complejo de configurar.

**Decisión**: para Fase 1, compilar el binario macOS en una máquina macOS
real (CI en GitHub Actions con `macos-14` runner que es ARM64) es más práctico
que configurar osxcross localmente.

Para compilación local en macOS (si tienes acceso a un Mac):
```bash
# En el Mac:
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
rustup target add aarch64-apple-darwin  # si estás en Intel Mac
cargo build --target aarch64-apple-darwin -p agent
```

Para CI con GitHub Actions, el target macOS ARM64 se compila con:
```yaml
# .github/workflows/build.yml
- name: Build macOS ARM64
  runs-on: macos-14        # runner ARM64
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
    - run: cargo build --release -p agent
```

---

## Paso 9 — grpcurl (para probar endpoints gRPC)

`grpcurl` es la herramienta equivalente a `curl` para gRPC. Se usa en los
tests de todas las tareas que involucran gRPC.

```bash
# Descargar binario precompilado:
curl -sSL https://github.com/fullstorydev/grpcurl/releases/download/v1.9.1/grpcurl_1.9.1_linux_x86_64.tar.gz \
  -o /tmp/grpcurl.tar.gz
tar -xzf /tmp/grpcurl.tar.gz -C /tmp
mkdir -p ~/.local/bin
mv /tmp/grpcurl ~/.local/bin/grpcurl
chmod +x ~/.local/bin/grpcurl

# Añadir ~/.local/bin al PATH si no está:
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc

# Verificar:
grpcurl --version
# grpcurl v1.9.1
```

---

## Paso 10 — protoc (ya instalado, verificar versión)

`protoc` ya está instalado en el sistema. Se usa en el `build.rs` del workspace
para generar código Rust desde los ficheros `.proto`.

```bash
protoc --version
# libprotoc 31.1  ← suficiente, se requiere >= 3.21

# El crate prost-build en build.rs lo usa automáticamente.
# No requiere configuración adicional.
```

---

## Paso 11 — Configurar el proyecto

```bash
cd /home/deck/Projects/all4one

# Verificar que git está configurado:
git config user.name   || git config --global user.name "Tu Nombre"
git config user.email  || git config --global user.email "tu@email.com"

# Crear .gitignore base:
cat > .gitignore << 'EOF'
/target/
**/*.rs.bk
.env
*.pem
*.key
node-id
EOF

# Inicializar el repositorio si no está inicializado:
git status || git init
```

---

## Verificación final del entorno

Ejecuta este script para confirmar que todo está listo antes de empezar Tarea 1:

```bash
#!/usr/bin/env bash
set -e
ERRORS=0

check() {
  local name="$1"
  local cmd="$2"
  local expected="$3"
  if eval "$cmd" &>/dev/null; then
    echo "✓  $name"
  else
    echo "✗  $name — $expected"
    ERRORS=$((ERRORS + 1))
  fi
}

echo "=== Verificación entorno All4One Fase 1 ==="

check "rustc >= 1.78"        "rustc --version"                   "instalar via rustup"
check "cargo"                "cargo --version"                   "instalar via rustup"
check "gcc (linker x86_64)"  "gcc --version"                     "sudo pacman -Sy gcc"
check "protoc >= 3.21"       "protoc --version"                  "ya instalado"
check "git"                  "git --version"                     "ya instalado"
check "docker"               "docker info"                       "ver Paso 6"
check "grpcurl"              "grpcurl --version"                 "ver Paso 9"
check "cross"                "cross --version"                   "cargo install cross"

check "target linux-arm64"   "rustup target list --installed | grep aarch64-unknown-linux-gnu" \
  "rustup target add aarch64-unknown-linux-gnu"

check "target windows"       "rustup target list --installed | grep x86_64-pc-windows-gnu" \
  "rustup target add x86_64-pc-windows-gnu"

check "linker arm64"         "which aarch64-linux-gnu-gcc"       "sudo pacman -Sy aarch64-linux-gnu-gcc"
check "linker windows"       "which x86_64-w64-mingw32-gcc"      "sudo pacman -Sy mingw-w64-gcc"

echo ""
if [ $ERRORS -eq 0 ]; then
  echo "✓  Entorno listo. Puedes empezar Tarea 1."
else
  echo "✗  $ERRORS herramienta(s) faltante(s). Instálalas antes de continuar."
  exit 1
fi
```

Guarda el script como `scripts/check-env.sh` y ejecútalo:

```bash
mkdir -p /home/deck/Projects/all4one/scripts
# (pegar el script arriba)
chmod +x scripts/check-env.sh
./scripts/check-env.sh
```

---

## Estado actual del entorno (pre-instalación)

| Herramienta  | Estado       | Acción requerida                          |
|--------------|--------------|-------------------------------------------|
| git          | ✓ instalado  | —                                         |
| python3      | ✓ 3.13.5     | —                                         |
| protoc       | ✓ 31.1       | —                                         |
| rustc/cargo  | ✗ falta      | Pasos 1 + 2                               |
| gcc          | ✗ falta      | Paso 1 (requiere sudo)                    |
| cross        | ✗ falta      | Paso 4 (tras instalar cargo)              |
| docker       | ✗ falta      | Paso 6 (requiere sudo)                    |
| grpcurl      | ✗ falta      | Paso 9                                    |
| linker ARM64 | ✗ falta      | Paso 5 (requiere sudo)                    |
| linker Win   | ✗ falta      | Paso 7 (requiere sudo)                    |

**Orden de instalación**:
1. Paso 1 (con sudo): `gcc`, `aarch64-linux-gnu-gcc`, `mingw-w64-gcc`
2. Paso 6 (con sudo): Docker
3. Paso 2 (sin sudo): rustup
4. Paso 3 (sin sudo): targets de cross-compilation
5. Paso 4 (sin sudo): cross
6. Paso 9 (sin sudo): grpcurl
