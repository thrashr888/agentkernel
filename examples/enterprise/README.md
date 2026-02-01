# Enterprise Policy Examples

Example Cedar policies for agentkernel's enterprise policy engine.

## Prerequisites

Build with the enterprise feature:

```bash
cargo build --features enterprise
```

## Configuration

See `agentkernel.toml` for the enterprise config section. Key settings:

- `enabled` — activate policy enforcement
- `policy_server` — URL of your policy server (policies are pulled via HTTPS)
- `org_id` — your organization identifier
- `offline_mode` — behavior when the policy server is unreachable
- `trust_anchors.keys` — Ed25519 public key IDs for signature verification

## Example Policies

Each `.cedar` file in `policies/` demonstrates a different authorization pattern:

| Policy | Description |
|--------|-------------|
| `default.cedar` | Permits all authenticated users to create and run sandboxes |
| `rbac.cedar` | Role-based access: developer, admin, viewer |
| `mfa-required.cedar` | Requires MFA for network access and volume mounts |
| `runtime-restrictions.cedar` | Limits which runtimes and agent types are available |
| `org-isolation.cedar` | Restricts access to a single organization |

## Cedar Schema

The built-in schema defines:

**Entities:**
- `AgentKernel::User` — email, org_id, roles (Set), mfa_verified
- `AgentKernel::Sandbox` — name, agent_type, runtime

**Actions:**
- `Create` — create a new sandbox
- `Run` — run a command in a new sandbox
- `Exec` — execute a command in a running sandbox
- `Attach` — attach to a sandbox session
- `Mount` — mount a host volume into a sandbox
- `Network` — enable network access for a sandbox

## How Policies Are Evaluated

1. Cedar uses **default deny** — if no `permit` matches, the action is denied
2. `forbid` rules always win over `permit` rules
3. Conditions in `when` clauses filter which requests match
4. Multiple policy files can be combined — all are evaluated together

## Kubernetes-Native Policy Management

When running on Kubernetes, Cedar policies can be managed as CRDs instead of (or alongside) the HTTP policy server.

### Namespaced Policies

Apply a policy to a specific namespace:

```yaml
apiVersion: agentkernel/v1alpha1
kind: AgentKernelPolicy
metadata:
  name: deny-network
  namespace: staging
spec:
  cedar: |
    forbid(
        principal,
        action == AgentKernel::Action::"Network",
        resource
    );
  priority: 100
```

### Cluster-Wide Policies

Apply a policy globally:

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
```

### GitOps Compatible

Policy CRDs work with Argo CD, Flux, and other GitOps tools. Store policies in your infrastructure repo and `kubectl apply` them automatically.

```bash
kubectl get akp -A           # Namespace policies
kubectl get cakp             # Cluster policies
```

See `docs/orchestration-kubernetes.md` for full CRD documentation.

## Testing Policies Locally

Run the enterprise tests to validate policies against the Cedar engine:

```bash
cargo test --features enterprise
```
