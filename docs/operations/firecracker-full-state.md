# Firecracker full-state pause, resume, and fork

AgentKernel can preserve a Firecracker microVM's guest memory, process state,
device state, and writable root filesystem, stop its VMM process, and later
resume the sandbox. The same immutable checkpoint can also seed independent
running children while the source remains paused and reusable.

This is a different contract from `agentkernel snapshot`. Existing snapshots
are filesystem or provider snapshots: they preserve files, but a restore boots
new processes. Full-state pause preserves the point of execution.

## Initial support boundary

The initial full-state implementation is deliberately narrow:

- Linux on x86_64 with readable and writable `/dev/kvm`;
- Firecracker `v1.16.1`;
- the AgentKernel `6.18.45` guest kernel; and
- restore on an exactly compatible host, described below.

Treat the capability as a preview until the native KVM gate at the end of this
page has passed on the exact release build. Compilation and mocked state-machine
tests on macOS are not evidence that guest memory, processes, or disks survive
on KVM.

Docker, Podman, Apple Containers, Hyperlight, Kubernetes, Nomad, and hosted
backends do not implement this full-state contract. They return an explicit
unsupported-capability error. AgentKernel never silently substitutes a
filesystem snapshot, because that would discard live processes, open file
descriptors, and guest memory while reporting success.

On backends that implement them, use the existing `snapshot take` and
`snapshot restore` commands when a cold, filesystem-level branch is acceptable.
Those commands are not a Firecracker full-state fallback.

## Lifecycle

Firecracker VMM processes must be owned by a long-lived AgentKernel process.
Start `agentkernel serve` first and keep it running; the CLI pause, resume, and
fork commands delegate to that local server. A standalone CLI manager cannot
safely retain or reconnect a Firecracker process after the command exits, so
these operations fail with an actionable error when the server is unavailable.
The server must also own the running source VM that will be paused.

Firecracker `start`, `stop`, and `remove` requests also execute as server-owned
tasks. If an HTTP client disconnects, cancellation of that waiter does not
cancel the lifecycle mutation. `agentkernel run --keep ...` leaves its VM under
daemon ownership; without `--keep`, cleanup is a server-owned remove rather
than a lossy local stop.

```bash
# Keep this process running (18888 is the default port).
agentkernel serve --host 127.0.0.1 --port 18888
```

The delegated CLI/MCP control path currently speaks plaintext HTTP on loopback.
A TLS-only (`--require-tls`) listener is deliberately not mistaken for a healthy
control endpoint; run the lifecycle daemon on a host-private loopback port
without `--require-tls`. TLS-aware or Unix-socket delegation is tracked before
this preview is suitable for a TLS-only service topology.

Then, from another terminal:

```bash
# Capture memory, VM state, and disk, then stop compute.
agentkernel sandbox pause research-agent
# `suspend` is a visible alias for `pause`.

# Create and immediately run an independent child from the source checkpoint.
agentkernel sandbox fork research-agent --as candidate-b

# The source remains paused, so it can seed another child or resume itself.
agentkernel sandbox fork research-agent --as candidate-c
agentkernel sandbox resume research-agent
```

`pause` is transactional while the owning service remains alive. AgentKernel
first records a deterministic transition ID, pauses the running microVM, stages
the full checkpoint, and publishes its manifest only after every required
artifact is ready. A failure before publication retries the live source in
place; an ambiguous double failure remains visibly paused under manager
ownership so `resume` can retry it. A successful pause leaves the sandbox in
the `paused` state and no longer consumes VMM compute.

`fork` keeps the source paused and starts the child immediately. Each child gets
a fresh Firecracker process, an independent writable disk, and unique API and
vsock Unix sockets, then loads the saved VM state. The source checkpoint remains
reusable for another fork or for resuming the source. Its immutable memory file
may still back private, demand-paged mappings in running children.

## Compatibility is exact, not best effort

A Firecracker checkpoint contains KVM and emulated-device state. AgentKernel
records the Firecracker version, CPU architecture, host kernel release, a
SHA-256 host-identity fingerprint, a SHA-256 CPU/feature fingerprint, and the
guest's exact `uname -r`. It refuses any mismatch before starting a VMM. Raw
machine identity and `/proc/cpuinfo` contents are not written to the manifest.

For the first release, treat all of these as restore requirements:

| Dimension | Requirement |
| --- | --- |
| Backend | Firecracker only; never restore to a different backend |
| Firecracker | Exact `v1.16.1` runtime |
| Snapshot format | Firecracker snapshot format `10.0.0`, produced by the 1.16 line |
| Architecture | `x86_64` only |
| Host kernel | Exact release recorded at pause time |
| Host identity | Exact machine-identity hash recorded at pause time (same-host restore) |
| CPU | Exact fingerprint of vendor, family, model, stepping, microcode, and guest-visible feature flags |
| Guest | Exact `6.18.45-agentkernel` release reported by the guest and the same device/boot configuration |

Firecracker documents cross-host-kernel snapshot restore as unstable, and CPU
compatibility depends on the features exposed to the guest. A same-architecture
label alone is not a portability guarantee. The machine-identity check makes
this initial contract intentionally same-host; portable CPU templates and a
cross-host validation matrix remain future work.

