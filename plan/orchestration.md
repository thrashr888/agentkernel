# Agent Sandbox Orchestration: Kubernetes & Nomad

## Problem

agentkernel runs sandboxes on a single machine (Docker, Firecracker, Apple containers). To run hundreds of concurrent agent sandboxes, we need orchestrators (Kubernetes, Nomad) to schedule sandbox workloads across nodes, manage resources, handle health/scaling, and provide a production-grade control plane.

## Architecture

```
                       Clients (Claude Code, SDKs, HTTP)
                                    |
                                    v
                    +------------------------------+
                    |   agentkernel API server      |
                    |   (HTTP API on :18888)         |
                    |   Deployed as K8s Deployment   |
                    |   or Nomad service job         |
                    +-------------+----------------+
                                  |
              +-------------------+-------------------+
              v                                       v
    +--------------------+              +--------------------+
    |   Kubernetes        |              |   Nomad             |
    |                     |              |                     |
    |  agentkernel asks   |              |  agentkernel asks   |
    |  K8s API to         |              |  Nomad API to       |
    |  create/exec/       |              |  create/exec/       |
    |  stop Pods          |              |  stop Allocations   |
    |                     |              |                     |
    |  [Pod][Pod][Pod]    |              |  [Alc][Alc][Alc]   |
    |  [Pod][Pod][Pod]    |              |  [Alc][Alc][Alc]   |
    |   ...hundreds...    |              |   ...hundreds...    |
    +--------------------+              +--------------------+
```

**How it works**: `agentkernel run --backend kubernetes` tells agentkernel to create a K8s Pod (or Nomad allocation) as the sandbox. The orchestrator handles scheduling it onto a node, enforcing resource limits, and restarting on failure. agentkernel's HTTP API becomes the control plane that clients talk to; K8s/Nomad is the data plane that actually runs the sandboxes.

---

## Architectural Decisions

| Decision | Choice | Rationale |
|---|---|---|
| How agentkernel creates sandboxes | New `BackendType::Kubernetes` and `BackendType::Nomad` implementing `Sandbox` trait | Same abstraction as Docker/Firecracker -- `start()` creates a Pod, `exec()` runs commands in it, `stop()` deletes it |
| How agentkernel itself is deployed | Helm chart (K8s) / Nomad service job | The API server runs as a managed workload in the same cluster it creates sandboxes in |
| K8s sandbox lifecycle | Raw Pods + optional CRD operator | Pods work immediately; CRD adds K8s-native declarative management later |
| Nomad sandbox lifecycle | Batch jobs per sandbox | Each sandbox = one Nomad job with a single allocation |
| K8s client crate | `kube` + `k8s-openapi` | De facto Rust K8s client; handles auth, exec, WebSocket natively |
| Nomad client | `reqwest` for API, shell to `nomad` CLI for exec (Phase 1) | Simple REST API; native WebSocket exec in Phase 2 |
| State management | Local JSON as pointer to remote resource (pod name, alloc ID) | Matches Docker pattern where `is_running()` checks the runtime directly |
| Warm pools at scale | K8s: labeled idle pods; Nomad: parameterized dispatch jobs | Pre-warm 10-50 sandboxes cluster-wide for sub-second acquisition |
| Feature flags | `--features kubernetes` / `--features nomad` | Opt-in, keeps default binary small |

---

## Phase 1: K8s/Nomad Backends (Sandbox trait)

This is the core: when `--backend kubernetes` is used, `start()` creates a Pod, `exec()` runs commands in it via the K8s API.

### Cargo.toml

```toml
[features]
kubernetes = ["dep:kube", "dep:k8s-openapi"]
nomad = ["dep:reqwest"]

[dependencies]
kube = { version = "0.98", features = ["client", "runtime", "ws"], optional = true }
k8s-openapi = { version = "0.23", features = ["latest"], optional = true }
reqwest = { version = "0.12", features = ["json", "rustls-tls"], optional = true }
```

### src/backend/mod.rs

Add `Kubernetes` and `Nomad` to `BackendType` enum. These are never auto-detected -- they require explicit `--backend` or `[orchestrator]` config. Update `Display`, `FromStr`, `backend_available()`, `create_sandbox()`.

### src/backend/kubernetes.rs (~550 lines)

```rust
pub struct KubernetesSandbox {
    name: String,
    namespace: String,          // from OrchestratorConfig
    pod_name: Option<String>,   // set after start()
    running: bool,
    client: Option<kube::Client>,
    runtime_class: Option<String>,  // gvisor, kata
    service_account: Option<String>,
    node_selector: HashMap<String, String>,
}
```

Sandbox trait mapping:

