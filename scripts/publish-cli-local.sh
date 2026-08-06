#!/usr/bin/env bash
# Build IntelliHelper CLI for as many platforms as this machine can, then
# optionally upload installers + binaries + stable pointer to Cloudflare R2.
#
# Usage (from repo root):
#   ./scripts/publish-cli-local.sh                 # build only → dist/cli/
#   ./scripts/publish-cli-local.sh --upload        # build + upload to R2
#   VERSION=0.1.1 ./scripts/publish-cli-local.sh --upload
#   PLATFORMS=macos-aarch64,linux-x86_64 ./scripts/publish-cli-local.sh
#
# R2 (required for --upload):
#   export CLOUDFLARE_ACCOUNT_ID=...
#   export R2_ACCESS_KEY_ID=...
#   export R2_SECRET_ACCESS_KEY=...
#   export R2_BUCKET_NAME=intellihelper-cli   # optional
#
# Platforms:
#   macos-aarch64  — native on Apple Silicon
#   macos-x86_64   — cross from Apple Silicon (needs Xcode CLT)
#   linux-x86_64   — Docker (rustc + protoc)
#   linux-aarch64  — Docker (linux/arm64; on Apple Silicon uses native arm)
#   windows-x86_64 — Docker + cargo-xwin (best-effort; may fail on deps)
#
# Note: A full multi-platform release from one Mac is heavy (time + disk).
# GitHub Actions remains the reliable path for all five every push to main.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

export PATH="${HOME}/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:${PATH}"

VERSION="${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' crates/codegen/intellihelper-pager-bin/Cargo.toml | head -1)}"
DIST="${DIST:-${ROOT}/dist/cli}"
BUCKET="${R2_BUCKET_NAME:-intellihelper-cli}"
UPLOAD=0
DO_WINDOWS=0

# Default platforms for a first local release (skip windows unless asked)
DEFAULT_PLATFORMS="macos-aarch64,macos-x86_64,linux-x86_64,linux-aarch64"
PLATFORMS="${PLATFORMS:-$DEFAULT_PLATFORMS}"

for arg in "$@"; do
  case "$arg" in
    --upload) UPLOAD=1 ;;
    --with-windows) DO_WINDOWS=1; PLATFORMS="${PLATFORMS},windows-x86_64" ;;
    --help|-h)
      sed -n '2,35p' "$0"
      exit 0
      ;;
    *)
      echo "Unknown arg: $arg (try --help)" >&2
      exit 1
      ;;
  esac
done

if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9._]+)?$ ]]; then
  echo "Invalid VERSION='$VERSION'" >&2
  exit 1
fi

mkdir -p "$DIST"
echo "==> Version: $VERSION"
echo "==> Output:  $DIST"
echo "==> Platforms: $PLATFORMS"

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "Missing required command: $1" >&2
    exit 1
  }
}

build_native() {
  local target="$1" platform="$2" exe_suffix="${3:-}"
  need_cmd cargo
  need_cmd rustup
  rustup target add "$target" >/dev/null
  echo "==> Building $platform ($target)…"
  INTELLIHELPER_VERSION="$VERSION" \
    cargo build -p intellihelper-pager-bin --release --target "$target"
  local src="target/${target}/release/intellihelper-pager${exe_suffix}"
  local dest="${DIST}/intellihelper-${VERSION}-${platform}${exe_suffix}"
  cp "$src" "$dest"
  if [[ -z "$exe_suffix" ]]; then
    chmod +x "$dest"
    "$dest" --version || true
  fi
  ls -lh "$dest"
}

build_linux_docker() {
  local target="$1" platform="$2" docker_platform="$3"
  need_cmd docker
  echo "==> Building $platform via Docker ($docker_platform, $target)…"
  # Mount cargo/registry caches when present to speed rebuilds.
  local cargo_home="${CARGO_HOME:-$HOME/.cargo}"
  docker run --rm \
    --platform "$docker_platform" \
    -v "$ROOT:/src" \
    -v "${cargo_home}/registry:/usr/local/cargo/registry" \
    -v "${cargo_home}/git:/usr/local/cargo/git" \
    -w /src \
    -e "INTELLIHELPER_VERSION=${VERSION}" \
    -e "CARGO_TERM_COLOR=always" \
    rust:1.94.0-bookworm \
    bash -lc '
      set -euo pipefail
      export DEBIAN_FRONTEND=noninteractive
      apt-get update -qq
      apt-get install -y -qq protobuf-compiler pkg-config libssl-dev cmake build-essential >/dev/null
      rustup target add "'"$target"'"
      cargo build -p intellihelper-pager-bin --release --target "'"$target"'"
    '
  local src="target/${target}/release/intellihelper-pager"
  local dest="${DIST}/intellihelper-${VERSION}-${platform}"
  cp "$src" "$dest"
  chmod +x "$dest"
  ls -lh "$dest"
}

