# AWS Nitro Sandboxes on EKS

AgentKernel can run a sandbox in an AWS Nitro Sandbox using the **Kubernetes
backend** and EKS's `isolated-sandbox` RuntimeClass. This is an opt-in integration,
not a separate AWS provider. AgentKernel creates/deletes Pods and executes commands
through the Kubernetes API. It does not create an EKS cluster, node group, IAM role,
StorageClass, or persistent volume claim.

This integration covers the direct sandbox lifecycle, including CLI `sandbox`
commands and `run`. The Kubernetes warm-pool manager and Sandbox CRD operator do
not use these storage, target-binding, and startup-timeout settings. Do not use
those paths for this recipe. Hardware isolation has not been validated on a live
EKS cluster by the local/mocked tests.

## Prerequisites

As documented by AWS on November 11, 2026:

- EKS Kubernetes **1.36 or later** with the `isolated-sandbox` RuntimeClass.
- **EKS Auto Mode:** compatible sandbox nodes are provisioned for this runtime
  class. The standard setup requires no additional node selector. A restrictive
  NodePool can still prevent provisioning.
- **Managed/self-managed nodes:** a custom sandbox-enabled AMI, a supported
  instance type, and `EnclaveOptions.Enabled=true` in the node launch template.
  Merely choosing an instance type that supports Nitro Enclaves is insufficient.
- `kubectl` for CLI running-sandbox discovery, plus Kubernetes credentials with Pod create/get/list/delete and Pod exec permissions
  in the intended namespace. Service and NetworkPolicy permissions are also
  required for cleanup and for their respective features.
- A sandbox image with `sh` and `sleep infinity`, usable by non-root UID 1000.
- For durable files, an existing namespace-local claim backed by compatible
  storage and a working CSI driver. For EBS, use **ReadWriteOncePod**, not shared
  attachment across sandboxes. Prepare volume ownership for UID/GID 1000 or use
  `fs_group = 1000` with a CSI driver that supports it.

