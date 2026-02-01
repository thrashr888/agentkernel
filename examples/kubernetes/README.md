# Kubernetes Example

Run agentkernel sandboxes as Kubernetes Pods.

## Prerequisites

- Kubernetes cluster (k3d, kind, minikube, or remote)
- `kubectl` configured with cluster access
- agentkernel built with `--features kubernetes`

## Quick Start with k3d

```bash
# Create a local cluster
k3d cluster create agentkernel

# Build agentkernel with Kubernetes support
cargo build --features kubernetes

# Create and run a sandbox
agentkernel create k8s-sandbox --backend kubernetes --image alpine:3.20
agentkernel start k8s-sandbox
agentkernel exec k8s-sandbox -- echo "hello from kubernetes"

# Verify with kubectl
kubectl get pods -n agentkernel -l agentkernel.io/managed-by=agentkernel

# Clean up
agentkernel stop k8s-sandbox
```

## Using the Config File

```bash
agentkernel create k8s-sandbox --config examples/kubernetes/agentkernel.toml
agentkernel start k8s-sandbox
agentkernel exec k8s-sandbox -- uname -a
agentkernel stop k8s-sandbox
```

## Configuration

See `agentkernel.toml` in this directory. Key settings:

- `[orchestrator].namespace` — Kubernetes namespace for sandbox pods
- `[orchestrator].runtime_class` — Set to `gvisor` or `kata` for stronger isolation
- `[orchestrator].warm_pool_size` — Number of pre-warmed idle pods

## Security

Sandbox pods run with:
- Non-root user (UID 1000)
- All capabilities dropped
- Read-only root filesystem (restrictive profile)
- NetworkPolicy denying all traffic when `network = false`
