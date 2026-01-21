#!/bin/bash
# Build minimal rootfs for Firecracker microVMs
#
# Usage: ./build-rootfs.sh [runtime]
# Example: ./build-rootfs.sh base
#          ./build-rootfs.sh python
#
# Requirements:
#   - Docker (for cross-platform builds)
#
# Output: images/rootfs/<runtime>.ext4

set -euo pipefail

RUNTIME="${1:-base}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$(dirname "$SCRIPT_DIR")")"
ROOTFS_DIR="$(dirname "$SCRIPT_DIR")/rootfs"
OUTPUT="$ROOTFS_DIR/${RUNTIME}.ext4"
SIZE_MB=256  # Default size
AGENT_BIN="$PROJECT_ROOT/target/x86_64-unknown-linux-musl/release/agent"

echo "==> Building rootfs for runtime: $RUNTIME"

# Step 1: Build guest-agent for musl (static binary)
echo "==> Building guest-agent for musl target..."
# Always rebuild to ensure it's up to date
echo "    Cross-compiling guest-agent..."
mkdir -p "$PROJECT_ROOT/target/x86_64-unknown-linux-musl/release"
docker run --rm \
    -v "$PROJECT_ROOT:/project" \
    -v "$PROJECT_ROOT/target:/project/target" \
    -w /project/guest-agent \
    rust:1.85-alpine \
    sh -c 'apk add --no-cache musl-dev && rustup target add x86_64-unknown-linux-musl && cargo build --release --target x86_64-unknown-linux-musl'

if [ ! -f "$AGENT_BIN" ]; then
    echo "ERROR: Failed to build guest-agent"
    exit 1
fi
echo "    Guest agent built: $(ls -lh "$AGENT_BIN" | awk '{print $5}')"

# Create rootfs directory
mkdir -p "$ROOTFS_DIR"

# Build in Docker for consistency
docker build -t agentkernel-rootfs-builder -f - "$SCRIPT_DIR" << 'DOCKERFILE'
FROM alpine:3.20

RUN apk add --no-cache \
    e2fsprogs \
    e2fsprogs-extra \
    dosfstools \
    coreutils \
    bash

WORKDIR /build
DOCKERFILE

# Create the rootfs based on runtime type
case "$RUNTIME" in
    base)
        SIZE_MB=64
        PACKAGES=""
        ;;
    python)
        SIZE_MB=256
        PACKAGES="python3 py3-pip"
        ;;
    node)
        SIZE_MB=256
        PACKAGES="nodejs npm"
        ;;
    go)
        SIZE_MB=512
        PACKAGES="go"
        ;;
    rust)
        SIZE_MB=512
        PACKAGES="rust cargo"
        ;;
    *)
        echo "Unknown runtime: $RUNTIME"
        echo "Available: base, python, node, go, rust"
        exit 1
        ;;
esac

echo "    Size: ${SIZE_MB}MB"
echo "    Packages: ${PACKAGES:-none}"

# Create rootfs image
docker run --rm --privileged \
    -v "$ROOTFS_DIR:/output" \
    -v "$AGENT_BIN:/agent-bin:ro" \
    -e RUNTIME="$RUNTIME" \
    -e SIZE_MB="$SIZE_MB" \
    -e PACKAGES="$PACKAGES" \
    agentkernel-rootfs-builder /bin/bash -c '
set -euo pipefail

ROOTFS_IMG="/output/${RUNTIME}.ext4"
MOUNT_DIR="/mnt/rootfs"

echo "==> Creating ${SIZE_MB}MB ext4 image..."
dd if=/dev/zero of="$ROOTFS_IMG" bs=1M count=$SIZE_MB status=progress
mkfs.ext4 -F "$ROOTFS_IMG"

echo "==> Mounting and populating rootfs..."
mkdir -p "$MOUNT_DIR"
mount -o loop "$ROOTFS_IMG" "$MOUNT_DIR"

# Create Alpine rootfs using static busybox approach
echo "==> Installing Alpine base system..."
apk -X https://dl-cdn.alpinelinux.org/alpine/v3.20/main \
    -X https://dl-cdn.alpinelinux.org/alpine/v3.20/community \
    -U --allow-untrusted --root "$MOUNT_DIR" --initdb \
    add alpine-base busybox-static $PACKAGES

# Create essential directories
mkdir -p "$MOUNT_DIR"/{dev,proc,sys,tmp,run,root,app}
chmod 1777 "$MOUNT_DIR/tmp"

# Create device nodes
mknod -m 622 "$MOUNT_DIR/dev/console" c 5 1 || true
mknod -m 666 "$MOUNT_DIR/dev/null" c 1 3 || true
mknod -m 666 "$MOUNT_DIR/dev/zero" c 1 5 || true
mknod -m 666 "$MOUNT_DIR/dev/tty" c 5 0 || true
mknod -m 666 "$MOUNT_DIR/dev/random" c 1 8 || true
mknod -m 666 "$MOUNT_DIR/dev/urandom" c 1 9 || true

# Copy guest agent binary
echo "==> Installing guest agent..."
cp /agent-bin "$MOUNT_DIR/usr/bin/agent"
chmod +x "$MOUNT_DIR/usr/bin/agent"

# Create init script that starts the agent
cat > "$MOUNT_DIR/init" << '\''INIT'\''
#!/bin/busybox sh

# Mount essential filesystems
/bin/busybox mount -t proc proc /proc
/bin/busybox mount -t sysfs sysfs /sys
/bin/busybox mount -t devtmpfs devtmpfs /dev 2>/dev/null || true

# Set hostname
/bin/busybox hostname agentkernel

# Start guest agent in background
echo "Starting agentkernel guest agent..."
/usr/bin/agent &

echo "Agentkernel guest ready"

# If no arguments, run shell (for debugging)
if [ $# -eq 0 ]; then
    exec /bin/busybox sh
else
    exec "$@"
fi
INIT
chmod +x "$MOUNT_DIR/init"

# Set up /etc files
echo "agentkernel" > "$MOUNT_DIR/etc/hostname"
echo "root:x:0:0:root:/root:/bin/sh" > "$MOUNT_DIR/etc/passwd"
echo "root:x:0:" > "$MOUNT_DIR/etc/group"

# Clean up
umount "$MOUNT_DIR"

echo "==> Rootfs created: $ROOTFS_IMG"
ls -lh "$ROOTFS_IMG"
'

echo ""
echo "==> Rootfs build complete!"
echo "    Output: $OUTPUT"
echo ""
