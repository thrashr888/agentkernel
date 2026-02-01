
# Kubernetes Backend

Run sandboxes as Kubernetes Pods on any cluster. Each sandbox is a Pod running `sleep infinity` that accepts commands via the K8s exec API.

## Quick Start

```bash
# Create and run a sandbox on Kubernetes
agentkernel create my-sandbox --backend kubernetes --image alpine:3.20
agentkernel start my-sandbox
agentkernel exec my-sandbox -- echo "hello from k8s"
agentkernel stop my-sandbox
```

Or use `run` for ephemeral one-shot execution:

```bash
agentkernel run --backend kubernetes --image python:3.12-alpine -- python -c "print('hello')"
```

## Configuration

```toml
[orchestrator]
provider = "kubernetes"
namespace = "agentkernel"           # K8s namespace (default: "agentkernel")
kubeconfig = "~/.kube/config"       # Optional, auto-detected
context = "my-cluster"              # Optional kubeconfig context
runtime_class = "gvisor"            # Optional: "gvisor", "kata"
service_account = "agentkernel-sa"  # Optional service account
warm_pool_size = 10                 # Pre-warmed pods (default: 10)
max_pool_size = 50                  # Maximum total pods (default: 50)
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `namespace` | string | `agentkernel` | Kubernetes namespace for sandbox pods |
| `kubeconfig` | string | auto-detected | Path to kubeconfig file |
| `context` | string | current context | Kubeconfig context to use |
| `runtime_class` | string | none | RuntimeClass for stronger isolation |
| `service_account` | string | none | Service account for sandbox pods |
| `warm_pool_size` | int | 10 | Number of pre-warmed idle pods |
| `max_pool_size` | int | 50 | Maximum concurrent pods |

## Client Configuration

The Kubernetes backend resolves credentials in order:

1. In-cluster service account (when running inside K8s)
2. `kubeconfig` path from config
3. `KUBECONFIG` environment variable
4. `~/.kube/config`

## Security

Each sandbox pod runs with:

- `privileged: false`
- `allowPrivilegeEscalation: false`
- `runAsNonRoot: true`, `runAsUser: 1000`
- All capabilities dropped (`drop: ["ALL"]`)
- `automountServiceAccountToken: false`
- Pod Security Standards: `restricted`

When `network: false`, a `NetworkPolicy` is automatically created that denies all ingress and egress for the sandbox pod. The policy is cleaned up on `stop`.

For stronger isolation, set `runtime_class` to `gvisor` or `kata` to run pods in a dedicated kernel sandbox.

## Warm Pool

The Kubernetes warm pool pre-creates pods labeled `agentkernel/pool=warm`. When you call `acquire()`, a warm pod is relabeled to `active` and returned immediately. When released, the pod is deleted and a replacement is created.

A background task runs every 30 seconds to maintain the target warm count.

## Verifying with kubectl

```bash
# List agentkernel pods
kubectl get pods -n agentkernel -l agentkernel/managed-by=agentkernel

# Check a specific sandbox pod
kubectl describe pod agentkernel-my-sandbox -n agentkernel

# View pod labels
kubectl get pod agentkernel-my-sandbox -n agentkernel --show-labels
```

## Operator and CRDs (Optional)

For Kubernetes-native management, agentkernel provides Custom Resource Definitions.

### AgentSandbox CRD

```yaml
apiVersion: agentkernel/v1alpha1
kind: AgentSandbox
metadata:
  name: my-sandbox
spec:
  image: python:3.12-alpine
  vcpus: 2
  memory_mb: 1024
  network: true
  read_only: false
  runtime_class: gvisor
  security_profile: moderate
  env:
    - name: API_KEY
      value: "sk-..."
```

The operator watches `AgentSandbox` resources and creates/manages pods automatically. Status is reported back to the CR:

```bash
kubectl get agentsandboxes
kubectl describe agentsandbox my-sandbox
```

### AgentSandboxPool CRD

```yaml
apiVersion: agentkernel/v1alpha1
kind: AgentSandboxPool
metadata:
  name: default-pool
spec:
  warm_pool_size: 20
  max_pool_size: 100
  image: alpine:3.20
  vcpus: 1
  memory_mb: 512
```

### Generating CRD Manifests

```rust
use agentkernel::backend::kubernetes_operator::generate_crd_manifests;

let (sandbox_crd, pool_crd) = generate_crd_manifests()?;
std::fs::write("sandbox-crd.yaml", sandbox_crd)?;
std::fs::write("pool-crd.yaml", pool_crd)?;
```

## Deployment

For running agentkernel as a service on Kubernetes, see the [Deployment Guide](deploy.md) for Helm chart installation instructions.
