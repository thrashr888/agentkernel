
# Kubernetes Backend

Run sandboxes as Kubernetes Pods on any cluster. Each sandbox is a Pod running `sleep infinity` that accepts commands via the K8s exec API.

## Quick Start

```bash
# Create and run a sandbox on Kubernetes
agentkernel sandbox create my-sandbox --backend kubernetes --image alpine:3.24
agentkernel sandbox start my-sandbox
agentkernel exec my-sandbox -- echo "hello from k8s"
agentkernel sandbox stop my-sandbox
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

When a sandbox declares one or more `ports`, the backend also creates a
per-sandbox `ClusterIP` Service. The Service selects the sandbox using the
`agentkernel/sandbox` label, preserves TCP/UDP mappings, and is removed with
the sandbox. Its deterministic name is visible through `kubectl get service`
and can be reached from the namespace through the normal Kubernetes DNS name
`<service>.<namespace>.svc`. This is internal service discovery only; expose a
sandbox outside the cluster with an Ingress, Gateway, or service mesh owned by
the application/operator.

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
  image: alpine:3.24
  vcpus: 1
  memory_mb: 512
```

### AgentKernelPolicy CRD (Enterprise)

Namespaced Cedar policy that applies to sandboxes in the same namespace. Requires the `enterprise` feature.

```yaml
apiVersion: agentkernel/v1alpha1
kind: AgentKernelPolicy
metadata:
  name: deny-network-staging
  namespace: staging
spec:
  cedar: |
    forbid(
        principal,
        action == AgentKernel::Action::"Network",
        resource
    );
  priority: 100
  description: "Block network access in staging"
```

Apply via `kubectl apply` — the operator validates Cedar syntax and reports status:

```bash
kubectl get akp -A          # List all namespace policies
kubectl describe akp deny-network-staging -n staging
```

### ClusterAgentKernelPolicy CRD (Enterprise)

Cluster-scoped Cedar policy that applies to all sandboxes globally.

```yaml
apiVersion: agentkernel/v1alpha1
kind: ClusterAgentKernelPolicy
metadata:
  name: default-permit
spec:
  cedar: |
    permit(
        principal is AgentKernel::User,
        action,
        resource is AgentKernel::Sandbox
    );
  priority: 0
  description: "Default permit for all authenticated users"
```

```bash
kubectl get cakp             # List cluster-wide policies
kubectl describe cakp default-permit
```

#### Policy Evaluation Order

1. Cluster-scoped policies are loaded first (lower scope weight)
2. Within the same scope, higher `priority` values take precedence
3. Cedar's default-deny model applies — if no `permit` matches, the action is denied
4. `forbid` rules always override `permit` rules regardless of priority

#### Policy Status

The operator sets status on each policy CR:

| Field | Description |
|-------|-------------|
| `valid` | Whether the Cedar syntax parsed successfully |
| `active` | Whether the policy is loaded in the evaluation engine |
| `message` | Error details when `valid: false` |
| `lastApplied` | Timestamp of last successful load |
| `observedGeneration` | Generation for change detection |

#### Identity from Sandbox Annotations

The policy engine reads principal identity from sandbox CR annotations:

| Annotation | Maps to | Default |
|------------|---------|---------|
| `agentkernel/user-id` | `Principal.id` | `anonymous` |
| `agentkernel/email` | `Principal.email` | `anonymous@unknown` |
| `agentkernel/org-id` | `Principal.org_id` | `default` |
| `agentkernel/roles` | `Principal.roles` (comma-separated) | `developer` |
| `agentkernel/mfa-verified` | `Principal.mfa_verified` | `false` |
| `agentkernel/agent-type` | `Resource.agent_type` | `unknown` |
| `agentkernel/runtime` | `Resource.runtime` | `unknown` |

### Generating CRD Manifests

```rust
use agentkernel::backend::kubernetes_operator::generate_crd_manifests;

let crds = generate_crd_manifests()?;
for (i, crd) in crds.iter().enumerate() {
    std::fs::write(format!("crd-{}.yaml", i), crd)?;
}
```

Or generate all CRDs at once:

```bash
# Output all CRDs as YAML (pipe to kubectl apply)
agentkernel operator crds | kubectl apply -f -
```

## Deploying agentkernel on Kubernetes

Run agentkernel itself as a Kubernetes service that manages sandbox pods via the HTTP API.

### Install with Helm

```bash
# Install from OCI registry (recommended)
helm install agentkernel oci://ghcr.io/thrashr888/charts/agentkernel \
  --version 0.6.0 \
  --namespace agentkernel-system \
  --create-namespace
```

> **Note:** The OCI chart is published automatically on each release. If not yet available, use the local clone method below.

Or install from a local clone:

```bash
git clone https://github.com/thrashr888/agentkernel.git
helm install agentkernel agentkernel/deploy/helm/agentkernel/ \
  --namespace agentkernel-system \
  --create-namespace
```

### Helm Values

Override defaults with `--set` flags or a custom `values.yaml`:

```yaml
backend: kubernetes

orchestrator:
  namespace: agentkernel-sandboxes    # Where sandbox pods run
  runtimeClass: ""                     # "gvisor" if available
  warmPoolSize: 10                     # Pre-warmed pods
  maxSandboxes: 200                    # Cluster-wide limit
  serviceAccount: agentkernel-sandbox  # SA for sandbox pods

sandbox:
  defaults:
    image: alpine:3.24
    memory: 512Mi
    cpu: "1"
    securityProfile: restrictive

apiKey: ""          # Set via --set apiKey=<key> or external secret

resources:
  limits:
    memory: 256Mi
    cpu: 500m
  requests:
    memory: 128Mi
    cpu: 100m

autoscaling:
  enabled: false
  minReplicas: 1
  maxReplicas: 5
```

### What the Chart Creates

| Resource | Purpose |
|----------|---------|
| Deployment | agentkernel API server |
| Service | ClusterIP on port 18888 |
| ServiceAccount | For the API server pod |
| ClusterRole | RBAC for managing sandbox pods |
| ClusterRoleBinding | Binds role to service account |
| ConfigMap | agentkernel.toml configuration |
| Namespace | Sandbox namespace (configurable) |
| Secret | API key (if set) |
| HPA | Horizontal Pod Autoscaler (optional) |

### RBAC

The Helm chart creates a ClusterRole with permissions to:

- Create, delete, list, get pods in the sandbox namespace
- Create and delete NetworkPolicies (for `network: false` sandboxes)
- Exec into pods (for `agentkernel exec`)
- Watch and update AgentSandbox, AgentSandboxPool CRDs
- (Enterprise) Watch and update AgentKernelPolicy, ClusterAgentKernelPolicy CRDs

### Upgrade and Uninstall

```bash
helm upgrade agentkernel oci://ghcr.io/thrashr888/charts/agentkernel \
  --version 0.6.0 \
  --namespace agentkernel-system

helm uninstall agentkernel --namespace agentkernel-system
```
