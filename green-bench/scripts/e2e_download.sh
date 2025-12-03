#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [ -f "$HOME/.cargo/env" ]; then
  . "$HOME/.cargo/env"
fi

# Fresh run location for the e2e download
rm -rf e2e_data

cargo build
cargo run -- download --dataset livebench/math --output-dir e2e_data --include-readme

TEST_FILE="e2e_data/livebench/math/data/test-00000-of-00001.parquet"
if [ ! -f "$TEST_FILE" ]; then
  echo "Download failed: missing ${TEST_FILE}"
  exit 1
fi

echo "Download succeeded: ${TEST_FILE}"