Firecracker `v1.16.0` and `v1.16.1` use the same snapshot format, but
`v1.16.1` contains a vsock-after-restore fix. AgentKernel pins the exact patch
release rather than inferring safety from the format version alone.

Relevant upstream references:

- [Firecracker snapshot support](https://github.com/firecracker-microvm/firecracker/blob/v1.16.1/docs/snapshotting/snapshot-support.md)
- [Snapshot versioning and host/CPU compatibility](https://github.com/firecracker-microvm/firecracker/blob/v1.16.1/docs/snapshotting/versioning.md)
- [Firecracker 1.16.1 changelog](https://github.com/firecracker-microvm/firecracker/blob/v1.16.1/CHANGELOG.md)

## Artifact invariants

A usable checkpoint is a set, not a single file:

- `vmstate.bin` contains Firecracker and KVM device state;
- `memory.bin` contains guest memory; and
- `rootfs.ext4` is the disk state captured while the VM is paused.

Firecracker does not package or manage the disk image. Its snapshot state also
retains block-device paths. AgentKernel therefore launches snapshot-capable VMs
from a per-sandbox runtime directory and uses a stable relative disk path. Each
resume or fork resolves that path inside its own runtime directory, where it
has an independent copy-on-write disk.

The state, memory, and disk artifacts are immutable. Firecracker maps the
memory file privately and reads pages on demand. AgentKernel records a SHA-256
digest and byte length for every artifact and verifies both before restore. It
keeps a paused source's checkpoint intact while it can still seed forks, and
removes a consumed source checkpoint only after Firecracker has accepted its
restore. Operators must not manually delete or replace reusable checkpoint
files. Mutating a memory file can corrupt restored guests and produces
undefined behavior.

Full snapshots write the complete configured guest memory. Operators must
budget snapshot storage by memory size plus disk size, account for concurrent
forks, and leave headroom for failed staging attempts. Firecracker's CRC covers
only the VM state file; it is not authentication for the memory or disk files.
Checkpoint directories and their contents must be restricted to the AgentKernel
service identity and protected like credentials when backed up or moved.

Before pausing, AgentKernel reserves conservatively for configured RAM, the
logical rootfs size, and 64 MiB of state/metadata overhead. The checkpoint store
defaults to a 64 GiB global cap and requires 5 GiB of filesystem headroom. Set
`AGENTKERNEL_FULL_STATE_MAX_BYTES` and
`AGENTKERNEL_FULL_STATE_MIN_FREE_BYTES` to positive byte counts to tune those
limits. The daemon serializes this check with checkpoint publication, so two
pause requests cannot both pass against the same observed capacity. These are
host-wide safety limits, not per-tenant storage accounting or garbage
collection.

Every VMM instance receives a private runtime directory under
`/tmp/agentkernel-fc-<uid>` with mode `0700`. API and vsock Unix sockets are
accepted only when they are real sockets owned by the service user; AgentKernel
rejects symlinks and changes the socket mode to `0600` before connecting. This
protects local control endpoints from other host users but does not replace
normal service-host isolation.

An interrupted transition remains in a deterministic
`.staging-<checkpoint-id>` directory and is reported when the service starts.
If a durable ready marker proves the original VMM had already terminated,
`resume` or `fork` can finish publishing it. Ambiguous staging is neither
restored nor deleted: safely reattaching to an orphaned Firecracker process
requires the durable supervisor/reconnect protocol described under known
limitations below.

Differential Firecracker snapshots are outside this contract. Upstream still
labels them developer preview, and a diff is generally not independently
resumable until it is merged with its base.

## Service ownership and crash recovery

The long-running `agentkernel serve` process owns Firecracker child processes
and their API/vsock sockets. CLI and MCP lifecycle commands delegate to that
service. AgentKernel can recover an ambiguous pause while that manager remains
alive and can identify deterministic staging after restart, but it cannot yet
reattach to an orphaned Firecracker process after a service crash. A durable
supervisor with PID identity, ownership locks, and startup reconciliation is
required before calling this lifecycle crash-resilient.

After a source is resumed or a child is forked, use `pause` rather than
ordinary `stop` when its writable filesystem must survive. AgentKernel rejects
ordinary stop for these full-state lineages until durable Firecracker disk
lineage is implemented; `remove` remains the explicit discard operation.

## Resume side effects and clone safety

Full-state continuation does not mean every external connection survives:

- Firecracker resets vsock across snapshot creation and restore. Existing
  connections close; guest listen sockets remain and the AgentKernel guest
  agent accepts a new host connection.
- Network and vsock packet loss is expected, and established network
  connections are not guaranteed to survive.
- On x86_64, AgentKernel restores with Firecracker's `clock_realtime` option so
  guest wall-clock time advances across the pause. That jump can surprise
  software, so time behavior must still be tested for the workload.
- Logging and metrics configuration are not part of the Firecracker snapshot
  and must be recreated by the host.

Firecracker updates VMGenID before resuming vCPUs. Linux 5.18 and newer uses
that notification to reseed its in-kernel random-number generator; the supported
6.18.45 guest is new enough for this path. VMGenID does **not** deduplicate
userspace state. Cached random values, one-time tokens, application IDs,
`/proc/sys/kernel/random/boot_id`, SSH host keys, and any credentials already in
guest memory or on disk can be identical in every fork.

Treat `fork` as an explicit security boundary. Workloads that consume unique or
one-time state need a cooperative post-restore hook or a pre-snapshot quiescence
protocol. The API and MCP responses warn that guest memory and filesystem state,
including credentials, are cloned.

Do not take a reusable checkpoint during early guest boot. Firecracker warns
that VMGenID interrupt handling might not yet be ready and the restored guest
can crash.

## Host proxy limitation

The AgentKernel secret and model-governance proxy runs on the host; it is not
part of the microVM snapshot. Processes restored from memory retain their old
environment and open sockets, while a restarted host proxy can have a different
endpoint. Full-state continuation therefore cannot currently promise transparent
proxy-secret or model-governance continuity.

Until a stable endpoint and rebind protocol is implemented and KVM-tested,
AgentKernel rejects full-state pause, resume, and fork for a sandbox with
host-side secret bindings or a governance proxy. Secrets supplied directly as
guest environment variables or files are even more sensitive: they are copied
into the checkpoint and duplicated into every fork.

## Upgrade and recovery policy

Before replacing Firecracker, the host kernel, or CPU configuration:

1. list paused sandboxes and their compatibility metadata;
2. resume and shut down work that must survive the upgrade, or retain the exact
   runtime required by those checkpoints;
3. upgrade and run the native KVM gate; and
4. only then make the new runtime the default for newly paused sandboxes.

AgentKernel must not attempt an incompatible full-state load and then fall back
to a cold boot. A mismatch should leave the checkpoint intact and return an
actionable error. Firecracker terminates its process when snapshot load fails,
so a retry must use a fresh VMM process.

## Test matrix

| Layer | Required coverage | Where it runs | Release meaning |
| --- | --- | --- | --- |
| API serialization | `PATCH /vm` pause/resume; full snapshot create; load with `mem_backend: File`, `vsock_override`, `resume_vm: false`, and the clock policy | Every pull request | Request shape only |
| Compatibility contract | Manifest round trip; exact VMM, architecture, host identity, host kernel, CPU fingerprint, and guest-kernel rejection; legacy filesystem snapshot separation | Every pull request | Deterministic policy only |
| Unsupported backends | Docker, Podman, Apple, Hyperlight, Kubernetes, Nomad, and hosted backends fail without creating checkpoint artifacts | Every pull request where the backend is compiled | No silent downgrade |
| Transaction failures | Failure after pause, memory/state creation, disk clone, and manifest staging either resumes the source or retains explicit recovery ownership and deterministic staging | Unit tests with injected failures | State-machine safety |
| Artifact lifecycle | Immutable memory/state/disk, SHA-256 validation, independent child disks, and reference-aware deletion | Linux filesystem tests | Storage safety without KVM |
| Native resume | A RAM-only value and the same guest process survive pause/resume; filesystem and guest-agent/vsock access still work | Dedicated x86_64 Linux KVM runner | Required for the feature claim |
| Native fork | Two children load concurrently from one checkpoint, receive unique host sockets, and diverge on independent disks without changing the source | Dedicated x86_64 Linux KVM runner | Required for the fork claim |
| Failure recovery | Corrupt/missing artifacts fail cleanly, leave the checkpoint retryable, and do not strand a VMM | Dedicated x86_64 Linux KVM runner | Required before release |
| Clone hygiene | VMGenID-capable guest, random reseed observation, documented duplicate userspace identity, clock behavior, and proxy limitation | Dedicated x86_64 Linux KVM runner | Security/operations evidence |

Unit tests on macOS do not validate KVM, Firecracker API behavior, guest process
continuity, or concurrent forks.

### Opt-in native KVM gate

The existing native test is ignored by default. On the access-controlled runner
labelled `self-hosted,Linux,X64,agentkernel-kvm-safe`, provision the pinned
binary and guest assets, then run:

```bash
AGENTKERNEL_KVM_SMOKE=1 \
FIRECRACKER_BIN=/opt/agentkernel/bin/firecracker \
AGENTKERNEL_KVM_KERNEL=/opt/agentkernel/images/kernel/vmlinux-6.18.45-agentkernel \
AGENTKERNEL_KVM_ROOTFS=/opt/agentkernel/images/rootfs/base.ext4 \
cargo test --locked --test firecracker_kvm_smoke -- --ignored --nocapture
```

The runner must have readable and writable `/dev/kvm`. The test must verify
Firecracker reports `v1.16.1`, and its cleanup path must execute even when an
assertion fails. Dispatching the repository workflow requires explicit runner
confirmation:

```bash
gh workflow run firecracker-kvm-smoke.yml -f confirm_safe_runner=true
```

Record the successful run URL before marking native suspend/resume/fork as
validated. A compiled ignored test, a queued workflow, or a macOS test run is
not KVM runtime evidence.
