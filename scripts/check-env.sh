#!/usr/bin/env bash
ERRORS=0

check() {
  local name="$1"
  local cmd="$2"
  local fix="$3"
  if eval "$cmd" &>/dev/null; then
    echo "✓  $name"
  else
    echo "✗  $name  →  $fix"
    ERRORS=$((ERRORS + 1))
  fi
}

echo "=== Verificación entorno All4One Fase 1 ==="
echo ""

check "rustc >= 1.78"        "rustc --version"                   "ver docs/phases/phase-0.md Paso 2"
check "cargo"                "cargo --version"                   "ver docs/phases/phase-0.md Paso 2"
check "gcc (linker x86_64)"  "gcc --version"                     "ver docs/phases/phase-0.md Paso 1"
check "protoc >= 3.21"       "protoc --version"                  "ya instalado — revisar PATH"
check "git"                  "git --version"                     "ya instalado"
check "docker"               "docker info"                       "ver docs/phases/phase-0.md Paso 6"
check "grpcurl"              "grpcurl --version"                 "ver docs/phases/phase-0.md Paso 9"
check "cross"                "cross --version"                   "cargo install cross"

check "target linux-arm64"   \
  "rustup target list --installed | grep -q aarch64-unknown-linux-gnu" \
  "rustup target add aarch64-unknown-linux-gnu"

check "target windows"       \
  "rustup target list --installed | grep -q x86_64-pc-windows-gnu" \
  "rustup target add x86_64-pc-windows-gnu"

check "linker arm64"         "which aarch64-linux-gnu-gcc"       "ver docs/phases/phase-0.md Paso 5"
check "linker windows"       "which x86_64-w64-mingw32-gcc"      "ver docs/phases/phase-0.md Paso 7"

echo ""
if [ $ERRORS -eq 0 ]; then
  echo "✓  Entorno listo. Puedes empezar la Tarea 1 de Fase 1."
else
  echo "✗  $ERRORS herramienta(s) faltante(s). Instálalas antes de continuar."
  exit 1
fi
