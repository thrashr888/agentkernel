#!/bin/bash
# Agentkernel installer
#
# This script installs agentkernel and its dependencies.
# Run with: curl -fsSL https://raw.githubusercontent.com/thrashr888/agentkernel/main/install.sh | sh
#
# After installation, run: agentkernel setup

set -euo pipefail

REPO="thrashr888/agentkernel"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"

echo "=== Agentkernel Installer ==="
echo ""

# Detect OS and architecture
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "$ARCH" in
    x86_64) ARCH="x86_64" ;;
    aarch64|arm64) ARCH="aarch64" ;;
    *)
        echo "Error: Unsupported architecture: $ARCH"
        exit 1
        ;;
esac

echo "Detected: $OS/$ARCH"

# Check for Rust/Cargo (required for now since we don't have prebuilt binaries)
if command -v cargo &>/dev/null; then
    echo ""
    echo "Installing via Cargo..."
    cargo install --git "https://github.com/$REPO" agentkernel

    echo ""
    echo "=== Installation Complete ==="
    echo ""
    echo "Next step: Run 'agentkernel setup' to download required components."
    echo ""
    exit 0
fi

# No Cargo - try to download prebuilt binary (not available yet)
echo ""
echo "Error: Rust/Cargo not found."
echo ""
echo "Install Rust first:"
echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
echo ""
echo "Then run this installer again, or install directly with:"
echo "  cargo install --git https://github.com/$REPO agentkernel"
echo ""
exit 1
