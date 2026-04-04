# Remote Backends

Remote backends keep the same CLI and HTTP model as local sandboxes, but the sandbox runs on a hosted provider instead of Docker, Firecracker, or Apple Containers.

The bundled bridge currently ships live adapters for:

- `daytona`
- `runloop`
- `e2b`
- `modal`

`agentcomputer` is parsed as a backend value and keeps the same contract shape, but the bundled live adapter is not landed yet.

## Prerequisites

```bash
npm install --prefix scripts
node --version   # Node.js 20+
```

Provider credentials can come from exported environment variables or from `api_key_env` / `token_*_env` names in `agentkernel.toml`.

## Common Behavior

- `mount_cwd = true` syncs the local project into `/workspace`
- `agentkernel exec` pushes local changes before the command and pulls remote changes back after it completes
- declared `ports` resolve to provider `endpoints`
- `snapshot take` and `snapshot restore` use the shared workspace-level snapshot contract

## Daytona

Example config:

```toml
[sandbox]
name = "daytona-demo"
base_image = "node:22"

[security]
mount_cwd = true
network = true

[remote]
bridge = "./scripts/remote-bridge.mjs"

[remote.daytona]
api_key_env = "DAYTONA_API_KEY"
base_url = "https://app.daytona.io/api"
organization = "your-org-id"
region = "us"
```

Run it:

```bash
agentkernel sandbox create my-daytona --backend daytona -c agentkernel.toml
agentkernel exec my-daytona -- sh -lc 'node -v'
agentkernel snapshot take my-daytona --name my-daytona-snap
```

## E2B

Example config:

```toml
[sandbox]
name = "e2b-demo"
base_image = "base"

[security]
mount_cwd = true
network = true

[remote]
bridge = "./scripts/remote-bridge.mjs"

[remote.e2b]
api_key_env = "E2B_API_KEY"
```

Run it:

```bash
agentkernel sandbox create my-e2b --backend e2b -c agentkernel.toml
agentkernel exec my-e2b -- sh -lc 'python3 --version || node -v'
agentkernel snapshot take my-e2b --name my-e2b-snap
```

## Modal

Example config:

```toml
[sandbox]
name = "modal-demo"
base_image = "base"

[security]
mount_cwd = true
network = true

[network]
ports = ["3000"]

[remote]
bridge = "./scripts/remote-bridge.mjs"

[remote.modal]
token_id_env = "MODAL_TOKEN_ID"
token_secret_env = "MODAL_TOKEN_SECRET"
project = "agentkernel"
```

Run it:

```bash
agentkernel sandbox create my-modal --backend modal -c agentkernel.toml
agentkernel exec my-modal -- sh -lc 'pwd && ls -la /workspace'
agentkernel snapshot take my-modal --name my-modal-snap
```

## Runloop

Example config:

```toml
[sandbox]
name = "runloop-demo"
base_image = "default"

[security]
mount_cwd = true
network = true

[network]
ports = ["3000"]

[remote]
bridge = "./scripts/remote-bridge.mjs"

[remote.runloop]
api_key_env = "RUNLOOP_API_KEY"
```

Run it:

```bash
agentkernel sandbox create my-runloop --backend runloop -c agentkernel.toml
agentkernel exec my-runloop -- sh -lc 'pwd && ls -la /workspace'
agentkernel snapshot take my-runloop --name my-runloop-snap
```

## Examples

- [examples/remote-daytona/README.md](../../examples/remote-daytona/README.md)
- [examples/remote-runloop/README.md](../../examples/remote-runloop/README.md)
- [examples/remote-e2b/README.md](../../examples/remote-e2b/README.md)
- [examples/remote-modal/README.md](../../examples/remote-modal/README.md)
