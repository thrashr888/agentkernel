#!/bin/bash
# Run examples script for agentkernel
#
# Tests each example by:
# 1. Creating a sandbox for the example
# 2. Starting the sandbox
# 3. Running a simple test command
# 4. Cleaning up
#
# Usage:
#   ./scripts/run-examples.sh           # Run all examples
#   ./scripts/run-examples.sh python    # Run specific example
#   ./scripts/run-examples.sh --list    # List available examples

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
EXAMPLES_DIR="$PROJECT_DIR/examples"
cd "$PROJECT_DIR"

AK="$PROJECT_DIR/target/release/agentkernel"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Build if needed
if [[ ! -f "$AK" ]]; then
    echo "Building agentkernel..."
    cargo build --release --quiet
fi

# Get list of examples (directories with agentkernel.toml)
get_examples() {
    for dir in "$EXAMPLES_DIR"/*/; do
        if [[ -f "$dir/agentkernel.toml" ]]; then
            basename "$dir"
        fi
    done
}

# List examples
if [[ "${1:-}" == "--list" ]]; then
    echo "Available examples:"
    for example in $(get_examples); do
        echo "  - $example"
    done
    exit 0
fi

# Run a single example
run_example() {
    local example="$1"
    local example_dir="$EXAMPLES_DIR/$example"
    local sandbox_name="example-$example"

    if [[ ! -d "$example_dir" ]]; then
        echo -e "${RED}Error: Example '$example' not found${NC}"
        return 1
    fi

    echo -e "${BLUE}=== Running example: $example ===${NC}"
    echo ""

    # Handle error example specially - it's expected to fail
    if [[ "$example" == "error-app" ]]; then
        echo "  (This example is expected to fail)"
        echo "  Creating sandbox..."
        local config_file="$example_dir/agentkernel.toml"
        if $AK create "$sandbox_name" --agent claude --config "$config_file" 2>&1; then
            echo "  Starting sandbox..."
            if $AK start "$sandbox_name" 2>&1; then
                echo -e "${RED}  ERROR: Expected failure but sandbox started!${NC}"
                $AK remove "$sandbox_name" 2>/dev/null || true
                return 1
            fi
        fi
        echo -e "${GREEN}  Error correctly detected and handled${NC}"
        $AK remove "$sandbox_name" 2>/dev/null || true
        echo ""
        echo -e "${GREEN}Example 'error-app' passed (expected failure)${NC}"
        return 0
    fi

    # Cleanup any existing sandbox
    $AK remove "$sandbox_name" 2>/dev/null || true

    echo "  Creating sandbox..."
    local config_file="$example_dir/agentkernel.toml"
    if ! $AK create "$sandbox_name" --agent claude --config "$config_file" 2>&1; then
        echo -e "${RED}  Failed to create sandbox${NC}"
        return 1
    fi

    echo "  Starting sandbox..."
    if ! $AK start "$sandbox_name" 2>&1; then
        echo -e "${RED}  Failed to start sandbox${NC}"
        $AK remove "$sandbox_name" 2>/dev/null || true
        return 1
    fi

    # Run language-specific test
    echo "  Running test command..."
    local test_output
    local test_passed=false

    case "$example" in
        python*)
            test_output=$($AK exec "$sandbox_name" python3 --version 2>&1) && test_passed=true
            ;;
        node*|js*)
            test_output=$($AK exec "$sandbox_name" node --version 2>&1) && test_passed=true
            ;;
        typescript*)
            test_output=$($AK exec "$sandbox_name" node --version 2>&1) && test_passed=true
            ;;
        go*)
            test_output=$($AK exec "$sandbox_name" go version 2>&1) && test_passed=true
            ;;
        rust*)
            test_output=$($AK exec "$sandbox_name" rustc --version 2>&1) && test_passed=true
            ;;
        ruby*)
            test_output=$($AK exec "$sandbox_name" ruby --version 2>&1) && test_passed=true
            ;;
        java*)
            test_output=$($AK exec "$sandbox_name" java --version 2>&1) && test_passed=true
            ;;
        c-*)
            test_output=$($AK exec "$sandbox_name" gcc --version 2>&1) && test_passed=true
            ;;
        dotnet*)
            test_output=$($AK exec "$sandbox_name" dotnet --version 2>&1) && test_passed=true
            ;;
        bash*)
            test_output=$($AK exec "$sandbox_name" sh --version 2>&1 || $AK exec "$sandbox_name" echo "shell ok" 2>&1) && test_passed=true
            ;;
        *)
            test_output=$($AK exec "$sandbox_name" echo "Hello from $example" 2>&1) && test_passed=true
            ;;
    esac

    if [[ "$test_passed" == "true" ]]; then
        echo -e "  ${GREEN}Test passed:${NC} $test_output"
    else
        echo -e "  ${RED}Test failed${NC}"
    fi

    # Cleanup
    echo "  Stopping sandbox..."
    $AK stop "$sandbox_name" >/dev/null 2>&1 || true

    echo "  Removing sandbox..."
    $AK remove "$sandbox_name" >/dev/null 2>&1 || true

    echo ""
    if [[ "$test_passed" == "true" ]]; then
        echo -e "${GREEN}Example '$example' completed successfully${NC}"
        return 0
    else
        echo -e "${RED}Example '$example' failed${NC}"
        return 1
    fi
}

# Main
echo "=== Agentkernel Examples Runner ==="
echo ""

# Check backend
if [[ -e /dev/kvm ]]; then
    echo "Backend: Firecracker (KVM)"
else
    echo "Backend: Docker"
fi
echo ""

# Get examples to run
if [[ $# -gt 0 && "$1" != "--list" ]]; then
    examples=("$@")
else
    mapfile -t examples < <(get_examples)
fi

# Track results
passed=0
failed=0
declare -a failed_examples

# Run examples
for example in "${examples[@]}"; do
    if run_example "$example"; then
        ((passed++))
    else
        ((failed++))
        failed_examples+=("$example")
    fi
    echo ""
    echo "---"
    echo ""
done

# Summary
echo "=== Summary ==="
echo ""
echo -e "Passed: ${GREEN}$passed${NC}"
echo -e "Failed: ${RED}$failed${NC}"

if [[ $failed -gt 0 ]]; then
    echo ""
    echo "Failed examples:"
    for ex in "${failed_examples[@]}"; do
        echo -e "  ${RED}- $ex${NC}"
    done
    exit 1
fi

echo ""
echo "=== All examples passed ==="
