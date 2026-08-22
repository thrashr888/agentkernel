#!/bin/bash
# Build minimal Linux kernel for Firecracker microVMs
#
# Usage: ./build-kernel.sh [version]
# Example: ./build-kernel.sh 6.18.45
#
# For macOS, use Docker:
#   docker build -t agentkernel-kernel-builder -f Dockerfile.kernel-builder .
#   docker run --rm -v "$(pwd)/../kernel:/kernel" agentkernel-kernel-builder 6.18.45
#
# Requirements (Linux native):
#   - gcc, make, flex, bison, libelf-dev, libssl-dev, bc
#
# Output: /kernel/vmlinux-<version>-agentkernel (Docker)
#         images/kernel/vmlinux-<version>-agentkernel (native)

set -euo pipefail

# Default kernel version (6.18 LTS, supported by Firecracker 1.16)
# An explicit first argument remains authoritative for existing build workflows.
KERNEL_VERSION="${1:-6.18.45}"
DEFAULT_KERNEL_SHA256="30fa4a56579ca614ac125a12614f7f6466f87ab1278aef7b951dd74156deab33"
KERNEL_SHA256="${2:-}"
KERNEL_MAJOR="${KERNEL_VERSION%%.*}"

echo "==> Building kernel $KERNEL_VERSION for Firecracker"

# Detect if running in Docker (mounted /kernel) or native
if [[ -d "/kernel" && -w "/kernel" ]]; then
    # Docker mode - config and output at /kernel
    KERNEL_DIR="/kernel"
    echo "    Mode: Docker (output to /kernel)"
else
    # Native mode - relative to script location
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    KERNEL_DIR="$(dirname "$SCRIPT_DIR")/kernel"
    echo "    Mode: Native (output to $KERNEL_DIR)"
fi

BUILD_DIR="${BUILD_DIR:-/tmp/agentkernel-kernel-build}"

# Create build directory
mkdir -p "$BUILD_DIR"
cd "$BUILD_DIR"

# Download kernel source if not present
KERNEL_TARBALL="linux-${KERNEL_VERSION}.tar.xz"
KERNEL_URL="https://cdn.kernel.org/pub/linux/kernel/v${KERNEL_MAJOR}.x/${KERNEL_TARBALL}"

if [[ ! -f "$KERNEL_TARBALL" ]]; then
    echo "==> Downloading kernel source..."
    curl -fL --output "$KERNEL_TARBALL" "$KERNEL_URL"
fi

# Verify the pinned default against kernel.org's signed checksum manifest. For an
# explicit version, accept an explicit checksum or resolve it from that manifest.
if [[ -z "$KERNEL_SHA256" ]]; then
    if [[ "$KERNEL_VERSION" == "6.18.45" ]]; then
        KERNEL_SHA256="$DEFAULT_KERNEL_SHA256"
    else
        KERNEL_SHA256="$(curl -fsSL "https://cdn.kernel.org/pub/linux/kernel/v${KERNEL_MAJOR}.x/sha256sums.asc" \
            | awk -v file="$KERNEL_TARBALL" '$2 == file { print $1; exit }')"
    fi
fi

if [[ -z "$KERNEL_SHA256" ]]; then
    echo "ERROR: No official checksum found for $KERNEL_TARBALL"
    echo "Pass the expected SHA-256 as the second argument for a custom source."
    exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
    echo "$KERNEL_SHA256  $KERNEL_TARBALL" | sha256sum -c -
else
    ACTUAL_SHA256="$(shasum -a 256 "$KERNEL_TARBALL" | awk '{print $1}')"
    [[ "$ACTUAL_SHA256" == "$KERNEL_SHA256" ]] || {
        echo "ERROR: SHA-256 mismatch for $KERNEL_TARBALL"
        exit 1
    }
fi

# Extract if not already extracted
KERNEL_SRC="linux-${KERNEL_VERSION}"
if [[ ! -d "$KERNEL_SRC" ]]; then
    echo "==> Extracting kernel source..."
    tar xf "$KERNEL_TARBALL"
fi

cd "$KERNEL_SRC"

# Check for config file
CONFIG_FILE="$KERNEL_DIR/microvm.config"
if [[ ! -f "$CONFIG_FILE" ]]; then
    echo "ERROR: Config file not found: $CONFIG_FILE"
    echo "Make sure to mount the kernel directory with the config file."
    exit 1
fi

# Copy our minimal config
echo "==> Applying microvm config..."
cp "$CONFIG_FILE" .config

# Update config with defaults for any missing options
make olddefconfig

# Build the kernel
echo "==> Building kernel (this may take a few minutes)..."
NPROC=$(nproc 2>/dev/null || echo 4)
make -j"$NPROC" vmlinux

# Verify the kernel was built
if [[ ! -f "vmlinux" ]]; then
    echo "ERROR: vmlinux not found after build"
    exit 1
fi

# Copy to output directory
OUTPUT="$KERNEL_DIR/vmlinux-${KERNEL_VERSION}-agentkernel"
cp vmlinux "$OUTPUT"

# Show result
SIZE=$(du -h "$OUTPUT" | cut -f1)
echo ""
echo "==> Kernel build complete!"
echo "    Output: $OUTPUT"
echo "    Size: $SIZE"
echo ""
echo "To use with Firecracker:"
echo "    firecracker --kernel $OUTPUT --rootfs <rootfs.ext4>"
