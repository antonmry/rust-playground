#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Ensure cargo is on PATH for non-login shells.
if [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1090
  . "$HOME/.cargo/env"
fi

echo "Running cargo tests..."
cargo test

echo "Running sample command..."
OUTPUT="$(cargo run --quiet -- --no-cpu --no-gpu --output json -- true)"
echo "$OUTPUT"

echo "$OUTPUT" | grep -q '"exit_code": 0'
echo "E2E checks passed."
