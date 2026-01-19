#!/bin/bash
# End-to-end test for agentkernel
#
# This script tests the full CLI workflow:
# 1. Build the binary
# 2. Run setup (downloads/builds all components)
# 3. Create, start, exec, stop, remove a sandbox
#
# On systems without KVM, only tests up to sandbox creation.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_DIR"

AK="$PROJECT_DIR/target/release/agentkernel"

echo "=== Agentkernel End-to-End Test ==="
echo ""

# Check KVM and Docker availability
HAS_KVM=false
HAS_DOCKER=false
if [[ -e /dev/kvm ]]; then
    HAS_KVM=true
    echo "KVM: available"
else
    echo "KVM: not available"
fi
if command -v docker &>/dev/null && docker version &>/dev/null; then
    HAS_DOCKER=true
    echo "Docker: available"
else
    echo "Docker: not available"
fi

if [[ "$HAS_KVM" == "false" && "$HAS_DOCKER" == "false" ]]; then
    echo "Neither KVM nor Docker available - cannot run execution tests"
fi
echo ""

echo "1. Building agentkernel..."
cargo build --release

echo ""
echo "2. Checking status..."
$AK status

echo ""
echo "3. Running setup (non-interactive)..."
$AK setup -y

echo ""
echo "4. Verifying setup completed..."
$AK status
echo ""

echo "5. Cleaning up any existing test sandbox..."
$AK remove test-e2e 2>/dev/null || true

echo ""
echo "6. Initializing config in temp directory..."
TEST_DIR=$(mktemp -d)
cd "$TEST_DIR"
$AK init --name test-e2e --agent claude
ls -la agentkernel.toml
cat agentkernel.toml

echo ""
echo "7. Creating sandbox..."
$AK create test-e2e --agent claude

echo ""
echo "8. Listing sandboxes..."
$AK list

if [[ "$HAS_KVM" == "true" || "$HAS_DOCKER" == "true" ]]; then
    BACKEND="KVM"
    if [[ "$HAS_KVM" == "false" ]]; then
        BACKEND="Docker"
    fi

    echo ""
    echo "9. Starting sandbox (using $BACKEND backend)..."
    $AK start test-e2e

    echo ""
    echo "10. Listing sandboxes (should show running)..."
    $AK list

    echo ""
    echo "11. Executing command in sandbox..."
    $AK exec test-e2e echo "Hello from agentkernel!"

    echo ""
    echo "12. Stopping sandbox..."
    $AK stop test-e2e
else
    echo ""
    echo "9-12. Skipping execution tests (neither KVM nor Docker available)"
fi

echo ""
echo "13. Removing sandbox..."
$AK remove test-e2e

echo ""
echo "14. Verifying sandbox removed..."
$AK list

# Cleanup temp directory
cd "$PROJECT_DIR"
rm -rf "$TEST_DIR"

echo ""
if [[ "$HAS_KVM" == "true" ]]; then
    echo "=== E2E Test Complete (Full - KVM backend) ==="
elif [[ "$HAS_DOCKER" == "true" ]]; then
    echo "=== E2E Test Complete (Full - Docker backend) ==="
else
    echo "=== E2E Test Complete (Partial - no execution) ==="
    echo ""
    echo "To run the full test with sandbox execution:"
    echo "  - Run on Linux with /dev/kvm available"
    echo "  - Or install Docker"
fi
