
# agentkernel snapshot

Save and restore sandbox state. Snapshots capture the full filesystem of a running or stopped sandbox using `docker commit`.

## Subcommands

| Command | Description |
|---------|-------------|
| `snapshot take <SANDBOX> [--name NAME]` | Take a snapshot of a sandbox |
| `snapshot list` | List all snapshots |
| `snapshot delete <NAME>` | Delete a snapshot (removes image + metadata) |
| `snapshot restore <NAME> [--as NAME] [-B BACKEND]` | Restore a sandbox from a snapshot |

## Examples

### Take a snapshot

```bash
# Auto-generated name (sandbox-YYYYMMDD)
agentkernel snapshot take my-sandbox

# Custom name
agentkernel snapshot take my-sandbox --name before-upgrade
```

### List snapshots

```bash
$ agentkernel snapshot list
NAME                      SANDBOX              BACKEND      CREATED
before-upgrade            my-sandbox           docker       2026-02-01T12:00:00+00:00
daily-backup              dev-env              docker       2026-02-01T18:00:00+00:00
```

### Restore a snapshot

```bash
# Restore with default name (original-restored)
agentkernel snapshot restore before-upgrade

# Restore with a custom name
agentkernel snapshot restore before-upgrade --as my-sandbox-v2

# Restore with a specific backend
agentkernel snapshot restore before-upgrade --as restored -B docker
```

### Delete a snapshot

```bash
agentkernel snapshot delete before-upgrade
```

## How It Works

1. `snapshot take` runs `docker commit` on the sandbox container, saving the filesystem as a tagged Docker image (`agentkernel-snap:<name>`)
2. Metadata (sandbox name, backend, resources, base image) is saved to `~/.local/share/agentkernel/snapshots/<name>.json`
3. `snapshot restore` creates a new sandbox using the committed image and the original resource settings

## Storage

- Snapshot metadata: `~/.local/share/agentkernel/snapshots/*.json`
- Snapshot images: Docker images tagged as `agentkernel-snap:<name>`
- Use `images list --all` to see snapshot images and their disk usage

## See Also

- [Sessions](cmd-sessions) - `session save` combines snapshots with session metadata
- [Images](cmd-images) - Manage Docker images including snapshot images
