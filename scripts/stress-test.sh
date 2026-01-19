#!/bin/bash
# Stress test script for agentkernel
#
# Measures throughput (commands per second) under load
# Similar to requests-per-second benchmarks for web servers
#
# Usage: ./scripts/stress-test.sh [total_commands] [concurrency]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_DIR"

AK="$PROJECT_DIR/target/release/agentkernel"
TOTAL_COMMANDS="${1:-100}"
CONCURRENCY="${2:-10}"
RESULTS_DIR="/tmp/agentkernel-stress-$$"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

echo -e "${BOLD}=== Agentkernel Stress Test ===${NC}"
echo ""
echo -e "Total commands:  ${CYAN}$TOTAL_COMMANDS${NC}"
echo -e "Concurrency:     ${CYAN}$CONCURRENCY${NC}"
echo ""

# Build if needed
if [[ ! -f "$AK" ]]; then
    echo "Building agentkernel (release)..."
    cargo build --release --quiet
fi

# Detect backend
BACKEND="Docker"
if command -v podman &>/dev/null && podman version &>/dev/null; then
    BACKEND="Podman"
fi
if [[ -e /dev/kvm ]]; then
    BACKEND="Firecracker"
fi
echo -e "Backend:         ${CYAN}$BACKEND${NC}"
echo ""

# Create results directory
mkdir -p "$RESULTS_DIR"

# Function to get time in milliseconds (cross-platform)
now_ms() {
    if [[ "$OSTYPE" == "darwin"* ]]; then
        perl -MTime::HiRes=time -e 'printf "%.0f", time*1000'
    else
        date +%s%3N
    fi
}

# Function to run a single command and record timing
run_command() {
    local id=$1
    local result_file="$RESULTS_DIR/result-$id.txt"

    local start=$(now_ms)

    # Run a simple command in a fresh sandbox
    if $AK run echo "stress-$id" >/dev/null 2>&1; then
        local status="success"
    else
        local status="failed"
    fi

    local end=$(now_ms)
    local duration=$((end - start))

    echo "$duration $status" > "$result_file"
}

# Warm up - pull images if needed
echo -e "${YELLOW}Warming up (pulling images if needed)...${NC}"
$AK run echo "warmup" >/dev/null 2>&1 || true
echo ""

# Run stress test
echo -e "${YELLOW}Running stress test...${NC}"
echo ""

START_TIME=$(now_ms)

# Track active jobs
active_jobs=0
completed=0

for i in $(seq 1 "$TOTAL_COMMANDS"); do
    # Start job in background
    run_command "$i" &
    active_jobs=$((active_jobs + 1))

    # If we hit concurrency limit, wait for one to finish
    if [[ $active_jobs -ge $CONCURRENCY ]]; then
        wait -n 2>/dev/null || true
        active_jobs=$((active_jobs - 1))
        completed=$((completed + 1))

        # Progress update every 10 completions
        if [[ $((completed % 10)) -eq 0 ]]; then
            pct=$((completed * 100 / TOTAL_COMMANDS))
            echo -ne "\r  Progress: $completed/$TOTAL_COMMANDS ($pct%)"
        fi
    fi
done

# Wait for remaining jobs
wait
echo -ne "\r  Progress: $TOTAL_COMMANDS/$TOTAL_COMMANDS (100%)\n"

END_TIME=$(now_ms)
TOTAL_TIME_MS=$((END_TIME - START_TIME))
TOTAL_TIME_SEC=$(echo "scale=2; $TOTAL_TIME_MS / 1000" | bc)

echo ""

# Collect results
declare -a latencies
successes=0
failures=0

for f in "$RESULTS_DIR"/result-*.txt; do
    if [[ -f "$f" ]]; then
        read -r duration status < "$f"
        latencies+=("$duration")
        if [[ "$status" == "success" ]]; then
            successes=$((successes + 1))
        else
            failures=$((failures + 1))
        fi
    fi
done

# Sort latencies for percentile calculations
IFS=$'\n' sorted_latencies=($(sort -n <<<"${latencies[*]}")); unset IFS

# Calculate statistics
calc_percentile() {
    local pct=$1
    local count=${#sorted_latencies[@]}
    local idx=$(( (count * pct / 100) - 1 ))
    if [[ $idx -lt 0 ]]; then idx=0; fi
    echo "${sorted_latencies[$idx]}"
}

calc_avg() {
    local sum=0
    for val in "${latencies[@]}"; do
        sum=$((sum + val))
    done
    echo $((sum / ${#latencies[@]}))
}

# Calculate throughput
throughput=$(echo "scale=2; $TOTAL_COMMANDS / $TOTAL_TIME_SEC" | bc)

# Print results
echo -e "${BOLD}=== Results ===${NC}"
echo ""
echo -e "${GREEN}Throughput${NC}"
printf "  Commands/sec:    %.2f\n" "$throughput"
printf "  Total time:      %.2f sec\n" "$TOTAL_TIME_SEC"
echo ""

echo -e "${GREEN}Success Rate${NC}"
printf "  Succeeded:       %d\n" "$successes"
printf "  Failed:          %d\n" "$failures"
success_rate=$(echo "scale=1; $successes * 100 / $TOTAL_COMMANDS" | bc)
printf "  Success rate:    %.1f%%\n" "$success_rate"
echo ""

max_idx=$((${#sorted_latencies[@]} - 1))
echo -e "${GREEN}Latency (ms)${NC}"
printf "  Avg:    %6d ms\n" "$(calc_avg)"
printf "  Min:    %6d ms\n" "${sorted_latencies[0]}"
printf "  p50:    %6d ms\n" "$(calc_percentile 50)"
printf "  p90:    %6d ms\n" "$(calc_percentile 90)"
printf "  p95:    %6d ms\n" "$(calc_percentile 95)"
printf "  p99:    %6d ms\n" "$(calc_percentile 99)"
printf "  Max:    %6d ms\n" "${sorted_latencies[$max_idx]}"
echo ""

# Cleanup
rm -rf "$RESULTS_DIR"

# Summary line for easy comparison
echo -e "${BOLD}=== Summary ===${NC}"
echo -e "  ${CYAN}$throughput cmd/s${NC} @ ${CYAN}${CONCURRENCY}${NC} concurrency (p50: $(calc_percentile 50)ms, p99: $(calc_percentile 99)ms)"
echo ""

# Suggest next steps
if [[ "$failures" -gt 0 ]]; then
    echo -e "${RED}Warning: $failures commands failed. Check Docker/Podman status.${NC}"
fi

echo -e "${BOLD}=== Stress Test Complete ===${NC}"
