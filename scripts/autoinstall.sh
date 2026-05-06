#!/usr/bin/env bash
set -euo pipefail

GH_REPO="cacafuty/all4one"
VERSION="latest"
INSTALL_DIR="/usr/local/bin"
CONFIG_PATH="/etc/all4one/agent.toml"
DATA_DIR="/var/lib/all4one"
BIND_ADDRESS="0.0.0.0"
REST_PORT="7946"
GRPC_PORT="7947"
SEEDS=""
SHARED_SECRET="change-me-before-production"
JOIN_CERT=""
JOIN_ENDPOINT=""
GITHUB_TOKEN="${GITHUB_TOKEN:-}"
NO_START="false"

log() {
  echo "[autoinstall] $*"
}

fail() {
  echo "[autoinstall] ERROR: $*" >&2
  exit 1
}

usage() {
  cat <<'USAGE'
Usage: autoinstall.sh [options]

Options:
  --repo <owner/repo>          GitHub repository (default: cacafuty/all4one)
  --version <semver|latest>    Release version without leading v; latest picks the newest published release, including prereleases when needed (default: latest)
  --install-dir <path>         Binary install directory (default: /usr/local/bin)
  --config <path>              Agent config path (default: /etc/all4one/agent.toml)
  --data-dir <path>            Data directory for node state (default: /var/lib/all4one)
  --bind-address <ip>          bind_address in config (default: 0.0.0.0)
  --rest-port <port>           rest_port in config (default: 7946)
  --grpc-port <port>           grpc_port in config (default: 7947)
  --seeds <host:port,...>      Comma-separated discovery seeds (default: empty)
  --shared-secret <secret>     Security shared secret in dev mode
  --join-cert <path>           Future join certificate path (stored as comment for future phases)
  --join-endpoint <host:port>  Future cluster join endpoint (stored as comment for future phases)
  --github-token <token>       GitHub token for private repos or API rate limits
  --no-start                   Install and configure, but do not launch agent
  --help                       Show this help

Examples:
  ./scripts/autoinstall.sh
  ./scripts/autoinstall.sh --version 0.1.5 --shared-secret mysecret
  ./scripts/autoinstall.sh --seeds 192.168.1.100:7947,192.168.1.101:7947
  GITHUB_TOKEN=ghp_xxx ./scripts/autoinstall.sh --version 0.1.5
USAGE
}

curl_auth_args() {
  if [ -n "$GITHUB_TOKEN" ]; then
    printf -- '-H\0Authorization: Bearer %s\0' "$GITHUB_TOKEN"
  fi
}

curl_download() {
  local url out
  url="$1"
  out="$2"

  if [ -n "$GITHUB_TOKEN" ]; then
    curl -fsSL -H "Authorization: Bearer ${GITHUB_TOKEN}" "$url" -o "$out"
  else
    curl -fsSL "$url" -o "$out"
  fi
}

curl_text() {
  local url
  url="$1"

  if [ -n "$GITHUB_TOKEN" ]; then
    curl -fsSL -H "Authorization: Bearer ${GITHUB_TOKEN}" "$url"
  else
    curl -fsSL "$url"
  fi
}

run_as_root() {
  if [ "${EUID:-$(id -u)}" -eq 0 ]; then
    "$@"
  elif command -v sudo >/dev/null 2>&1; then
    sudo "$@"
  else
    fail "This action requires root privileges and sudo is not available: $*"
  fi
}

parse_args() {
  while [ $# -gt 0 ]; do
    case "$1" in
      --repo)
        GH_REPO="$2"
        shift 2
        ;;
      --version)
        VERSION="$2"
        shift 2
        ;;
      --install-dir)
        INSTALL_DIR="$2"
        shift 2
        ;;
      --config)
        CONFIG_PATH="$2"
        shift 2
        ;;
      --data-dir)
        DATA_DIR="$2"
        shift 2
        ;;
      --bind-address)
        BIND_ADDRESS="$2"
        shift 2
        ;;
      --rest-port)
        REST_PORT="$2"
        shift 2
        ;;
      --grpc-port)
        GRPC_PORT="$2"
        shift 2
        ;;
      --seeds)
        SEEDS="$2"
        shift 2
        ;;
      --shared-secret)
        SHARED_SECRET="$2"
        shift 2
        ;;
      --join-cert)
        JOIN_CERT="$2"
        shift 2
        ;;
      --join-endpoint)
        JOIN_ENDPOINT="$2"
        shift 2
        ;;
      --github-token)
        GITHUB_TOKEN="$2"
        shift 2
        ;;
      --no-start)
        NO_START="true"
        shift
        ;;
      --help|-h)
        usage
        exit 0
        ;;
      *)
        fail "Unknown option: $1"
        ;;
    esac
  done
}

resolve_platform_asset() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Linux)
      case "$arch" in
        x86_64|amd64)
          echo "all4one-agent-linux-x86_64.tar.gz"
          ;;
        aarch64|arm64)
          echo "all4one-agent-linux-arm64.tar.gz"
          ;;
        *)
          fail "Unsupported Linux architecture: $arch"
          ;;
      esac
      ;;
    Darwin)
      case "$arch" in
        arm64)
          echo "all4one-agent-macos-arm64.tar.gz"
          ;;
        *)
          fail "Unsupported macOS architecture: $arch (only arm64 is published)"
          ;;
      esac
      ;;
    *)
      fail "Unsupported OS for this script: $os"
      ;;
  esac
}

