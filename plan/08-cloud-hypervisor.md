# ADR 08: Cloud Hypervisor Backend

## Status
Accepted

## Context
AgentKernel currently uses Firecracker as its primary Linux microVM backend. While Firecracker is excellent for extremely fast, lightweight, and secure sandboxing, there are use cases where users require features not supported by Firecracker, such as:
1. `virtio-fs` for high-performance shared file systems between the host and guest.
2. Support for slightly heavier workloads that require more complex device models.
3. Leveraging standard Cloud-init or Ignition for VM bootstrapping.

Cloud Hypervisor is a modern, Rust-based VMM that is optimized for cloud workloads. Adding it as an optional backend provides users with an alternative that strikes a balance between the extreme minimalism of Firecracker and the full-featured capabilities of QEMU.

## Decision
We will add `CloudHypervisor` as an official backend type in AgentKernel.
This backend will:
- Implement the unified `Sandbox` trait in `src/backend/cloud_hypervisor.rs`.
- Utilize the `cloud-hypervisor` binary via its API/CLI for lifecycle management.
- Coexist with the `Firecracker` backend, allowing users to choose the right VMM for their specific use case using `--backend cloudhypervisor`.

## Consequences
- **Positive**: Users gain access to `virtio-fs` and other advanced VMM features. Greater flexibility on Linux hosts.
- **Negative**: Increased maintenance surface. We must ensure `cloud-hypervisor` binaries are available (either via `agentkernel setup` or system PATH) and keep up with its API changes.
- **Neutral**: The networking and storage models will need to accommodate the specific requirements of Cloud Hypervisor, which may differ slightly from Firecracker.

## Implementation Details
1. Add `CloudHypervisor` to `BackendType`.
2. Implement `CloudHypervisorSandbox`.
3. Add CLI fallback and selection logic.