Read the current [AWS isolated-mode setup](https://docs.aws.amazon.com/eks/latest/userguide/sandboxing-isolated-setup.html)
and [limitations](https://docs.aws.amazon.com/eks/latest/userguide/sandboxing-limitations.html)
before provisioning. Supported instance types, regional availability, and
provisioning constraints can change.

## Configure one sandbox

Copy [the example configuration](https://github.com/thrashr888/agentkernel/blob/main/examples/aws-nitro/agentkernel.toml)
and replace `your-eks-context` with the intended EKS context. Nitro mode requires
both `kubeconfig` and `context` explicitly; it never silently falls back to the
current cluster. A nonexistent context fails before creating a Pod.

```toml
[sandbox]
name = "nitro-demo"
base_image = "alpine:3.24"

[resources]
vcpus = 1
memory_mb = 512

[security]
profile = "moderate"

[orchestrator]
provider = "kubernetes"
namespace = "agentkernel"
kubeconfig = "~/.kube/config"
context = "your-eks-context"
runtime_class = "isolated-sandbox"
startup_timeout_secs = 600
warm_pool_size = 0
fs_group = 1000

[[orchestrator.persistent_volume_claims]]
claim_name = "nitro-demo-workspace"
mount_path = "/workspace"
read_only = false
```

Remove `persistent_volume_claims` and `fs_group` for an ephemeral sandbox.
Claims are **explicitly shared by every sandbox using the same configuration**.
Use a dedicated claim and config per sandbox. AgentKernel does not create,
clone, resize, or delete claims. `sandbox remove` also retains the claim and its
storage charges. Kubernetes PVCs are separate from AgentKernel's Docker/Podman
`--volume` feature.

`startup_timeout_secs` defaults to 120 and accepts 1–86400 seconds. A larger value
allows time for node provisioning, image pulls, and volume attachment. It does not
make incompatible nodes compatible. Timeout errors include the last Pod status
so scheduling and image problems remain visible.

The manager stores the selected Kubernetes settings with the sandbox. Editing or
removing the original TOML does not redirect later stop/remove operations. To
change the runtime, namespace, storage, or cluster, remove the old sandbox and
create a new one. The kubeconfig file is still read for credentials and context
resolution: do not remap the saved context to another cluster. This is a pinned
path/context selection, not an immutable cluster identity or a credential copy.
Legacy sandbox records created before this integration retain the old default
namespace and ambient kubeconfig selection. The previous manager ignored their
project's orchestrator settings, so reading those settings during an upgrade
could redirect cleanup to the wrong cluster. Remove the old sandbox using its
original ambient context, then create a new sandbox to enable Nitro or custom
target settings.
New manager-created sandboxes also carry a UUID ownership label, so cleanup
cannot adopt a same-named Pod belonging to a different sandbox record.

## Start, stop, and recreate

These commands act on an existing cluster and can incur compute/storage charges.
Run them only after the prerequisites and cost limits are approved:

```bash
agentkernel sandbox create nitro-demo --backend kubernetes \
  --config ./agentkernel.toml
agentkernel exec nitro-demo -- sh -c 'echo retained > /workspace/probe'
agentkernel sandbox stop nitro-demo
agentkernel sandbox start nitro-demo
agentkernel exec nitro-demo -- cat /workspace/probe
agentkernel sandbox stop nitro-demo
```

Check the actual runtime and claim attachment with your explicit target:

```bash
kubectl --kubeconfig ~/.kube/config --context your-eks-context \
  -n agentkernel get pod agentkernel-nitro-demo \
  -o jsonpath='{.spec.runtimeClassName}'
```

The Pod is deleted on stop. AgentKernel waits for disappearance (up to 120
seconds) rather than equating a delete acknowledgement with termination. API or
delete failures are reported and can be retried. Service and NetworkPolicy
deletion is also checked: if an object remains, stop reports pending cleanup and
retains ownership for retry after its finalizers clear. It retains ownership when
startup rollback fails. A fresh start creates a new Pod using the same claim;
file data persists, but processes, connections, and RAM do not. This is **not**
AgentKernel full-state pause/resume or fork, and those capabilities remain
unsupported for Kubernetes. AWS's [pause/resume example](https://aws-samples.github.io/sample-eks-nitro-sandbox/agent-sandbox/pause-resume/index.html)
uses the same delete-and-reattach pattern.

## Scale-to-zero and security boundaries

- `warm_pool_size = 0` documents the intended cold-start configuration. The direct
  backend does not initialize a warm pool; the separate warm-pool API does not
  provide a cold fallback when empty.
- Deleting the last sandbox Pod **does not itself terminate an EC2 node**.
  Auto Mode/Karpenter or another configured autoscaler must consolidate eligible
  nodes. Other Pods, disruption budgets, and NodePool minimums can prevent it.
- EKS control-plane charges, EBS/EFS storage, NAT gateways, and other infrastructure
  can remain billable at zero sandbox Pods. This is not a zero-cost idle promise.
- `isolated-sandbox` trusts the parent instance. This recipe does not provide
  confidential mode, attestation, encrypted-image policy, or Nitro Cedar policy
  management. AgentKernel's own enterprise Cedar policy is a different layer.
- Service-account token automount stays disabled. Do not assume EC2 IMDS provides
  credentials; arrange any AWS workload identity separately and verify it.
- Host networking, GPU/device passthrough, full-state snapshots, and in-place
  resource resizing are not provided by this integration. Bind internal services
  to the Pod IP or `0.0.0.0`, not only loopback, if using port-forwarding.
- Network isolation still depends on a NetworkPolicy-capable cluster. Test your
  actual EKS networking and policy enforcement before running untrusted workloads.

## Validation

Local unit tests cover the rendered RuntimeClass/PVC/resource configuration,
explicit target validation, persisted configuration, timeout diagnostics,
retained storage, and retryable/ownership-checked cleanup with a mock Kubernetes
API. They do not establish EKS compatibility or exercise real CSI attachment.

Before production use, perform an authorized staging smoke test with a dedicated
claim: create, exec, write a file, stop, verify the Pod is gone and the claim remains,
restart, verify the file, and stop again. Observe node consolidation separately.
Also test a Pending Pod, a denied delete, a finalizer-delayed delete, and image-pull
failure. Do not use a production namespace or a claim holding valuable data for
the first test.
