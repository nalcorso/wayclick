#!/usr/bin/env bash
# bench-report.sh — Run benchmarks and save a timestamped JSON report.
#
# Usage: ./scripts/bench-report.sh [--quick]
#   --quick: Run benchmarks with reduced sample size for CI/smoke testing
#
# Output: bench-results/<timestamp>-<commit>.json

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RESULTS_DIR="$REPO_ROOT/bench-results"
mkdir -p "$RESULTS_DIR"

TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"
COMMIT="$(git -C "$REPO_ROOT" rev-parse --short HEAD 2>/dev/null || echo "unknown")"
COMMIT_FULL="$(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || echo "unknown")"
BRANCH="$(git -C "$REPO_ROOT" rev-parse --abbrev-ref HEAD 2>/dev/null || echo "unknown")"
DIRTY="$(git -C "$REPO_ROOT" diff --quiet 2>/dev/null && echo "false" || echo "true")"

REPORT_FILE="$RESULTS_DIR/${TIMESTAMP}-${COMMIT}.json"

# Collect system info
RUSTC_VERSION="$(rustc --version 2>/dev/null || echo "unknown")"
TARGET_TRIPLE="$(rustc -vV 2>/dev/null | grep 'host:' | awk '{print $2}' || echo "unknown")"
KERNEL="$(uname -r)"
CPU_MODEL="$(grep -m1 'model name' /proc/cpuinfo 2>/dev/null | cut -d: -f2 | xargs || echo "unknown")"
CPU_CORES="$(nproc 2>/dev/null || echo "unknown")"
CPU_GOVERNOR="$(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor 2>/dev/null || echo "unknown")"
TOTAL_MEM_KB="$(grep MemTotal /proc/meminfo 2>/dev/null | awk '{print $2}' || echo "0")"
TOTAL_MEM_MB=$((TOTAL_MEM_KB / 1024))

echo "=== Wayclick Benchmark Report ==="
echo "Timestamp: $TIMESTAMP"
echo "Commit:    $COMMIT_FULL (dirty=$DIRTY)"
echo "Branch:    $BRANCH"
echo "Rust:      $RUSTC_VERSION"
echo "Target:    $TARGET_TRIPLE"
echo "CPU:       $CPU_MODEL ($CPU_CORES cores, governor=$CPU_GOVERNOR)"
echo "Memory:    ${TOTAL_MEM_MB}MB"
echo ""

# Parse optional flags
CRITERION_ARGS=""
if [[ "${1:-}" == "--quick" ]]; then
    CRITERION_ARGS="--sample-size 10 --warm-up-time 1 --measurement-time 3"
    echo "Quick mode: reduced sample size"
fi

# Run Criterion benchmarks with JSON output
echo "Running benchmarks..."
CRITERION_OUTPUT="$RESULTS_DIR/.criterion-output-$$"

# Run each benchmark group and capture output
for BENCH in action_execution config_loading ipc_framing; do
    echo "  → $BENCH"
    # shellcheck disable=SC2086
    cargo bench \
        --features bench-internals \
        --bench "$BENCH" \
        -- \
        --color never \
        $CRITERION_ARGS \
        2>&1 | tee -a "$CRITERION_OUTPUT"
done

# Collect peak RSS of the last benchmark run (from /proc/self/status during bench)
# Note: This measures the benchmark binary's RSS, not wayclick's
PEAK_RSS_KB=0
if [[ -f /proc/self/status ]]; then
    PEAK_RSS_KB=$(grep VmHWM /proc/self/status 2>/dev/null | awk '{print $2}' || echo "0")
fi

# Parse Criterion output into structured results
# Criterion outputs lines like:
#   do_click/instant        time:   [123.45 ns 125.00 ns 126.50 ns]
#   execute_action_sync/click
#                           time:   [1.2345 µs 1.2500 µs 1.2650 µs]
echo ""
echo "Parsing results..."

