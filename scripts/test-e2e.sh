#!/bin/bash
# End-to-end test for agentkernel
#
# This script:
# 1. Builds the kernel (if not present)
# 2. Builds the base rootfs
# 3. Creates, starts, and tests a microVM
#
# Run on Linux with KVM, or inside Docker with --privileged

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_DIR"

echo "=== Agentkernel End-to-End Test ==="
echo ""

# Check for KVM
if [[ ! -e /dev/kvm ]]; then
    echo "ERROR: /dev/kvm not found. This test requires Linux with KVM."
    echo ""
    echo "On macOS, run inside Docker with --privileged:"
    echo "  docker run --privileged -it agentkernel-test"
    exit 1
fi

# Check for Firecracker
if ! command -v firecracker &>/dev/null; then
    echo "ERROR: Firecracker not found in PATH."
    echo "Download from: https://github.com/firecracker-microvm/firecracker/releases"
    exit 1
fi

echo "1. Building agentkernel..."
cargo build --release

echo ""
echo "2. Checking kernel..."
KERNEL=$(find images/kernel -name 'vmlinux-*-agentkernel' 2>/dev/null | head -1)
if [[ -z "$KERNEL" ]]; then
    echo "   Kernel not found, building..."
    cd images/build
    docker build -t agentkernel-kernel-builder -f Dockerfile.kernel-builder .
    docker run --rm -v "$(pwd)/../kernel:/kernel" agentkernel-kernel-builder 6.1.70
    cd "$PROJECT_DIR"
    KERNEL=$(find images/kernel -name 'vmlinux-*-agentkernel' | head -1)
fi
echo "   Kernel: $KERNEL"

echo ""
echo "3. Checking rootfs..."
if [[ ! -f images/rootfs/base.ext4 ]]; then
    echo "   Rootfs not found, building..."
    cd images/build
    ./build-rootfs.sh base
    cd "$PROJECT_DIR"
fi
echo "   Rootfs: images/rootfs/base.ext4"

echo ""
echo "4. Creating sandbox..."
./target/release/agentkernel create test-e2e --agent claude

echo ""
echo "5. Starting sandbox..."
./target/release/agentkernel start test-e2e

echo ""
echo "6. Listing sandboxes..."
./target/release/agentkernel list

echo ""
echo "7. Stopping sandbox..."
./target/release/agentkernel stop test-e2e

echo ""
echo "8. Removing sandbox..."
./target/release/agentkernel remove test-e2e

echo ""
echo "=== Test Complete ==="
