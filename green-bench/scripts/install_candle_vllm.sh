#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_DIR="${REPO_DIR:-${ROOT}/candle-vllm}"
FEATURES="${CANDLE_VLLM_FEATURES:-}"

if [ -f "$HOME/.cargo/env" ]; then
  . "$HOME/.cargo/env"
fi

echo "[install] Installing system dependencies (build-essential, pkg-config, libssl-dev, git-lfs)..."
sudo apt-get update -y
sudo apt-get install -y build-essential pkg-config libssl-dev git-lfs jq

if [ ! -d "$REPO_DIR/.git" ]; then
  echo "[install] Cloning candle-vllm into ${REPO_DIR}..."
  git clone --depth 1 https://github.com/EricLBuehler/candle-vllm.git "$REPO_DIR"
else
  echo "[install] Updating existing candle-vllm repo at ${REPO_DIR}..."
  git -C "$REPO_DIR" pull --ff-only
fi

echo "[install] Building candle-vllm in release mode..."
cd "$REPO_DIR"
if [ -n "$FEATURES" ]; then
  echo "[install] Using features: ${FEATURES}"
  cargo build --release --features "${FEATURES}"
else
  cargo build --release
fi

echo "[install] Done. Binary at ${REPO_DIR}/target/release/candle-vllm"
