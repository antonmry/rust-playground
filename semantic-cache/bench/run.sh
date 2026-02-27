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

# --- Find tokenizer for a given model file ---
# For model "foo.gguf" or "foo.safetensors", look for "foo-tokenizer.json"
# first, then fall back to "tokenizer.json" in the same directory.
find_tokenizer() {
    local model_file="$1"
    local dir
    dir="$(dirname "$model_file")"
    local base
    base="$(basename "$model_file")"

    # Strip the extension (.gguf or .safetensors)
    local stem="${base%.gguf}"
    stem="${stem%.safetensors}"

    local specific="$dir/${stem}-tokenizer.json"
    if [[ -f "$specific" ]]; then
        echo "$specific"
        return
    fi

    local fallback="$dir/tokenizer.json"
    if [[ -f "$fallback" ]]; then
        echo "$fallback"
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

# --- Human-readable file size ---
human_size() {
    local file="$1"
    local bytes
    bytes=$(stat --format='%s' "$file" 2>/dev/null || stat -f '%z' "$file" 2>/dev/null)
    if (( bytes >= 1073741824 )); then
        printf "%.1f GB" "$(echo "scale=1; $bytes / 1073741824" | bc)"
    elif (( bytes >= 1048576 )); then
        printf "%.0f MB" "$(echo "scale=0; $bytes / 1048576" | bc)"
    else
        printf "%.0f KB" "$(echo "scale=0; $bytes / 1024" | bc)"
    fi
}

# --- Per-model eval threshold ---
# Each model produces different similarity distributions, so each needs its
# own threshold to correctly separate hits from misses.
DEFAULT_THRESHOLD="0.55"

model_threshold() {
    local label="$1"
    case "$label" in
        all-MiniLM-L6-v2) echo "0.90" ;;
        pplx-embed-*)     echo "0.45" ;;
        *)                echo "$DEFAULT_THRESHOLD" ;;
    esac
}

# --- Discover models ---
MODELS=()
LABELS=()
TOKENIZERS=()
THRESHOLDS=()
SIZES=()

# Always include hash backend
MODELS+=("")
LABELS+=("hash")
TOKENIZERS+=("")
THRESHOLDS+=("$DEFAULT_THRESHOLD")
SIZES+=("-")

