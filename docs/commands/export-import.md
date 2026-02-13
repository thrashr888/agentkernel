
# agentkernel sandbox export / export-config / import-config

Export sandbox filesystems and configurations, and import configurations to create new sandboxes.

## Commands

| Command | Description |
|---------|-------------|
| `sandbox export <NAME> [-o FILE]` | Export sandbox filesystem as a tar archive |
| `sandbox export-config <NAME>` | Export sandbox config as TOML (prints to stdout) |
| `sandbox import-config <FILE> [--as NAME] [-B BACKEND]` | Create a sandbox from a TOML config |

## Export Filesystem

Export the full filesystem of a sandbox as a tar archive:

```bash
# Default output: <name>.tar
agentkernel sandbox export my-sandbox

# Custom output path
agentkernel sandbox export my-sandbox -o /tmp/backup.tar
```

Output:
```
Exporting sandbox 'my-sandbox' to my-sandbox.tar...
Exported 52.7 MB to my-sandbox.tar
```

The sandbox must be running for `export` to work (it uses `docker export`).

## Export Configuration

Export a sandbox's settings as TOML for sharing or backup:

```bash
$ agentkernel sandbox export-config my-sandbox
[sandbox]
name = "my-sandbox"
base_image = "python:3.12-alpine"

[resources]
vcpus = 1
memory_mb = 512
```

Redirect to a file:

```bash
agentkernel sandbox export-config my-sandbox > my-sandbox.toml
```

## Import Configuration

Create a new sandbox from an exported TOML config:

```bash
# Use the name from the config
agentkernel sandbox import-config my-sandbox.toml

# Override the name
agentkernel sandbox import-config my-sandbox.toml --as new-sandbox

# Specify backend
agentkernel sandbox import-config my-sandbox.toml --as imported -B docker
```

Output:
```
Importing config as sandbox 'new-sandbox' (image: python:3.12-alpine)...
Sandbox 'new-sandbox' created from config.

Next steps:
  agentkernel sandbox start new-sandbox
```

## Workflow: Share a Sandbox Configuration

```bash
# On machine A: export
agentkernel sandbox export-config my-project > my-project.toml

# Transfer the file (git, email, etc.)

# On machine B: import
agentkernel sandbox import-config my-project.toml -B docker
agentkernel sandbox start my-project
```

## See Also

- [Templates](templates.md) - Reusable sandbox configurations
- [Snapshots](snapshots.md) - Save/restore full sandbox state (including filesystem)
