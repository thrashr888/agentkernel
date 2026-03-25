
# agentkernel.toml

Complete reference for the agentkernel configuration file.

## [sandbox]

Basic sandbox settings.

```toml
[sandbox]
name = "my-project"           # Sandbox name
base_image = "python:3.12"    # Base Docker image (if not using build)
```

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Sandbox name (alphanumeric, hyphens, underscores) |
| `base_image` | string | Docker image to use (ignored if `[build]` is present) |

## [build]

Build a custom Docker image.

```toml
[build]
dockerfile = "Dockerfile"     # Path to Dockerfile (relative to config)
context = "."                 # Build context directory
target = "runtime"            # Multi-stage build target
no_cache = false              # Disable build cache

[build.args]
NODE_VERSION = "22"           # Build arguments
```

| Field | Type | Description |
|-------|------|-------------|
| `dockerfile` | string | Path to Dockerfile |
| `context` | string | Build context (default: Dockerfile's directory) |
| `target` | string | Multi-stage build target |
| `no_cache` | bool | Force rebuild without cache |
| `args` | table | Build arguments passed to `docker build` |

When `dockerfile` is specified, `agentkernel sandbox create` automatically builds the image.

## [agent]

AI agent settings.

```toml
[agent]
preferred = "claude"          # Agent type
compatibility_mode = "claude" # Compatibility adjustments
```

| Field | Type | Values |
|-------|------|--------|
| `preferred` | string | `claude`, `codex`, `gemini`, `opencode` |
| `compatibility_mode` | string | Same as preferred |

## [resources]

Resource limits.

```toml
[resources]
vcpus = 2                     # Virtual CPUs
memory_mb = 1024              # Memory in MB
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `vcpus` | int | 1 | Number of virtual CPUs |
| `memory_mb` | int | 512 | Memory limit in megabytes |

## [security]

Security and isolation settings.

```toml
[security]
profile = "moderate"          # Security profile preset
network = true                # Allow network access
mount_cwd = true              # Mount current directory
mount_home = false            # Mount home directory
pass_env = false              # Pass host environment variables
read_only = false             # Read-only root filesystem
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `profile` | string | `moderate` | Preset: `permissive`, `moderate`, `restrictive` |
| `network` | bool | varies | Allow network access |
| `mount_cwd` | bool | varies | Mount current working directory to `/workspace` |
| `mount_home` | bool | varies | Mount `$HOME` to `/home/user` (read-only) |
| `pass_env` | bool | varies | Pass through host environment variables |
| `read_only` | bool | varies | Make root filesystem read-only |

Individual settings override the profile defaults.

## [network]

Advanced network settings.

```toml
[network]
vsock_cid = 3                 # Vsock CID (Firecracker only)
ports = ["8080:80", "3000"]   # Port mappings (host:container or container-only)
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `vsock_cid` | int | auto | Vsock CID (Firecracker only) |
| `ports` | array | `[]` | Port mappings. Format: `"host:container"`, `"container"`, or `"host:container/udp"` |

Port mappings have no effect when network access is disabled (`[security] network = false` or `--no-network`).

## [api]

HTTP API server security settings.

```toml
[api]
api_key = "my-secret-key"
api_key_env = "AGENTKERNEL_API_KEY"
allow_sudo_exec = false
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `api_key` | string | - | Static API key for HTTP Bearer authentication. |
| `api_key_env` | string | - | Environment variable name to read the API key from (used if set). |
| `allow_sudo_exec` | bool | `false` | Allow `/exec` and `/sandboxes/{name}/exec` to run with `sudo: true` when explicitly requested. |

HTTP API authentication uses the `Authorization: Bearer <api_key>` header when enabled.

## [ssh]

SSH access configuration. When enabled, an OpenSSH server is injected into the sandbox with certificate-only authentication.

```toml
[ssh]
enabled = true                          # Enable SSH server in sandbox
port = 22                               # sshd port inside container
user = "sandbox"                        # SSH login user
cert_ttl = "30m"                        # Client certificate validity
# vault_addr = "https://vault:8200"     # Vault address for CA signing
# vault_ssh_mount = "ssh"               # Vault SSH secrets engine mount
# vault_ssh_role = "agentkernel-client" # Vault signing role
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | bool | `false` | Inject sshd into sandbox (same as `--ssh` flag) |
| `port` | int | `22` | sshd listen port inside the container |
| `user` | string | `sandbox` | Username for SSH login |
| `cert_ttl` | string | `30m` | Client certificate time-to-live (e.g. `1h`, `30m`, `3600`) |
| `vault_addr` | string | none | HashiCorp Vault address for certificate signing |
| `vault_ssh_mount` | string | `ssh` | Vault SSH secrets engine mount path |
| `vault_ssh_role` | string | `agentkernel-client` | Vault SSH signing role |

Without Vault, a per-sandbox CA keypair is generated locally. Client certs are signed on each `agentkernel ssh connect` invocation and stored in `~/.agentkernel/ssh/<name>/`.

## [[files]]

Inject files into the sandbox at startup.

```toml
[[files]]
source = ".env"               # Local file path
dest = "/app/.env"            # Path inside sandbox

[[files]]
source = "config/settings.json"
dest = "/etc/app/settings.json"
```

| Field | Type | Description |
|-------|------|-------------|
| `source` | string | Local file path (relative to config file) |
| `dest` | string | Absolute path inside sandbox |

## [orchestrator]

Configuration for Kubernetes and Nomad orchestration backends. Only needed when using `--backend kubernetes` or `--backend nomad`.

```toml
[orchestrator]
provider = "kubernetes"              # "kubernetes" or "nomad"
namespace = "agentkernel"            # Namespace for sandbox resources

# Kubernetes-specific
kubeconfig = "~/.kube/config"        # Optional, auto-detected
context = "my-cluster"               # Optional kubeconfig context
runtime_class = "gvisor"             # Optional: "gvisor", "kata"
service_account = "agentkernel-sa"   # Optional service account

# Nomad-specific
nomad_addr = "http://127.0.0.1:4646"  # Nomad API address
nomad_driver = "docker"                 # "docker", "exec", "raw_exec"
nomad_datacenter = "dc1"               # Target datacenter

# Pool settings
warm_pool_size = 10                  # Pre-warmed instances
max_pool_size = 50                   # Maximum concurrent sandboxes
max_sandboxes = 200                  # Hard cap on total sandboxes
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `provider` | string | none | `kubernetes` or `nomad` |
| `namespace` | string | `agentkernel` | Namespace for sandbox resources |
| `kubeconfig` | string | auto-detected | Path to kubeconfig file |
| `context` | string | current | Kubeconfig context |
| `runtime_class` | string | none | K8s RuntimeClass (gvisor, kata) |
| `service_account` | string | none | K8s service account |
| `nomad_addr` | string | `NOMAD_ADDR` env | Nomad API address |
| `nomad_token` | string | `NOMAD_TOKEN` env | Nomad ACL token |
| `nomad_driver` | string | `docker` | Nomad task driver |
| `nomad_datacenter` | string | `dc1` | Nomad datacenter |
| `warm_pool_size` | int | 10 | Pre-warmed idle instances |
| `max_pool_size` | int | 50 | Maximum pool capacity |
| `max_sandboxes` | int | 200 | Hard cap on total sandboxes |

See the [Orchestration Guide](../operations/index.md) for detailed usage and deployment instructions.

## [remote]

Configuration for hosted remote backends (`daytona`, `runloop`, `e2b`, `agentcomputer`).

```toml
[remote]
default_profile = "node-dev"
bridge = "./scripts/remote-bridge.mjs"
sync_mode = "managed"

[remote.daytona]
api_key_env = "DAYTONA_API_KEY"
organization = "acme"

[remote.runloop]
api_key_env = "RUNLOOP_API_KEY"

[remote.e2b]
api_key_env = "E2B_API_KEY"

[remote.agentcomputer]
api_key_env = "AGENTCOMPUTER_API_KEY"

[remote.profiles.node-dev]
image = "node:22"
workspace_dir = "/workspace"
bootstrap = "npm install"

[remote.profiles.node-dev.env]
NODE_ENV = "development"
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `default_profile` | string | none | Remote runtime profile used when no profile is selected |
| `bridge` | string | `scripts/remote-bridge.mjs` | Custom remote bridge executable or script |
| `sync_mode` | string | `managed` | Remote workspace sync mode |

### [remote.<provider>]

Supported providers: `daytona`, `runloop`, `e2b`, `agentcomputer`.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `api_key` | string | none | Inline provider API key |
| `api_key_env` | string | none | Environment variable containing the API key |
| `base_url` | string | provider default | Override API base URL |
| `organization` | string | none | Provider organization or team |
| `project` | string | none | Provider project/workspace name |
| `region` | string | none | Default provider region |
| `profile` | string | none | Provider-specific default runtime profile |

### [remote.profiles.<name>]

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `image` | string | none | Provider-neutral runtime hint used by the bridge |
| `workspace_dir` | string | `/workspace` | Workspace root inside the remote sandbox |
| `bootstrap` | string | none | Startup/bootstrap command for the remote runtime |
| `env` | table | `{}` | Environment variables injected into the remote runtime |

## Full Example

```toml
[sandbox]
name = "my-fullstack-app"

[build]
dockerfile = "Dockerfile.dev"
context = "."

[build.args]
NODE_VERSION = "22"

[agent]
preferred = "claude"

[resources]
vcpus = 4
memory_mb = 2048

[security]
profile = "moderate"
network = true
mount_cwd = true

[network]
ports = ["3000:3000", "8080:80"]

[ssh]
enabled = true
cert_ttl = "1h"

[[files]]
source = ".env.development"
dest = "/app/.env"
```
