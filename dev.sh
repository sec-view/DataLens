#!/usr/bin/env bash
set -euo pipefail

# Root dev runner:
# - checks required ports before starting
# - starts one or more services
# - ensures all started services exit when this script exits (CTRL+C / terminal close / error)
#
# Usage:
#   ./dev.sh                # start tauri dev (includes vite dev)
#   ./dev.sh tauri          # same as default
#   ./dev.sh vite           # only start vite dev
#
# Optional env:
#   FORCE_KILL=1|0          # if port is occupied, kill the process(es) on that port before starting (default: 1)

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="$ROOT_DIR/apps/desktop"

MODE="${1:-tauri}"

#
# Test-friendly defaults:
# - Always rebuild frontend once before starting dev servers, so the runtime always picks up latest changes.
# - Optionally clean common Svelte/Vite caches to avoid "stale UI" surprises.
#
# Env:
#   REBUILD_FRONTEND=1|0   (default: 1)改成0可以跳过重建
#   CLEAN_FRONTEND=1|0     (default: 1)
#
REBUILD_FRONTEND="${REBUILD_FRONTEND:-1}"
CLEAN_FRONTEND="${CLEAN_FRONTEND:-1}"

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "Missing required command: $1" >&2
    exit 1
  }
}

prebuild_frontend() {
  if [[ ! -d "$APP_DIR" ]]; then
    echo "App directory not found: $APP_DIR" >&2
    exit 1
  fi

  if [[ "$CLEAN_FRONTEND" == "1" ]]; then
    echo "Cleaning frontend caches..."
    rm -rf \
      "$APP_DIR/.svelte-kit" \
      "$APP_DIR/build" \
      "$APP_DIR/dist" \
      "$APP_DIR/node_modules/.vite" \
      2>/dev/null || true
  fi

  if [[ "$REBUILD_FRONTEND" == "1" ]]; then
    echo "Rebuilding frontend (vite build)..."
    (
      cd "$APP_DIR"
      npm run build
    )
  fi
}

port_pids() {
  # macOS-friendly: return PIDs listening on TCP port
  local port="$1"
  lsof -nP -t -iTCP:"$port" -sTCP:LISTEN 2>/dev/null || true
}

print_port_info() {
  local port="$1"
  echo "Port $port is in use:"
  # pid, user, cmd, full line (best-effort)
  lsof -nP -iTCP:"$port" -sTCP:LISTEN 2>/dev/null || true
}

kill_port_listeners() {
  local port="$1"
  local pids
  pids="$(port_pids "$port")"
  if [[ -z "${pids}" ]]; then
    return 0
  fi
  echo "Killing listeners on port $port: ${pids//$'\n'/ }"
  # best-effort: first TERM, then KILL
  while IFS= read -r pid; do
    [[ -z "$pid" ]] && continue
    kill -TERM "$pid" 2>/dev/null || true
  done <<< "$pids"
  sleep 0.3
  if [[ -n "$(port_pids "$port")" ]]; then
    while IFS= read -r pid; do
      [[ -z "$pid" ]] && continue
      kill -KILL "$pid" 2>/dev/null || true
    done <<< "$pids"
  fi
}

check_or_free_port() {
  local port="$1"
  if [[ -n "$(port_pids "$port")" ]]; then
    print_port_info "$port"
    # Default behavior: auto-free occupied ports to avoid dev startup failures.
    # Set FORCE_KILL=0 to make this script refuse to start when a port is occupied.
    if [[ "${FORCE_KILL:-1}" == "1" ]]; then
      kill_port_listeners "$port"
      # Ensure it's actually free before proceeding.
      if [[ -n "$(port_pids "$port")" ]]; then
        echo "Failed to free port $port. Refusing to start." >&2
        exit 1
      fi
    else
      echo "Refusing to start: port $port is occupied. Set FORCE_KILL=1 to kill it automatically." >&2
      exit 1
    fi
  fi
}

require_cmd lsof
require_cmd npm

if [[ ! -d "$APP_DIR" ]]; then
  echo "App directory not found: $APP_DIR" >&2
  exit 1
fi

# Services started by this script (store their leader PIDs)
PIDS=()

cleanup() {
  local code=$?
  if ((${#PIDS[@]} > 0)); then
    echo
    echo "Stopping services..."
  fi

  # Kill each service's process group so child processes also exit.
  # Background jobs in bash get their own process group (pgid == pid typically).
  for pid in "${PIDS[@]:-}"; do
    [[ -z "${pid:-}" ]] && continue
    kill -TERM "-$pid" 2>/dev/null || true
  done

  # Give them a moment to exit gracefully.
  local deadline=$((SECONDS + 6))
  while (( SECONDS < deadline )); do
    local any_alive=0
    for pid in "${PIDS[@]:-}"; do
      [[ -z "${pid:-}" ]] && continue
      if kill -0 "$pid" 2>/dev/null; then
        any_alive=1
        break
      fi
    done
    (( any_alive == 0 )) && break
    sleep 0.2
  done

  # Hard kill remaining groups
  for pid in "${PIDS[@]:-}"; do
    [[ -z "${pid:-}" ]] && continue
    kill -KILL "-$pid" 2>/dev/null || true
  done

  exit "$code"
}

trap cleanup EXIT INT TERM

start_service() {
  local name="$1"
  local cwd="$2"
  shift 2
  local cmd=("$@")

  echo "Starting ${name}: (cd ${cwd} && ${cmd[*]})"
  (
    cd "$cwd"
    exec "${cmd[@]}"
  ) &

  local pid=$!
  PIDS+=("$pid")
  echo "  -> ${name} pid=${pid}"
}

case "$MODE" in
  tauri)
    # tauri dev will run "npm run dev" (Vite) as beforeDevCommand, using port 5173.
    check_or_free_port 5173
    prebuild_frontend
    start_service "tauri" "$APP_DIR" npm run tauri
    ;;
  vite)
    check_or_free_port 5173
    prebuild_frontend
    start_service "vite" "$APP_DIR" npm run dev
    ;;
  *)
    echo "Unknown mode: $MODE" >&2
    echo "Usage: ./dev.sh [tauri|vite]" >&2
    exit 1
    ;;
esac

echo
echo "Services are running. Press CTRL+C to stop."

# Wait for all services; if any exits, trap will run and clean up.
wait

