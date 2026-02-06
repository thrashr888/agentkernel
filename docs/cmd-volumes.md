
# agentkernel volume

Manage persistent volumes that survive sandbox restarts. Volumes are stored in `~/.agentkernel/volumes/`.

## Subcommands

| Command | Description |
|---------|-------------|
| `volume create <slug> [--size SIZE]` | Create a new volume |
| `volume list` | List all volumes |
| `volume info <slug>` | Show volume details |
| `volume delete <slug> [--force]` | Delete a volume |

## Examples

### Create a volume

```bash
# Create a volume with default settings
agentkernel volume create mydata

# Create with a size hint (for display only)
agentkernel volume create cache --size 2GB
```

Size accepts: `B`, `KB`, `MB`, `GB` suffixes (e.g., `512MB`, `2GB`).

### List volumes

```bash
$ agentkernel volume list
SLUG       SIZE      CREATED              MOUNTS
mydata     1.2MB     2025-01-20 10:30:00  3
cache      0B        2025-01-20 11:00:00  0
```

### Use a volume with a sandbox

Mount volumes when creating a sandbox with `--volume` or `-v`:

```bash
# Mount mydata volume at /data
agentkernel create dev --volume mydata:/data

# Mount read-only
agentkernel create dev --volume mydata:/data:ro

# Multiple volumes
agentkernel create dev \
  --volume mydata:/data \
  --volume cache:/cache
```

The volume must exist before mounting. Create it first:

```bash
agentkernel volume create mydata
agentkernel create dev --volume mydata:/data
agentkernel start dev
agentkernel exec dev -- ls /data
```

### Volume info

```bash
$ agentkernel volume info mydata
Slug:       mydata
Size:       1.2MB
Created:    2025-01-20 10:30:00
Last Used:  2025-01-20 14:00:00
Mounts:     3
Path:       /Users/user/.agentkernel/volumes/mydata
```

### Delete a volume

```bash
# Delete (will prompt if in use)
agentkernel volume delete mydata

# Force delete without prompt
agentkernel volume delete mydata --force
```

## Volume Format

Volume mount specs follow the format: `slug:/container/path[:ro]`

- `slug` - Volume identifier (alphanumeric, hyphens, underscores)
- `/container/path` - Mount point inside the sandbox
- `:ro` - Optional read-only flag

Examples:
- `mydata:/data` - Read-write mount
- `mydata:/data:ro` - Read-only mount
- `project-cache:/home/user/.cache` - User cache directory

## Storage Location

Volumes are stored in `~/.agentkernel/volumes/<slug>/`. The directory is created when you create the volume and persists across sandbox restarts.

## See Also

- [Create](cmd-create) - `--volume` flag
- [Images](cmd-images) - Image management
