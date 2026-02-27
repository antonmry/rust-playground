#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BENCH_DIR="$SCRIPT_DIR"
MODELS_DIR="$ROOT_DIR/models"
DATA_DIR="$ROOT_DIR/data"

# --- Find faq_cli binary ---
find_faq_cli() {
    if command -v faq_cli &>/dev/null; then
        echo "faq_cli"
        return
    fi

    local release="$ROOT_DIR/target/release/faq_cli"
    if [[ -x "$release" ]]; then
        echo "$release"
        return
    fi

    local debug="$ROOT_DIR/target/debug/faq_cli"
    if [[ -x "$debug" ]]; then
        echo "$debug"
        return
    fi

    echo ""
}

FAQ_CLI="$(find_faq_cli)"
if [[ -z "$FAQ_CLI" ]]; then
    echo "ERROR: faq_cli not found in PATH, target/release/, or target/debug/"
    echo "Run 'cargo build --release' first."
    exit 1
fi

echo "Using: $FAQ_CLI"
echo "========================================"

# --- Discover models ---
MODELS=()
LABELS=()
TOKENIZER="$MODELS_DIR/tokenizer.json"

# Always include hash backend
MODELS+=("")
LABELS+=("hash")

# Auto-detect all GGUF files in models/
for gguf in "$MODELS_DIR"/*.gguf; do
    [[ -f "$gguf" ]] || continue
    MODELS+=("$gguf")
    LABELS+=("$(basename "$gguf" .gguf)")
done

echo "Detected backends: ${LABELS[*]}"
echo ""

# --- Build indexes ---
echo "=== Building indexes ==="
for i in "${!MODELS[@]}"; do
    label="${LABELS[$i]}"
    model="${MODELS[$i]}"
    index="$BENCH_DIR/index_${label}.jsonl"

    echo -n "  Building index for '$label' ... "

    if [[ -z "$model" ]]; then
        "$FAQ_CLI" build-index \
            --input "$DATA_DIR/faq_seed.jsonl" \
            --output "$index" 2>/dev/null
    else
        "$FAQ_CLI" \
            --model-path "$model" \
            --tokenizer-path "$TOKENIZER" \
            build-index \
            --input "$DATA_DIR/faq_seed.jsonl" \
            --output "$index" 2>/dev/null
    fi

    echo "done ($index)"
done

echo ""

# --- Run evals ---
echo "=== Running evals ==="
for i in "${!MODELS[@]}"; do
    label="${LABELS[$i]}"
    model="${MODELS[$i]}"
    index="$BENCH_DIR/index_${label}.jsonl"

    echo "--- $label ---"

    if [[ -z "$model" ]]; then
        "$FAQ_CLI" eval \
            --index "$index" \
            --cases "$BENCH_DIR/eval_cases.json" 2>/dev/null
    else
        "$FAQ_CLI" \
            --model-path "$model" \
            --tokenizer-path "$TOKENIZER" \
            eval \
            --index "$index" \
            --cases "$BENCH_DIR/eval_cases.json" 2>/dev/null
    fi

    echo ""
done
