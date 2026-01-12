#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_PATH="${ROOT_DIR}/target-linux/debug/anxo-game"
OUT_PATH="${ROOT_DIR}/tmp/anxo-screen.png"
mkdir -p "${ROOT_DIR}/tmp"

CODE=$'from game import hero\nhero.move_up()\n'
if [[ ${1-} != "" ]]; then
  CODE="$1"
fi

CARGO_TARGET_DIR="${ROOT_DIR}/target-linux" cargo build

export ANXO_PLACEHOLDER=1
export ANXO_AUTORUN=1
export ANXO_START_CODE="$CODE"
xvfb-run -s '-screen 0 1024x768x24' sh -c \
  "${BIN_PATH} & pid=\$!; sleep 5; import -window root ${OUT_PATH}; kill \$pid"

echo "Saved screenshot to ${OUT_PATH}"