build_windows_docker() {
  local platform="windows-x86_64"
  need_cmd docker
  echo "==> Building $platform via cargo-xwin (best-effort)…"
  docker run --rm \
    -v "$ROOT:/io" \
    -w /io \
    -e "INTELLIHELPER_VERSION=${VERSION}" \
    ghcr.io/rust-cross/cargo-xwin:latest \
    bash -lc '
      set -euo pipefail
      apt-get update -qq || true
      apt-get install -y -qq protobuf-compiler cmake pkg-config 2>/dev/null || true
      cargo xwin build -p intellihelper-pager-bin --release --target x86_64-pc-windows-msvc
    '
  local src="target/x86_64-pc-windows-msvc/release/intellihelper-pager.exe"
  local dest="${DIST}/intellihelper-${VERSION}-${platform}.exe"
  cp "$src" "$dest"
  ls -lh "$dest"
}

IFS=',' read -ra WANT <<< "$PLATFORMS"
for p in "${WANT[@]}"; do
  p="$(echo "$p" | xargs)" # trim
  [[ -z "$p" ]] && continue
  case "$p" in
    macos-aarch64)
      build_native aarch64-apple-darwin macos-aarch64
      ;;
    macos-x86_64)
      build_native x86_64-apple-darwin macos-x86_64
      ;;
    linux-x86_64)
      build_linux_docker x86_64-unknown-linux-gnu linux-x86_64 linux/amd64
      ;;
    linux-aarch64)
      build_linux_docker aarch64-unknown-linux-gnu linux-aarch64 linux/arm64
      ;;
    windows-x86_64)
      build_windows_docker
      ;;
    *)
      echo "Unknown platform: $p" >&2
      exit 1
      ;;
  esac
done

echo ""
echo "==> Built artifacts:"
ls -lh "$DIST"/intellihelper-"${VERSION}"-* 2>/dev/null || ls -lh "$DIST"

upload_r2() {
  need_cmd aws
  if [[ -z "${CLOUDFLARE_ACCOUNT_ID:-}" || -z "${R2_ACCESS_KEY_ID:-}" || -z "${R2_SECRET_ACCESS_KEY:-}" ]]; then
    echo "Missing CLOUDFLARE_ACCOUNT_ID / R2_ACCESS_KEY_ID / R2_SECRET_ACCESS_KEY" >&2
    exit 1
  fi
  local endpoint="https://${CLOUDFLARE_ACCOUNT_ID}.r2.cloudflarestorage.com"
  export AWS_ACCESS_KEY_ID="$R2_ACCESS_KEY_ID"
  export AWS_SECRET_ACCESS_KEY="$R2_SECRET_ACCESS_KEY"
  export AWS_DEFAULT_REGION=auto

  put() {
    local key="$1" file="$2" ctype="$3"
    echo "→ s3://${BUCKET}/${key}"
    aws s3 cp "$file" "s3://${BUCKET}/${key}" \
      --endpoint-url "$endpoint" \
      --content-type "$ctype" \
      --only-show-errors
  }

  put "install.sh" "crates/codegen/intellihelper-pager/scripts/install.sh" "text/x-shellscript"
  put "install.ps1" "crates/codegen/intellihelper-pager/scripts/install.ps1" "text/plain"
  put "enterprise-install.sh" "crates/codegen/intellihelper-pager/scripts/install-enterprise.sh" "text/x-shellscript"
  put "enterprise-install.ps1" "crates/codegen/intellihelper-pager/scripts/install-enterprise.ps1" "text/plain"

  printf '%s' "$VERSION" > "${DIST}/stable"
  put "stable" "${DIST}/stable" "text/plain"

  shopt -s nullglob
  for f in "$DIST"/intellihelper-"${VERSION}"-*; do
    put "$(basename "$f")" "$f" "application/octet-stream"
  done

  echo ""
  echo "Published ${VERSION} to R2 bucket ${BUCKET}"
  echo "  https://cli.intellihelper.in/stable"
  echo "  curl -fsSL https://cli.intellihelper.in/install.sh | bash"
}

if [[ "$UPLOAD" -eq 1 ]]; then
  upload_r2
else
  echo ""
  echo "Build only (no upload). To publish:"
  echo "  export CLOUDFLARE_ACCOUNT_ID=..."
  echo "  export R2_ACCESS_KEY_ID=..."
  echo "  export R2_SECRET_ACCESS_KEY=..."
  echo "  VERSION=${VERSION} ./scripts/publish-cli-local.sh --upload"
fi
