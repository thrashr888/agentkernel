
# Orchestration Backends

agentkernel supports Kubernetes and HashiCorp Nomad as orchestration backends for running sandboxes on remote clusters. These complement the local backends (Docker, Podman, Firecracker, Apple, Hyperlight) for team, cloud, and multi-tenant environments.

Both backends implement the same `Sandbox` trait as local backends. All CLI commands (`create`, `start`, `exec`, `stop`, `list`) work identically — only the `--backend` flag changes.

## Comparison

| Feature | Kubernetes | Nomad |
|---------|-----------|-------|
| Isolation | Pod | Job allocation |
| Network control | NetworkPolicy | Network stanza (`mode: "none"`) |
| Warm pool | Label-based (warm → active relabel) | Parameterized batch jobs |
| Operator CRD | AgentSandbox, AgentSandboxPool | N/A |
| Auth | kubeconfig / in-cluster SA | ACL token / `NOMAD_TOKEN` |
| Requirements | `kubectl`, cluster access | `nomad` CLI, cluster access |

Both backends are included in the default build. You must specify `--backend kubernetes` or `--backend nomad` explicitly — they are never auto-detected.

## Next Steps

- [Kubernetes Backend](orchestration-kubernetes.md) — Pods, NetworkPolicy, warm pools, CRDs, operator, Helm deployment
- [Nomad Backend](orchestration-nomad.md) — Jobs, parameterized warm pools, task drivers, Nomad Pack deployment
- [Deployment Guide](deploy.md) — Docker image, building from source, HTTP API reference
