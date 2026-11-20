# Native Windows containers

Native Windows workloads and Linux containers hosted on Windows are separate
targets. AgentKernel's existing Docker backend runs Linux images and expects
Linux tools, paths, and policies. WSL2 with a Linux Docker engine is a Linux
target; it does not run Windows images. Check the server selected by the
current Docker context, not the OS of the client:

```text
docker version --format "{{.Server.Os}}"
```

A `windows` result is not ready for AgentKernel's Linux Docker backend.
Native Windows execution is not supported by this build.

## Proposed native runtime

Keep AgentKernel's control plane on a supported host initially. Add an explicit
Windows backend that delegates to a narrowly scoped worker on a dedicated
Windows Server machine. The worker owns a local Windows container runtime and
uses its supported Docker-compatible API or runtime adapter. Don't expose an
unauthenticated daemon socket or let arbitrary workspace users choose an
engine endpoint. Require authenticated worker registration, scoped lifecycle
requests and worker-owned container IDs. Never silently fall back to Linux
or process isolation.

Start with one compatible Windows Server host and Server Core image pinned by
digest. Verify the Windows container feature, architecture and host/image
build pair. Require Hyper-V isolation explicitly for all initial workloads and
verify it from the actual container state; Hyper-V on a virtual host also
requires nested virtualization. Keep network disabled by default and prove
that enforcement separately. No host mounts, engine pipes, device passthrough,
unrestricted environment inheritance or arbitrary runtime options in the first
slice. Use Windows-native paths and explicit executable/argument lists rather
than assuming a POSIX shell. Reject unsupported settings rather than
silently dropping them.

The first end-to-end gate is a benign run on a real Windows Server test host:
pull the pinned image, create a Hyper-V-isolated container, verify image OS and
actual isolation, run a command with a known output and exit code, collect
stdout/stderr, stop and remove by owned ID, and prove no orphan survives
timeout/cancellation. Reject a Linux engine, an incompatible image, and any
attempt to request weaker isolation. Verify retries cannot remove a container
with the same name that is not owned by the worker. Only then enable a Windows
agent image smoke test. Packaging a native Windows AgentKernel CLI or daemon
is independent work; the current daemon uses Unix sockets.

References: [Windows container requirements](https://learn.microsoft.com/en-us/virtualization/windowscontainers/deploy-containers/system-requirements),
[host/image compatibility](https://learn.microsoft.com/en-us/virtualization/windowscontainers/deploy-containers/version-compatibility),
[container security and isolation](https://learn.microsoft.com/en-us/virtualization/windowscontainers/manage-containers/container-security),
[Docker server version](https://docs.docker.com/reference/cli/docker/version/).
