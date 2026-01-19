#!/bin/bash
# Benchmark script for agentkernel sandbox operations
#
# Measures timing of:
# - Create sandbox
# - Start sandbox
# - Execute commands
# - Stop sandbox
# - Remove sandbox

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_DIR"

AK="$PROJECT_DIR/target/release/agentkernel"
BENCH_NAME="bench-sandbox"
ITERATIONS="${1:-5}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "=== Agentkernel Benchmark ==="
echo ""
echo "Iterations: $ITERATIONS"
echo ""

# Build if needed
if [[ ! -f "$AK" ]]; then
    echo "Building agentkernel..."
    cargo build --release --quiet
fi

# Check backend
BACKEND="Unknown"
if [[ -e /dev/kvm ]]; then
    BACKEND="Firecracker (KVM)"
else
    BACKEND="Docker"
fi
echo "Backend: $BACKEND"
echo ""

# Cleanup any existing benchmark sandbox
$AK remove "$BENCH_NAME" 2>/dev/null || true

# Function to measure time in milliseconds
# Works on both Linux and macOS
measure() {
    local start end
    if [[ "$OSTYPE" == "darwin"* ]]; then
        # macOS: use perl for millisecond timing
        start=$(perl -MTime::HiRes=time -e 'printf "%.0f", time*1000')
        "$@" >/dev/null 2>&1
        end=$(perl -MTime::HiRes=time -e 'printf "%.0f", time*1000')
    else
        # Linux: use date with nanoseconds
        start=$(date +%s%N | cut -b1-13)
        "$@" >/dev/null 2>&1
        end=$(date +%s%N | cut -b1-13)
    fi
    echo $((end - start))
}

# Arrays to store timings
declare -a create_times
declare -a start_times
declare -a exec_times
declare -a stop_times
declare -a remove_times

echo "Running benchmarks..."
echo ""

for i in $(seq 1 "$ITERATIONS"); do
    echo -n "  Iteration $i/$ITERATIONS: "

    # Create
    create_time=$(measure $AK create "$BENCH_NAME" --agent claude)
    create_times+=("$create_time")
    echo -n "create=${create_time}ms "

    # Start
    start_time=$(measure $AK start "$BENCH_NAME")
    start_times+=("$start_time")
    echo -n "start=${start_time}ms "

    # Exec (run a simple command)
    exec_time=$(measure $AK exec "$BENCH_NAME" echo "hello")
    exec_times+=("$exec_time")
    echo -n "exec=${exec_time}ms "

    # Stop
    stop_time=$(measure $AK stop "$BENCH_NAME")
    stop_times+=("$stop_time")
    echo -n "stop=${stop_time}ms "

    # Remove
    remove_time=$(measure $AK remove "$BENCH_NAME")
    remove_times+=("$remove_time")
    echo "remove=${remove_time}ms"
done

echo ""

# Calculate averages
calc_avg() {
    local sum=0
    local count=0
    for val in "$@"; do
        sum=$((sum + val))
        count=$((count + 1))
    done
    echo $((sum / count))
}

calc_min() {
    local min=${1}
    for val in "$@"; do
        if [[ $val -lt $min ]]; then
            min=$val
        fi
    done
    echo "$min"
}

calc_max() {
    local max=${1}
    for val in "$@"; do
        if [[ $val -gt $max ]]; then
            max=$val
        fi
    done
    echo "$max"
}

echo "=== Results ==="
echo ""
printf "%-12s %8s %8s %8s\n" "Operation" "Avg(ms)" "Min(ms)" "Max(ms)"
printf "%-12s %8s %8s %8s\n" "---------" "------" "------" "------"
printf "%-12s %8d %8d %8d\n" "Create" "$(calc_avg "${create_times[@]}")" "$(calc_min "${create_times[@]}")" "$(calc_max "${create_times[@]}")"
printf "%-12s %8d %8d %8d\n" "Start" "$(calc_avg "${start_times[@]}")" "$(calc_min "${start_times[@]}")" "$(calc_max "${start_times[@]}")"
printf "%-12s %8d %8d %8d\n" "Exec" "$(calc_avg "${exec_times[@]}")" "$(calc_min "${exec_times[@]}")" "$(calc_max "${exec_times[@]}")"
printf "%-12s %8d %8d %8d\n" "Stop" "$(calc_avg "${stop_times[@]}")" "$(calc_min "${stop_times[@]}")" "$(calc_max "${stop_times[@]}")"
printf "%-12s %8d %8d %8d\n" "Remove" "$(calc_avg "${remove_times[@]}")" "$(calc_min "${remove_times[@]}")" "$(calc_max "${remove_times[@]}")"

# Calculate total cycle time
total_avg=$(($(calc_avg "${create_times[@]}") + $(calc_avg "${start_times[@]}") + $(calc_avg "${exec_times[@]}") + $(calc_avg "${stop_times[@]}") + $(calc_avg "${remove_times[@]}")))
echo ""
printf "%-12s %8d ms\n" "Full Cycle" "$total_avg"

echo ""
echo "=== Benchmark Complete ==="
