# Agentkernel VM Images

Pre-built kernel and rootfs images for Firecracker microVMs.

Fresh setup defaults to Firecracker `v1.16.1` and Linux `6.18.45`. Firecracker
1.16 is the first supported line for the 6.18 host kernel, and 1.16.1 is the
minimum supported release for the 6.18 guest kernel. The setup command keeps an
already-installed Firecracker binary and any existing `vmlinux-*` image; an
explicit kernel version passed to `build-kernel.sh` also remains authoritative.

Release metadata is sourced from the upstream [Firecracker release policy](https://github.com/firecracker-microvm/firecracker/blob/main/docs/RELEASE_POLICY.md),
[Firecracker kernel policy](https://github.com/firecracker-microvm/firecracker/blob/main/docs/kernel-policy.md),
and [kernel.org releases](https://www.kernel.org/releases.html). Downloads are
verified against the upstream SHA-256 values before extraction.

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
./build-kernel.sh 6.18.45

# Existing explicit selections remain supported. The checksum is resolved from
# kernel.org, or can be supplied as a second argument for a custom source.
./build-kernel.sh 6.18.44
```

### Using Docker (Any Platform)

```bash
cd images/build
docker build -t agentkernel-kernel-builder -f Dockerfile.kernel-builder .
docker run -v $(pwd)/../kernel:/output agentkernel-kernel-builder 6.18.45
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

### Building Rootfs

The rootfs is built using Docker (works on any platform):

```bash
cd images/build

# Build base image (~64MB)
./build-rootfs.sh base

# Build with Python runtime (~256MB)
./build-rootfs.sh python

# Build with Node.js runtime (~256MB)
./build-rootfs.sh node
```

### Available Runtimes

| Runtime | Size | Contents |
|---------|------|----------|
| `base` | ~64MB | Alpine Linux, busybox, guest agent |
| `python` | ~256MB | Base + Python 3, pip |
| `node` | ~256MB | Base + Node.js, npm |
| `go` | ~512MB | Base + Go toolchain |
| `rust` | ~512MB | Base + Rust, Cargo |

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
