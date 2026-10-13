# Cloud Hypervisor backend (experimental)

This backend is **disabled by default**. Set
`AGENTKERNEL_EXPERIMENTAL_CLOUD_HYPERVISOR=1` only to explicitly evaluate it.
Readiness and every public start path enforce this gate. Ordinary releases do
not enable it merely because a binary, KVM, and kernel are present. Real hardware
boot validation has not yet been completed; do not roll it out in production
until the opt-in gate below passes on the target host.

This backend currently targets **Linux x86_64 with writable `/dev/kvm`**, using
Cloud Hypervisor's KVM backend. Hyper-V enlightenments are passed to KVM; this does
not implement Microsoft Hypervisor/MSHV or native Windows support. macOS and
AArch64 are not supported by this integration yet.

## Assets and kernel provenance

`agentkernel setup` retains the existing checksum-verified kernel.org **6.18.45**
Firecracker kernel source build. It must not replace that kernel with an older
prebuilt image or label a different kernel as 6.18.45.

Cloud Hypervisor needs a **separate compatible kernel**. Set
`CLOUD_HYPERVISOR_KERNEL` to its absolute path. There is deliberately no fallback
to the Firecracker kernel: that configuration disables ACPI and does not provide
the required virtio PCI configuration. Do not pass `pci=off` to a Cloud Hypervisor
guest. For direct x86_64 boot use a PVH-enabled ELF kernel or bzImage with PCI,
virtio PCI, virtio block, virtio vsock, ext4, devtmpfs, and the console built in.
Follow the [upstream kernel build guidance](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v53.0/README.md#building-your-kernel),
using a maintained kernel version and verified source. Supplying a path does not
prove kernel compatibility; the opt-in boot gate below is required before rollout.

If the original PR's setup was previously run, a file named
`vmlinux-6.18.45-agentkernel` may actually contain the third-party 5.10.77 kernel.
This fix does not delete or overwrite existing user-provided assets. Verify its
provenance and explicitly replace a known mislabeled file with a source-built
kernel; the filename alone is not proof of version.

Install Cloud Hypervisor v53.0 through setup on a supported host, or set
`CLOUD_HYPERVISOR_BIN` to an executable. PATH discovery is also supported. Prepare
setup's base rootfs (`agentkernel setup --runtimes base`); it contains `/init` and
`/usr/bin/agent`. Readiness checks the binary, KVM access, explicit kernel, and base
rootfs before advertising this backend as usable.

## Supported contract

- Explicit selection: `--backend cloudhypervisor` or `--backend cloud-hypervisor`
- Use **network disabled** (`--no-network` for `run`), with host mounts and SSH disabled
- Setup images: `base` / `alpine:3.24`, `python` / `python:3.12`, `node` / `node:20`,
  `rust` / `rust:1.85`, `go` / `golang:1.24`; other images fail rather than becoming base
- Configured vCPU/memory limits, read-only root disk, guest command environment/workdir,
  file operations on writable roots, and stop/restart cleanup
- Private per-instance 0700 socket/log directory; no shared `/tmp/ch_error.log`

Networking, port mappings, host mounts, SSH, exec user switching, durable disk
resume, and snapshots are not implemented. Unsupported requests are rejected or
not advertised. Stop discards the transient COW disk. Use another backend when
these capabilities are needed. Read-only roots cannot accept injected files.

## Verification

Ordinary unit tests use mock processes/API sockets and never require KVM.
The real boot/exec/file-I/O/restart/read-only test is explicitly ignored by default;
it does not return success when prerequisites are missing:

```sh
AGENTKERNEL_EXPERIMENTAL_CLOUD_HYPERVISOR=1 \
AGENTKERNEL_CH_SMOKE=1 \
CLOUD_HYPERVISOR_BIN=/absolute/path/cloud-hypervisor \
CLOUD_HYPERVISOR_KERNEL=/absolute/path/compatible-kernel \
cargo test --locked --test cloud_hypervisor_smoke -- --ignored --nocapture
```

Run this on an authorized Linux x86_64/KVM host with a prepared base rootfs.
A unit-test pass or skipped hardware test is not a verified VM boot.