resolve_tag() {
  if [ "$VERSION" = "latest" ]; then
    local latest_api list_api tag
    latest_api="https://api.github.com/repos/${GH_REPO}/releases/latest"
    tag="$(curl_text "$latest_api" 2>/dev/null | grep -o '"tag_name"[[:space:]]*:[[:space:]]*"[^"]*"' | head -n1 | sed 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)"/\1/')"
    if [ -n "$tag" ]; then
      echo "$tag"
      return
    fi

    list_api="https://api.github.com/repos/${GH_REPO}/releases?per_page=10"
    tag="$(curl_text "$list_api" | grep -o '"tag_name"[[:space:]]*:[[:space:]]*"[^"]*"' | head -n1 | sed 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)"/\1/')"
    [ -n "$tag" ] || fail "Could not resolve latest release tag from ${latest_api}"
    echo "$tag"
  else
    echo "v${VERSION}"
  fi
}

verify_checksum() {
  local asset_path checksums_path filename expected actual entry
  asset_path="$1"
  checksums_path="$2"
  filename="$(basename "$asset_path")"
  entry="$(awk -v target="$filename" '$NF == target || $NF == ("./" target) { print; exit }' "$checksums_path")"
  [ -n "$entry" ] || fail "No checksum entry found for ${filename}"
  expected="$(printf '%s\n' "$entry" | awk '{print $1}')"
  [ -n "$expected" ] || fail "Invalid checksum entry for ${filename}"

  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$asset_path" | awk '{print $1}')"
  elif command -v shasum >/dev/null 2>&1; then
    actual="$(shasum -a 256 "$asset_path" | awk '{print $1}')"
  else
    log "No sha256 tool found (sha256sum/shasum). Skipping checksum verification."
    return
  fi

  [ "$expected" = "$actual" ] || fail "Checksum mismatch for ${filename}"
}

download_or_fail() {
  local url out label
  url="$1"
  out="$2"
  label="$3"

  if ! curl_download "$url" "$out"; then
    fail "Could not download ${label} from ${url}. If this is a tagged version, confirm the GitHub Release has binary assets published (not only source archives)."
  fi
}

write_default_config_if_missing() {
  if [ -f "$CONFIG_PATH" ]; then
    log "Config already exists at ${CONFIG_PATH}. Keeping existing file."
    return
  fi

  local cfg_dir seeds_toml
  cfg_dir="$(dirname "$CONFIG_PATH")"
  run_as_root mkdir -p "$cfg_dir" "$DATA_DIR"

  if [ -n "$SEEDS" ]; then
    IFS=',' read -r -a seed_items <<< "$SEEDS"
    seeds_toml="["
    for i in "${!seed_items[@]}"; do
      local item
      item="${seed_items[$i]}"
      if [ "$i" -gt 0 ]; then
        seeds_toml+=" ,"
      fi
      seeds_toml+="\"${item}\""
    done
    seeds_toml+="]"
  else
    seeds_toml="[]"
  fi

  local tmp
  tmp="$(mktemp)"
  cat > "$tmp" <<EOF
[node]
tier = 0
availability = "always"
quorum_participant = true
data_dir = "${DATA_DIR}"

[roles]
scheduler = true
executor = true
storage = false

[network]
bind_address = "${BIND_ADDRESS}"
grpc_port = ${GRPC_PORT}
rest_port = ${REST_PORT}

[discovery]
mdns = true
seeds = ${seeds_toml}

[security]
mode = "dev"
shared_secret = "${SHARED_SECRET}"

[executor]
max_concurrent_jobs = 4
docker_socket = "/var/run/docker.sock"
cgroups_enabled = true

[capabilities]
docker = true
python = "/usr/bin/python3"
wasm = true

[logging]
level = "info"
format = "text"

# Future join parameters (phase 2+ mTLS enrollment)
# join_cert = "${JOIN_CERT}"
# join_endpoint = "${JOIN_ENDPOINT}"
EOF

  run_as_root cp "$tmp" "$CONFIG_PATH"
  rm -f "$tmp"
  log "Created default config at ${CONFIG_PATH}"
}

main() {
  parse_args "$@"

  local asset tag base_url tmp_dir asset_path checksums_path bin_path
  asset="$(resolve_platform_asset)"
  tag="$(resolve_tag)"
  base_url="https://github.com/${GH_REPO}/releases/download/${tag}"

  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "${tmp_dir:-}"' EXIT

  asset_path="${tmp_dir}/${asset}"
  checksums_path="${tmp_dir}/checksums.sha256"

  log "Downloading ${asset} from ${base_url}"
  download_or_fail "${base_url}/${asset}" "$asset_path" "$asset"

  log "Downloading checksums"
  download_or_fail "${base_url}/checksums.sha256" "$checksums_path" "checksums.sha256"

  log "Verifying checksum"
  verify_checksum "$asset_path" "$checksums_path"

  run_as_root mkdir -p "$INSTALL_DIR"
  log "Installing binary into ${INSTALL_DIR}"
  run_as_root tar xzf "$asset_path" -C "$INSTALL_DIR"
  run_as_root chmod +x "${INSTALL_DIR}/all4one-agent"

  bin_path="${INSTALL_DIR}/all4one-agent"
  log "Installed binary: ${bin_path}"
  "$bin_path" --version || true

  write_default_config_if_missing

  if [ "$NO_START" = "true" ]; then
    log "Install completed (no-start mode)."
    exit 0
  fi

  log "Starting agent with config ${CONFIG_PATH}"
  exec "$bin_path" start --config "$CONFIG_PATH"
}

main "$@"