| Method | K8s Implementation |
|---|---|
| `start()` | Create Pod with `sleep infinity`, wait for Running phase |
| `exec()` | `Api::<Pod>::exec()` via WebSocket, `tokio::join!` stdout+stderr |
| `exec_with_env()` | Wrap command: `env KEY=VAL ... <cmd>` |
| `stop()` | Delete Pod, wait for termination |
| `write_file()` | Exec `sh -c 'base64 -d > /path'` with piped stdin |
| `read_file()` | Exec `base64 /path`, decode output |
| `remove_file()` | Exec `rm -f /path` |
| `attach()` | `kube::api::attach` with TTY |
| `is_running()` | Query Pod phase via API |

Pod spec:
- Labels: `agentkernel.io/sandbox={name}`, `agentkernel.io/managed-by=agentkernel`
- `automountServiceAccountToken: false` (sandbox pods don't need K8s API access)
- `runtimeClassName` from config (gvisor for stronger isolation)
- Security context from `Permissions` (see Phase 4)
- Resource limits from `SandboxConfig`
- When `network: false`: create a `NetworkPolicy` denying all traffic for this pod

### src/backend/nomad.rs (~450 lines)

```rust
pub struct NomadSandbox {
    name: String,
    job_id: Option<String>,
    alloc_id: Option<String>,
    running: bool,
    client: NomadClient,  // addr, token, region, reqwest::Client
    driver: String,       // "docker", "exec", "raw_exec"
    datacenter: Option<String>,
}
```

| Method | Nomad Implementation |
|---|---|
| `start()` | `PUT /v1/jobs` with job spec, poll until alloc is `running` |
| `exec()` | `nomad alloc exec {alloc_id} -- <cmd>` (Phase 1) |
| `stop()` | `DELETE /v1/job/{id}?purge=true` |
| `read_file()` | `GET /v1/client/fs/cat/{alloc_id}?path=...` |
| `write_file()` | Exec-based (same as K8s) |
| `is_running()` | `GET /v1/allocation/{alloc_id}` check status |

Job spec:
- Type: `batch` with `sleep infinity` entrypoint
- Driver: configurable (docker default, exec, raw_exec)
- `network { mode = "none" }` when `!config.network`
- Resources: memory MB, CPU MHz from SandboxConfig
- Meta: `agentkernel-sandbox={name}`, `agentkernel-managed=true`

### src/config.rs -- OrchestratorConfig

```toml
[orchestrator]
provider = "kubernetes"       # or "nomad"
namespace = "agentkernel"     # K8s namespace / Nomad namespace
warm_pool_size = 10           # pre-warmed sandboxes

# Kubernetes-specific
kubeconfig = "~/.kube/config" # optional, auto-detected
context = "k3s-default"       # kubeconfig context
runtime_class = "gvisor"      # optional: gvisor, kata
service_account = "agentkernel-sandbox"

# Nomad-specific
nomad_addr = "http://127.0.0.1:4646"
nomad_token = ""              # or use NOMAD_TOKEN env
nomad_driver = "docker"       # docker, exec, raw_exec
nomad_datacenter = "dc1"
```

### src/vmm.rs -- Remote state

Extend `SandboxState` with `remote_id: Option<String>` (pod name / alloc ID) and `remote_namespace: Option<String>`. Update `detect_running_sandboxes()` to query K8s/Nomad for pods/allocs with `agentkernel.io/managed-by` labels.

---

## Phase 2: Deploy agentkernel into K8s (Helm Chart)

The agentkernel API server itself runs as a K8s Deployment, so it can create sandbox Pods in the same cluster.

### deploy/helm/agentkernel/

```
Chart.yaml
values.yaml
templates/
  deployment.yaml      # agentkernel API server
  service.yaml         # ClusterIP or LoadBalancer
  serviceaccount.yaml  # SA for the API server (needs Pod create/exec/delete)
  clusterrole.yaml     # RBAC: pods, networkpolicies in sandbox namespace
  clusterrolebinding.yaml
  namespace.yaml       # agentkernel namespace for sandbox pods
  configmap.yaml       # agentkernel.toml
  secret.yaml          # API key
  hpa.yaml             # optional: autoscale API server
```

**values.yaml** (key settings):

```yaml
replicaCount: 1

image:
  repository: ghcr.io/thrashr888/agentkernel
  tag: latest

backend: kubernetes

orchestrator:
  namespace: agentkernel-sandboxes
  runtimeClass: ""        # "gvisor" if available
  warmPoolSize: 10
  maxSandboxes: 200       # cluster-wide limit

resources:
  limits:
    memory: 256Mi
    cpu: 500m

sandbox:
  defaults:
    image: alpine:3.20
    memory: 512Mi
    cpu: "1"
    securityProfile: restrictive

rbac:
  create: true

apiKey: ""  # set via --set or external secret

service:
  type: ClusterIP
  port: 18888
```

**RBAC** -- the API server's ServiceAccount needs:

```yaml
rules:
  - apiGroups: [""]
    resources: [pods, pods/exec, pods/attach, pods/log]
    verbs: [create, get, list, watch, delete]
  - apiGroups: ["networking.k8s.io"]
    resources: [networkpolicies]
    verbs: [create, get, delete]
```

### Install and test on k3s:

```bash
# Build image
docker build -t agentkernel:dev .
# Import into k3s
k3s ctr images import <(docker save agentkernel:dev)

# Install via Helm
helm install agentkernel deploy/helm/agentkernel/ \
  --set image.repository=agentkernel --set image.tag=dev \
  --set image.pullPolicy=Never \
  --set orchestrator.warmPoolSize=5

# Port-forward and test
kubectl port-forward svc/agentkernel 18888:18888

# Watch sandbox pods get created
kubectl -n agentkernel-sandboxes get pods -w
```

---

## Phase 3: Deploy agentkernel into Nomad

### deploy/nomad/agentkernel.nomad.hcl

```hcl
job "agentkernel" {
  datacenters = ["dc1"]
  type        = "service"

  group "api" {
    count = 1

    network {
      port "http" { static = 18888 }
    }

    service {
      name = "agentkernel"
      port = "http"
      check {
        type     = "http"
        path     = "/health"
        interval = "10s"
        timeout  = "2s"
      }
    }

    task "server" {
      driver = "docker"

      config {
        image = "ghcr.io/thrashr888/agentkernel:latest"
        args  = ["serve", "--host", "0.0.0.0", "--port", "18888",
                 "--backend", "nomad"]
        ports = ["http"]
      }

      env {
        NOMAD_ADDR = "http://${attr.unique.network.ip-address}:4646"
      }

      resources {
        cpu    = 500
        memory = 256
      }
    }
  }
}
```

### Test on local Nomad:

```bash
nomad job run deploy/nomad/agentkernel.nomad.hcl
nomad job status agentkernel
curl -X POST http://localhost:18888/run \
  -H 'Content-Type: application/json' \
  -d '{"command": ["echo", "hello from nomad"]}'
```

---

## Phase 4: Security Mapping

Map agentkernel's `Permissions` struct to orchestrator-native security.

### src/permissions.rs additions

```rust
impl Permissions {
    #[cfg(feature = "kubernetes")]
    pub fn to_k8s_security_context(&self) -> SecurityContext { ... }

    #[cfg(feature = "kubernetes")]
    pub fn to_k8s_resources(&self) -> ResourceRequirements { ... }

    #[cfg(feature = "nomad")]
    pub fn to_nomad_resources(&self) -> serde_json::Value { ... }
}
```

| Permissions | Kubernetes | Nomad |
|---|---|---|
| `network: false` | NetworkPolicy deny-all | `network { mode = "none" }` |
| `read_only_root` | `readOnlyRootFilesystem: true` | `readonly_rootfs = true` |
| `max_memory_mb` | `resources.limits.memory` | `resources { memory = N }` |
| `max_cpu_percent` | `resources.limits.cpu` (millicores) | `resources { cpu = N }` MHz |
| `seccomp` | `seccompProfile` | `security_opt` (Docker driver) |
| `allow_privileged: false` | `privileged: false`, drop all caps | `privileged = false`, `cap_drop = ["ALL"]` |

K8s-specific extras:
- `runtimeClassName: "gvisor"` for kernel-level sandbox isolation
- Pod Security Standards: `pod-security.kubernetes.io/enforce: restricted`
- `automountServiceAccountToken: false`

---

## Phase 5: Warm Pools at Scale

### Kubernetes warm pool

Pre-create N idle pods labeled `agentkernel.io/pool=warm`. On sandbox `start()`:
1. Check if a warm pod with matching image exists (label selector)
2. If yes: relabel to `agentkernel.io/pool=active`, return immediately
3. If no: create a new pod (cold start)
4. Background: replenish warm pool back to target size

### Nomad warm pool

Use Nomad parameterized batch jobs:
1. Pre-dispatch N jobs that start containers and wait
2. On sandbox `start()`: claim an idle allocation
3. On sandbox `stop()`: stop the allocation, dispatch a replacement

### Pool configuration

```toml
[orchestrator]
warm_pool_size = 20       # pre-warmed sandboxes
max_sandboxes = 500       # hard cap on total concurrent sandboxes
warm_pool_images = [      # which images to pre-warm
  "alpine:3.20",
  "python:3.12-alpine",
  "node:22-alpine",
]
```

---

## Phase 6: CRD + Operator (Optional, K8s-native)

For teams that want K8s-native sandbox management (create sandboxes via `kubectl apply`):

### CRD: AgentSandbox

```yaml
apiVersion: agentkernel.io/v1alpha1
kind: AgentSandbox
metadata:
  name: my-agent-sandbox
  namespace: agentkernel-sandboxes
spec:
  image: python:3.12-alpine
  resources:
    memory: 512Mi
    cpu: "1"
  securityProfile: restrictive
  network: false
  ttl: 1h
status:
  phase: Running
  podName: agentkernel-my-agent-sandbox-abc123
  startedAt: "2026-01-31T12:00:00Z"
```

### CRD: AgentSandboxPool (warm pool)

```yaml
apiVersion: agentkernel.io/v1alpha1
kind: AgentSandboxPool
metadata:
  name: default-pool
spec:
  size: 20
  image: alpine:3.20
  resources:
    memory: 256Mi
```

The operator watches AgentSandbox CRs, creates pods, reports status. This is a later addition -- raw Pod API works first.

---

## Files to Create/Modify

| File | Action | Phase |
|---|---|---|
| `Cargo.toml` | Modify -- features + deps | 1 |
| `src/backend/mod.rs` | Modify -- `BackendType` enum, factory | 1 |
| `src/backend/kubernetes.rs` | **Create** -- K8s Sandbox impl | 1 |
| `src/backend/nomad.rs` | **Create** -- Nomad Sandbox impl | 1 |
| `src/config.rs` | Modify -- `OrchestratorConfig` | 1 |
| `src/vmm.rs` | Modify -- remote state fields | 1 |
| `src/main.rs` | Modify -- CLI backend options | 1 |
| `Dockerfile` | **Create** -- multi-stage build | 2 |
| `deploy/helm/agentkernel/Chart.yaml` | **Create** | 2 |
| `deploy/helm/agentkernel/values.yaml` | **Create** | 2 |
| `deploy/helm/agentkernel/templates/*.yaml` | **Create** (7 templates) | 2 |
| `deploy/nomad/agentkernel.nomad.hcl` | **Create** | 3 |
| `src/permissions.rs` | Modify -- security mapping | 4 |
| `src/backend/kubernetes_pool.rs` | **Create** -- cluster warm pool | 5 |
| `src/backend/nomad_pool.rs` | **Create** -- Nomad warm pool | 5 |
| `deploy/helm/agentkernel/crds/*.yaml` | **Create** -- CRDs | 6 |
| `tests/kubernetes_integration_test.rs` | **Create** | 1-2 |
| `tests/nomad_integration_test.rs` | **Create** | 1-3 |

---

## Testing on Local k3s + Nomad

### k3s testing

```bash
kubectl get nodes
docker build -t agentkernel:dev .
k3s ctr images import <(docker save agentkernel:dev)

helm install agentkernel deploy/helm/agentkernel/ \
  --set image.repository=agentkernel --set image.tag=dev \
  --set image.pullPolicy=Never \
  --set orchestrator.warmPoolSize=5

kubectl port-forward svc/agentkernel 18888:18888

# Create 100 sandboxes
for i in $(seq 1 100); do
  curl -s -X POST http://localhost:18888/sandboxes \
    -H 'Content-Type: application/json' \
    -d "{\"name\": \"test-$i\", \"image\": \"alpine:3.20\"}" &
done; wait

kubectl -n agentkernel-sandboxes get pods | wc -l
```

### Nomad testing

```bash
nomad server members
nomad job run deploy/nomad/agentkernel.nomad.hcl
curl http://localhost:18888/health
```

---

## Known Challenges

1. **K8s exec WebSocket**: `kube` returns `AttachedProcess` with async readers. Must `tokio::join!` both to avoid deadlocks.
2. **Nomad exec Phase 1**: Shell out to `nomad alloc exec`. Phase 2 adds native WebSocket.
3. **Pod scheduling latency**: New pod takes 1-5s. Warm pools eliminate this.
4. **k3s resource limits**: Laptop can't run 500 pods. Set `max_sandboxes` to 50-100 for local testing.
5. **Namespace isolation**: Sandbox pods run in dedicated namespace with restricted RBAC.
6. **Image pull**: First run of new image is slow. Warm pools + `IfNotPresent` mitigate.
7. **Auth tokens**: Never store in SandboxState JSON. K8s uses kubeconfig/in-cluster SA. Nomad uses NOMAD_TOKEN env.
8. **File transfer**: Base64 adds ~33% overhead. Acceptable for agent workloads. Tar streaming later.
