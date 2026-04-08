#!/usr/bin/env bash
set -euo pipefail

if [[ ${EUID:-$(id -u)} -ne 0 ]]; then
  echo "This script must run as root."
  echo "Use: sudo bash scripts/install-root-deps-steamos.sh"
  exit 1
fi

echo "==> Disabling SteamOS read-only mode"
steamos-readonly disable

cleanup() {
  echo "==> Re-enabling SteamOS read-only mode"
  steamos-readonly enable || true
}
trap cleanup EXIT

echo "==> Initializing pacman keyring (safe if already initialized)"
pacman-key --init || true
pacman-key --populate archlinux || true

echo "==> Installing required root-level packages"
pacman -Sy --noconfirm \
  gcc \
  pkg-config \
  openssl \
  docker \
  aarch64-linux-gnu-gcc \
  mingw-w64-gcc

echo "==> Enabling Docker service"
systemctl enable --now docker

echo "==> Adding current user to docker group"
if [[ -n "${SUDO_USER:-}" ]]; then
  usermod -aG docker "$SUDO_USER"
  echo "User '$SUDO_USER' added to docker group. Log out/in to apply."
else
  echo "SUDO_USER not detected; skipping usermod."
fi

echo "==> Root-level dependencies installed successfully"
echo "Run the verifier as your normal user:"
echo "  source \"$HOME/.cargo/env\" && export PATH=\"$HOME/.local/bin:$PATH\" && ./scripts/check-env.sh"
