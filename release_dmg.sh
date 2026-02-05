#!/usr/bin/env bash
set -euo pipefail

# Build Tauri desktop app (frontend + Rust backend) and copy .dmg into ./release
#
# Usage:
#   ./release_dmg.sh
#
# Optional env:
#   BUILD_MODE=release|debug          (default: release)
#   SKIP_NPM_INSTALL=1               (default: 0; if node_modules missing, will run npm install)
#   OUT_DIR=/abs/or/relative/path    (default: ./release)
#
# Extra args are forwarded to `tauri build`, e.g.:
#   ./release_dmg.sh -- --verbose
#   BUILD_MODE=debug ./release_dmg.sh -- --debug

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="$ROOT_DIR/apps/desktop"
TAURI_DIR="$APP_DIR/src-tauri"

BUILD_MODE="${BUILD_MODE:-release}"
SKIP_NPM_INSTALL="${SKIP_NPM_INSTALL:-0}"
OUT_DIR="${OUT_DIR:-$ROOT_DIR/release}"

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "Missing required command: $1" >&2
    exit 1
  }
}

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This script builds a macOS .dmg and must run on macOS (Darwin)." >&2
  exit 1
fi

require_cmd npm
require_cmd node
require_cmd cargo
require_cmd rustc

if [[ ! -d "$APP_DIR" ]]; then
  echo "App directory not found: $APP_DIR" >&2
  exit 1
fi

if [[ ! -d "$TAURI_DIR" ]]; then
  echo "Tauri directory not found: $TAURI_DIR" >&2
  exit 1
fi

# Parse productName/version from tauri.conf.json (fallback to safe defaults)
TAURI_CONF="$TAURI_DIR/tauri.conf.json"
PRODUCT_NAME="$(
  node -e "try{const c=require(process.argv[1]);process.stdout.write(((c.package&&c.package.productName)||'app')+'')}catch(e){process.stdout.write('app')}" "$TAURI_CONF"
)"
APP_VERSION="$(
  node -e "try{const c=require(process.argv[1]);process.stdout.write(((c.package&&c.package.version)||'0.0.0')+'')}catch(e){process.stdout.write('0.0.0')}" "$TAURI_CONF"
)"

SAFE_NAME="${PRODUCT_NAME// /-}"
ARCH="$(uname -m)"
STAMP="$(date +%Y%m%d-%H%M%S)"

mkdir -p "$OUT_DIR"

(
  cd "$APP_DIR"

  if [[ "$SKIP_NPM_INSTALL" != "1" ]] && [[ ! -d node_modules ]]; then
    echo "node_modules not found. Installing dependencies..."
    npm install
  fi

  echo "Building Tauri bundle (.dmg)..."
  # Use local @tauri-apps/cli (devDependency) via npm exec.
  # Forward extra args after optional `--` to `tauri build`.
  EXTRA_BUILD_ARGS=()
  if [[ "$BUILD_MODE" == "debug" ]]; then
    EXTRA_BUILD_ARGS+=(--debug)
  fi
  # Bash 3.2 + `set -u` can error on expanding an empty array, so guard it.
  if ((${#EXTRA_BUILD_ARGS[@]} > 0)); then
    npm exec tauri -- build --bundles dmg "${EXTRA_BUILD_ARGS[@]}" "$@"
  else
    npm exec tauri -- build --bundles dmg "$@"
  fi
)

TARGET_DIR="$TAURI_DIR/target/$BUILD_MODE/bundle/dmg"
if [[ ! -d "$TARGET_DIR" ]]; then
  # Tauri sometimes emits bundles under `target/release` even if profile env differs.
  TARGET_DIR="$TAURI_DIR/target/release/bundle/dmg"
fi

DMG_PATH="$(ls -t "$TARGET_DIR"/"${SAFE_NAME}"*.dmg 2>/dev/null | head -n 1 || true)"
if [[ -z "$DMG_PATH" ]]; then
  DMG_PATH="$(ls -t "$TARGET_DIR"/*.dmg 2>/dev/null | head -n 1 || true)"
  if [[ -n "$DMG_PATH" ]]; then
    echo "Warning: DMG name does not match productName (${PRODUCT_NAME})." >&2
    echo "         Check tauri.conf.json productName and rebuild." >&2
  fi
fi
if [[ -z "$DMG_PATH" ]]; then
  echo "Failed to find .dmg in: $TARGET_DIR" >&2
  echo "Tip: open \"$TAURI_DIR/target\" to inspect build outputs." >&2
  exit 1
fi

# Remove stale DMGs from previous product names (avoid accidental installs).
for dmg in "$OUT_DIR"/*.dmg; do
  [[ -e "$dmg" ]] || continue
  base="$(basename "$dmg")"
  if [[ "$base" != "${SAFE_NAME}"*".dmg" ]]; then
    rm -f "$dmg"
  fi
done

OUT_NAME="${SAFE_NAME}-v${APP_VERSION}-macos-${ARCH}-${STAMP}.dmg"

cp -f "$DMG_PATH" "$OUT_DIR/$OUT_NAME"
cp -f "$DMG_PATH" "$OUT_DIR/${SAFE_NAME}-latest.dmg"

echo "OK"
echo "  Source: $DMG_PATH"
echo "  Output: $OUT_DIR/$OUT_NAME"
echo "  Latest: $OUT_DIR/${SAFE_NAME}-latest.dmg"

