#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_DIR="${REPO_DIR:-${ROOT}/candle-vllm}"
BIN="${BIN:-${REPO_DIR}/target/release/candle-vllm}"
PORT="${PORT:-2000}"
MODEL_ID="${MODEL_ID:-Qwen/Qwen2.5-0.5B-Instruct-GGUF}"
MODEL_FILE="${MODEL_FILE:-qwen2.5-0.5b-instruct-q4_0.gguf}"
HF_CACHE="${HF_CACHE:-${ROOT}/hf-cache}"

if [ -f "$HOME/.cargo/env" ]; then
  . "$HOME/.cargo/env"
fi

if [ ! -x "$BIN" ]; then
  echo "[run] candle-vllm binary not found at ${BIN}. Build it first (scripts/install_candle_vllm.sh)."
  exit 1
fi

mkdir -p "$HF_CACHE"
export HF_HOME="$HF_CACHE"
export HF_HUB_CACHE="$HF_CACHE"

echo "[run] Starting candle-vllm"
echo "       repo   : ${REPO_DIR}"
echo "       binary : ${BIN}"
echo "       port   : ${PORT}"
echo "       model  : ${MODEL_ID}"
echo "       file   : ${MODEL_FILE}"
echo "       cache  : ${HF_CACHE}"

exec "$BIN" --p "$PORT" --m "$MODEL_ID" --f "$MODEL_FILE" --log
