#!/bin/bash
# Entrypoint for agentkernel KVM host container
#
# This script sets up the environment for running Firecracker microVMs.

set -e

# Check for KVM support
if [ -e /dev/kvm ]; then
    echo "KVM is available"
    # Ensure proper permissions
    chmod 666 /dev/kvm 2>/dev/null || true
else
    echo "WARNING: /dev/kvm not found. Firecracker requires KVM support."
    echo "On macOS, ensure Docker Desktop has 'Use Virtualization.framework' enabled"
    echo "and 'Use Rosetta for x86_64/amd64 emulation' disabled for best compatibility."
fi

# Create runtime directories
mkdir -p /var/run/agentkernel

# Print environment info
echo "Agentkernel KVM Host"
echo "===================="
echo "Firecracker: $(firecracker --version 2>&1 || echo 'not found')"
echo "Kernel: $(ls /opt/agentkernel/images/kernel/ 2>/dev/null || echo 'none')"
echo "Rootfs: $(ls /opt/agentkernel/images/rootfs/ 2>/dev/null || echo 'none')"
echo ""

# Execute the command
exec "$@"
