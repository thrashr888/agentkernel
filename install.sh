#!/bin/bash
# Agentkernel installer
#
# This script installs agentkernel and its dependencies.
# Run with: curl -fsSL https://raw.githubusercontent.com/thrashr888/agentkernel/main/install.sh | sh
#
# After installation, run: agentkernel setup
#
# Environment variables:
#   INSTALL_DIR  - Where to install the binary (default: ~/.local/bin)
#   USE_CARGO=1  - Force building from source via cargo instead of downloading

set -euo pipefail

REPO="thrashr888/agentkernel"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"

echo "=== Agentkernel Installer ==="
echo ""

# Detect OS and architecture
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "$ARCH" in
    x86_64) ARCH_LABEL="x64" ;;
    aarch64|arm64) ARCH_LABEL="arm64" ;;
    *)
        echo "Error: Unsupported architecture: $ARCH"
        exit 1
        ;;
esac

echo "Detected: $OS/$ARCH_LABEL"

# Force cargo build if requested
if [ "${USE_CARGO:-}" = "1" ]; then
    if ! command -v cargo &>/dev/null; then
        echo "Error: USE_CARGO=1 but cargo not found."
        exit 1
    fi
    echo ""
    echo "Installing via Cargo (forced)..."
    cargo install --git "https://github.com/$REPO" agentkernel --force
    echo ""
    echo "=== Installation Complete ==="
    echo "Next step: Run 'agentkernel setup' to download required components."
    exit 0
fi

# Download prebuilt binary from GitHub releases
ASSET_NAME="agentkernel-${OS}-${ARCH_LABEL}.tar.gz"
echo "Downloading prebuilt binary: $ASSET_NAME"

# Get latest release download URL
DOWNLOAD_URL="https://github.com/$REPO/releases/latest/download/$ASSET_NAME"

# Create install directory
mkdir -p "$INSTALL_DIR"

# Download and extract
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

if command -v curl &>/dev/null; then
    curl -fsSL "$DOWNLOAD_URL" -o "$TMPDIR/$ASSET_NAME"
elif command -v wget &>/dev/null; then
    wget -q "$DOWNLOAD_URL" -O "$TMPDIR/$ASSET_NAME"
else
    echo "Error: Neither curl nor wget found."
    exit 1
fi

tar xzf "$TMPDIR/$ASSET_NAME" -C "$TMPDIR"

# Find the binary (may be in a subdirectory)
BINARY="$(find "$TMPDIR" -name agentkernel -type f | head -1)"
if [ -z "$BINARY" ]; then
    echo "Error: agentkernel binary not found in archive."
    exit 1
fi

chmod +x "$BINARY"
mv "$BINARY" "$INSTALL_DIR/agentkernel"

echo ""
echo "=== Installation Complete ==="
echo ""
echo "Installed to: $INSTALL_DIR/agentkernel"
echo "Version: $("$INSTALL_DIR/agentkernel" --version 2>/dev/null || echo 'unknown')"
echo ""

# Check if install dir is in PATH
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
        echo "NOTE: $INSTALL_DIR is not in your PATH."
        echo "Add it with: export PATH=\"$INSTALL_DIR:\$PATH\""
        echo ""
        ;;
esac

echo "Next step: Run 'agentkernel setup' to download required components."
echo ""