# Auto-detect all GGUF files in models/
for gguf in "$MODELS_DIR"/*.gguf; do
    [[ -f "$gguf" ]] || continue
    tok="$(find_tokenizer "$gguf")"
    if [[ -z "$tok" ]]; then
        echo "WARNING: no tokenizer found for $gguf, skipping"
        continue
    fi
    label="$(basename "$gguf" .gguf)"
    MODELS+=("$gguf")
    LABELS+=("$label")
    TOKENIZERS+=("$tok")
    THRESHOLDS+=("$(model_threshold "$label")")
    SIZES+=("$(human_size "$gguf")")
done

# Auto-detect all safetensors files in models/
for st in "$MODELS_DIR"/*.safetensors; do
    [[ -f "$st" ]] || continue
    tok="$(find_tokenizer "$st")"
    if [[ -z "$tok" ]]; then
        echo "WARNING: no tokenizer found for $st, skipping"
        continue
    fi
    label="$(basename "$st" .safetensors)"
    MODELS+=("$st")
    LABELS+=("$label")
    TOKENIZERS+=("$tok")
    THRESHOLDS+=("$(model_threshold "$label")")
    SIZES+=("$(human_size "$st")")
done

echo "Detected backends: ${LABELS[*]}"
echo ""

# --- Build indexes ---
echo "=== Building indexes ==="
for i in "${!MODELS[@]}"; do
    label="${LABELS[$i]}"
    model="${MODELS[$i]}"
    tokenizer="${TOKENIZERS[$i]}"
    index="$BENCH_DIR/index_${label}.jsonl"

    echo -n "  Building index for '$label' ... "

    if [[ -z "$model" ]]; then
        "$FAQ_CLI" build-index \
            --input "$DATA_DIR/faq_seed.jsonl" \
            --output "$index" 2>/dev/null
    else
        "$FAQ_CLI" \
            --model-path "$model" \
            --tokenizer-path "$tokenizer" \
            build-index \
            --input "$DATA_DIR/faq_seed.jsonl" \
            --output "$index" 2>/dev/null
    fi

    echo "done ($index)"
done

echo ""

# --- Run evals ---
echo "=== Running evals ==="

# Collect summary data for the final table
SUMMARY_LABELS=()
SUMMARY_THRESHOLDS=()
SUMMARY_PASS_RATES=()
SUMMARY_PASSED=()
SUMMARY_TOTAL=()
SUMMARY_AVG_LATENCIES=()
SUMMARY_STATUSES=()
SUMMARY_SIZES=()

for i in "${!MODELS[@]}"; do
    label="${LABELS[$i]}"
    model="${MODELS[$i]}"
    tokenizer="${TOKENIZERS[$i]}"
    threshold="${THRESHOLDS[$i]}"
    index="$BENCH_DIR/index_${label}.jsonl"

    echo "--- $label (threshold=$threshold) ---"

    if [[ -z "$model" ]]; then
        eval_output="$("$FAQ_CLI" eval \
            --index "$index" \
            --cases "$BENCH_DIR/eval_cases.json" \
            --threshold "$threshold" 2>/dev/null)"
    else
        eval_output="$("$FAQ_CLI" \
            --model-path "$model" \
            --tokenizer-path "$tokenizer" \
            eval \
            --index "$index" \
            --cases "$BENCH_DIR/eval_cases.json" \
            --threshold "$threshold" 2>/dev/null)"
    fi

    echo "$eval_output"
    echo ""

    # Parse summary fields from eval output
    pass_rate=$(echo "$eval_output" | grep -o 'pass_rate=[0-9.]*' | head -1 | cut -d= -f2)
    passed=$(echo "$eval_output" | grep -o 'passed=[0-9]*' | head -1 | cut -d= -f2)
    total=$(echo "$eval_output" | grep -o 'total=[0-9]*' | head -1 | cut -d= -f2)
    avg_lat=$(echo "$eval_output" | grep -o 'avg_latency=[0-9.]*ms' | head -1 | cut -d= -f2)
    status=$(echo "$eval_output" | grep -o 'status=[A-Za-z]*' | head -1 | cut -d= -f2)

    SUMMARY_LABELS+=("$label")
    SUMMARY_THRESHOLDS+=("$threshold")
    SUMMARY_PASS_RATES+=("${pass_rate:-?}")
    SUMMARY_PASSED+=("${passed:-?}")
    SUMMARY_TOTAL+=("${total:-?}")
    SUMMARY_AVG_LATENCIES+=("${avg_lat:-?}")
    SUMMARY_STATUSES+=("${status:-?}")
    SUMMARY_SIZES+=("${SIZES[$i]}")
done

# --- Summary table ---
echo "========================================"
echo "=== Summary ==="
echo "========================================"
printf "%-35s %10s %10s %12s %12s %12s\n" "Backend" "Size" "Threshold" "Pass Rate" "Passed" "Avg Latency"
printf "%-35s %10s %10s %12s %12s %12s\n" "---" "---" "---" "---" "---" "---"
for i in "${!SUMMARY_LABELS[@]}"; do
    printf "%-35s %10s %10s %12s %12s %12s\n" \
        "${SUMMARY_LABELS[$i]}" \
        "${SUMMARY_SIZES[$i]}" \
        "${SUMMARY_THRESHOLDS[$i]}" \
        "${SUMMARY_PASS_RATES[$i]}" \
        "${SUMMARY_PASSED[$i]}/${SUMMARY_TOTAL[$i]}" \
        "${SUMMARY_AVG_LATENCIES[$i]}"
done
