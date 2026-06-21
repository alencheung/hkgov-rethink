#!/usr/bin/env bash
# One-shot demo: boots hkgov-api, waits for the cache to warm, prints three
# real insights captured from live HKGOV data, then exits.
#
# Usage:
#   ./scripts/demo.sh                     # build if needed, run, print, exit
#   HKGOV_DEMO_NOBUILD=1 ./scripts/demo.sh   # skip the cargo build
#
# Requires: cargo (or a prebuilt binary if HKGOV_DEMO_NOBUILD=1), curl, jq.
# Falls back gracefully if jq is missing.
set -euo pipefail

cd "$(dirname "$0")/.."

BIN="${HKGOV_DEMO_BIN:-target/release/hkgov-api}"
PORT="${HKGOV_DEMO_PORT:-18080}"
BASE="http://localhost:${PORT}"

if [[ "${HKGOV_DEMO_NOBUILD:-0}" != "1" ]]; then
  echo "==> building hkgov-api (release)…"
  cargo build --release -p hkgov-api >/dev/null
fi

echo "==> starting hkgov-api on :${PORT} (agent enabled, heuristic mode)…"
HKGOV_API__BIND="0.0.0.0:${PORT}" HKGOV_AGENT__ENABLED=true "$BIN" &
SERVER_PID=$!

cleanup() { kill "$SERVER_PID" 2>/dev/null || true; }
trap cleanup EXIT INT TERM

# Wait for health.
echo "==> waiting for server…"
for _ in $(seq 1 60); do
  if curl -fsS "${BASE}/health" >/dev/null 2>&1; then break; fi
  sleep 1
done

echo "==> warming cache (waiting for the first agent pass; ~30s)…"
# The agent's first pass is delayed 20s after boot; give it room.
for _ in $(seq 1 50); do
  n=$(curl -fsS "${BASE}/v1/insights?limit=1" 2>/dev/null | jq 'length' 2>/dev/null || echo 0)
  if [[ "$n" -gt 0 ]]; then break; fi
  sleep 1
done

echo
echo "================ hkgov-rethink — live insights ================"
echo "Captured from live HKGOV data via the heuristic agent (no LLM key)."
echo

print_section() {
  local title="$1" query="$2"
  echo "----- ${title} -----"
  if command -v jq >/dev/null 2>&1; then
    curl -fsS "${BASE}${query}" | jq '.'
  else
    curl -fsS "${BASE}${query}"
  fi
  echo
}

print_section "insights (latest 5)" "/v1/insights?limit=5"

echo "----- ask: interbank liquidity -----"
if command -v jq >/dev/null 2>&1; then
  curl -fsS -X POST "${BASE}/v1/ask" \
    -H 'Content-Type: application/json' \
    -d '{"question":"what is the interbank liquidity doing?"}' | jq '.'
else
  curl -fsS -X POST "${BASE}/v1/ask" \
    -H 'Content-Type: application/json' \
    -d '{"question":"what is the interbank liquidity doing?"}'
fi
echo

echo "==============================================================="
echo "Server still running at ${BASE} — Ctrl-C to stop, or it'll exit on script end."
