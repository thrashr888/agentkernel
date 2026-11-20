# NVIDIA OpenShell backend (experimental)

AgentKernel can use the NVIDIA OpenShell 0.1.9 CLI against an **existing gateway and dedicated workspace**. OpenShell, not AgentKernel, supplies the workload isolation and the gateway's effective filesystem and network policies. This preview is an explicit choice, never an automatically selected backend.

## Try a named sandbox

Use a gateway already configured and authenticated in the local OpenShell CLI. The workspace is for a single trusted operator. Run lifecycle commands serially and don't share it with independent writers while testing. The CLI targets names rather than immutable IDs for mutations, so don't repoint the gateway alias, replace objects or edit their ownership labels while AgentKernel is managing them. AgentKernel doesn't create gateways or workspaces, change the active gateway, alter policies, or attach credential providers.

```bash
export AGENTKERNEL_OPENSHELL_GATEWAY=my-gateway
export AGENTKERNEL_OPENSHELL_WORKSPACE=agentkernel-test

agentkernel sandbox create openshell-smoke --backend openshell --image alpine:3.24
agentkernel exec openshell-smoke -- /bin/sh -c 'printf "openshell-ok\n"'
agentkernel sandbox stop openshell-smoke
agentkernel sandbox start openshell-smoke
agentkernel sandbox remove openshell-smoke
```

Run these commands from a directory without unrelated AgentKernel project configuration.

Use a disposable Linux OCI image that includes `/bin/sh` and `sleep`. AgentKernel saves a randomly generated ownership marker before provisioning and checks the returned gateway workspace, object identity, and ownership marker before use. Stop preserves the sandbox for a later start. Removal waits for the owned object to disappear and reports incomplete cleanup rather than declaring success. If a gateway becomes unavailable, restore access to that same target before retrying cleanup.

This first adapter covers named create, captured non-interactive exec with an optional working directory, stop/start, and remove. It does not provide an AgentKernel permission-policy translation, mounts, file injection/sync, secrets, model governance, ports, console attachment, snapshots, or full-state resume. Offline and read-only-root requirements fail instead of being silently dropped. CPU and memory quantities are passed to OpenShell; enforcement depends on its [compute driver](https://docs.nvidia.com/openshell/v0.1.9/how-it-works/sandboxes/overview#cpu-and-memory). This is not a supported configuration for a shared or untrusted AgentKernel service.

The adapter is tested against a fake CLI. A live OpenShell gateway smoke and verification of its effective policy are still required before treating the preview as validated for deployment. See [NVIDIA's sandbox CLI documentation](https://docs.nvidia.com/openshell/v0.1.9/how-it-works/sandboxes/overview).
