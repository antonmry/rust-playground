#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PORT="${PORT:-2000}"
LOG_FILE="${LOG_FILE:-${ROOT}/candle-vllm-server.log}"

if [ -f "$HOME/.cargo/env" ]; then
  . "$HOME/.cargo/env"
fi

echo "[test] Launching candle-vllm in background..."
HF_CACHE="${HF_CACHE:-${ROOT}/hf-cache}" PORT="$PORT" bash "${ROOT}/scripts/run_candle_vllm.sh" >"$LOG_FILE" 2>&1 &
SERVER_PID=$!

cleanup() {
  if kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID"
    wait "$SERVER_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

echo "[test] Waiting for server to become ready (PID: $SERVER_PID)..."
READY=0
for i in $(seq 1 60); do
  sleep 2
  if curl -s "http://127.0.0.1:${PORT}/v1/models" >/dev/null 2>&1; then
    READY=1
    break
  fi
done

if [ "$READY" -ne 1 ]; then
  echo "[test] Server did not become ready. Logs:"
  tail -n 40 "$LOG_FILE" || true
  exit 1
fi

echo "[test] Sending chat completion request..."
REQ=$(cat <<'EOF'
{
  "model": "Qwen/Qwen2.5-0.5B-Instruct-GGUF",
  "messages": [{"role": "user", "content": "Say hello in two words."}],
  "max_tokens": 16
}
EOF
)

RESP="$(curl -sS -X POST "http://127.0.0.1:${PORT}/v1/chat/completions" \
  -H "Content-Type: application/json" \
  -d "$REQ")"

echo "$RESP" | jq '.choices[0].message.content' >/dev/null

echo "[test] Success. Response content:"
echo "$RESP" | jq '.choices[0].message.content'
