# Firecracker rootfs reflink-staging benchmark

This is filesystem-reflink staging groundwork for the Firecracker backend. It
does not yet provide ZFS/devmapper snapshots, instant clone/reset APIs, or a
fixed end-to-end latency guarantee. Hosts without reflink support continue to
use a full byte-for-byte copy.

Firecracker uses an ext4 image file as its root drive.  AgentKernel prepares a
private image per sandbox using a filesystem reflink when `cp --reflink` is
available; otherwise it uses the existing full-copy behavior.  Overlayfs is
reported during capability detection but is not mounted for image files.

The following commands compare preparation time and disk usage without
claiming a fixed latency.  Run them on the same host and filesystem, with a
representative rootfs image:

```bash
ROOTFS=/path/to/base.ext4
WORK=$(mktemp -d)

hyperfine --warmup 2 \
  "cp --reflink=always \"$ROOTFS\" \"$WORK/reflink.ext4\"" \
  "cp \"$ROOTFS\" \"$WORK/full-copy.ext4\""

du -h "$ROOTFS" "$WORK/reflink.ext4" "$WORK/full-copy.ext4"
rm -rf "$WORK"
```

For an end-to-end measurement, run a Firecracker sandbox with the same image
and record the elapsed time from `sandbox start` through guest-agent readiness.
Keep the host, image, vCPU, memory, and kernel constant when comparing runs.
The implementation falls back automatically if reflinks are unsupported. The
Firecracker backend logs the selected `Reflink` or `FullCopy` strategy; record
that diagnostic rather than inferring it from elapsed time alone.

`AGENTKERNEL_ROOTFS_COW_DIR` may point to a new private directory or an
existing directory owned by the current user with mode `0700`. Existing
permissive directories are rejected rather than chmodded in place.