parse_criterion_results() {
    local output_file="$1"
    local first=true
    local current_bench=""

    echo "["

    while IFS= read -r line; do
        # Match benchmark name (may appear on its own line or with time on same line)
        if [[ "$line" =~ ^([a-zA-Z_][a-zA-Z0-9_/]*) ]]; then
            # Check if this line also has time data
            if [[ "$line" =~ time:.*\[([0-9.]+)\ (ns|µs|ms|s)\ +([0-9.]+)\ (ns|µs|ms|s)\ +([0-9.]+)\ (ns|µs|ms|s)\] ]]; then
                current_bench="${BASH_REMATCH[1]%%[[:space:]]*}"
                # Handled below
            else
                current_bench="${BASH_REMATCH[1]}"
                current_bench="${current_bench%%[[:space:]]*}"
                continue
            fi
        fi

        # Match time line
        if [[ "$line" =~ time:.*\[([0-9.]+)\ (ns|µs|ms|s)\ +([0-9.]+)\ (ns|µs|ms|s)\ +([0-9.]+)\ (ns|µs|ms|s)\] ]]; then
            if [[ -z "$current_bench" ]]; then
                continue
            fi

            local low="${BASH_REMATCH[1]}"
            local low_unit="${BASH_REMATCH[2]}"
            local mid="${BASH_REMATCH[3]}"
            local mid_unit="${BASH_REMATCH[4]}"
            local high="${BASH_REMATCH[5]}"
            local high_unit="${BASH_REMATCH[6]}"

            # Convert to nanoseconds for consistency
            to_ns() {
                local val="$1" unit="$2"
                case "$unit" in
                    ns) echo "$val" ;;
                    µs|"µs") echo "$val * 1000" | bc ;;
                    ms) echo "$val * 1000000" | bc ;;
                    s)  echo "$val * 1000000000" | bc ;;
                esac
            }

            local low_ns high_ns mid_ns
            low_ns=$(to_ns "$low" "$low_unit")
            mid_ns=$(to_ns "$mid" "$mid_unit")
            high_ns=$(to_ns "$high" "$high_unit")

            if [[ "$first" != "true" ]]; then
                echo ","
            fi
            first=false

            printf '    {"name": "%s", "lower_ns": %s, "estimate_ns": %s, "upper_ns": %s}' \
                "$current_bench" "$low_ns" "$mid_ns" "$high_ns"

            current_bench=""
        fi
    done < "$output_file"

    echo ""
    echo "  ]"
}

BENCH_RESULTS=$(parse_criterion_results "$CRITERION_OUTPUT")

# Build final JSON report
cat > "$REPORT_FILE" <<EOF
{
  "schema_version": 1,
  "timestamp": "$TIMESTAMP",
  "git": {
    "commit": "$COMMIT_FULL",
    "commit_short": "$COMMIT",
    "branch": "$BRANCH",
    "dirty": $DIRTY
  },
  "system": {
    "rustc": "$RUSTC_VERSION",
    "target": "$TARGET_TRIPLE",
    "kernel": "$KERNEL",
    "cpu_model": "$CPU_MODEL",
    "cpu_cores": $CPU_CORES,
    "cpu_governor": "$CPU_GOVERNOR",
    "total_memory_mb": $TOTAL_MEM_MB
  },
  "peak_rss_kb": $PEAK_RSS_KB,
  "benchmarks": $BENCH_RESULTS
}
EOF

# Clean up
rm -f "$CRITERION_OUTPUT"

echo ""
echo "=== Report saved to: $REPORT_FILE ==="
echo ""

# Show summary
echo "Benchmark Summary:"
echo "━━━━━━━━━━━━━━━━━"
python3 -c "
import json, sys
with open('$REPORT_FILE') as f:
    data = json.load(f)
for b in data.get('benchmarks', []):
    est = b['estimate_ns']
    if est >= 1e9:
        val, unit = est / 1e9, 's'
    elif est >= 1e6:
        val, unit = est / 1e6, 'ms'
    elif est >= 1e3:
        val, unit = est / 1e3, 'µs'
    else:
        val, unit = est, 'ns'
    print(f'  {b[\"name\"]:<45} {val:>8.2f} {unit}')
" 2>/dev/null || echo "(install python3 for formatted summary)"

echo ""
echo "Done."
