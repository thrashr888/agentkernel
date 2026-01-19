# Agentkernel VM Images

Pre-built kernel and rootfs images for Firecracker microVMs.

## Directory Structure

```
images/
├── kernel/
│   ├── microvm.config          # Minimal kernel config for Firecracker
│   └── vmlinux-*-agentkernel   # Built kernel (after running build script)
├── rootfs/
│   └── (rootfs images go here)
└── build/
    ├── build-kernel.sh         # Kernel build script
    └── Dockerfile.kernel-builder
```

## Building the Kernel

### On Linux (Native)

Requirements:
- build-essential, bc, bison, flex, libelf-dev, libssl-dev, curl, xz-utils

```bash
cd images/build
./build-kernel.sh 6.1.70
```

### Using Docker (Any Platform)

```bash
cd images/build
docker build -t agentkernel-kernel-builder -f Dockerfile.kernel-builder .
docker run -v $(pwd)/../kernel:/output agentkernel-kernel-builder 6.1.70
```

### Output

The build produces `images/kernel/vmlinux-<version>-agentkernel` (~4-6MB).

## Kernel Configuration

The `microvm.config` is optimized for Firecracker:

- **Enabled**: virtio (blk, net, vsock), serial console, ext4, squashfs, overlayfs, networking
- **Disabled**: modules, USB, sound, graphics, WiFi, Bluetooth, NFS, debugging
- **Boot**: PVH entry point for fast boot (<125ms target)
- **Size**: ~4MB vmlinux

## Rootfs Images

(TODO: Document rootfs build process)

Target images:
- `base.ext4` - Minimal Alpine (~20MB)
- `python.ext4` - Python 3.12 runtime (~50MB)
- `node.ext4` - Node.js 20 LTS (~40MB)

## Testing

Run the stress test (requires Firecracker VMM implementation):

```bash
cargo test --test stress_test -- --nocapture --ignored
```

This spins up 100 VMs in parallel, runs `echo hello` in each, and validates output.

Target metrics:
- Boot time: <125ms per VM
- Total time for 100 VMs: <30s
- Memory overhead: <10MB per VM
