
# agentkernel images & build

Manage Docker images and build custom images from Dockerfiles.

## Subcommands

| Command | Description |
|---------|-------------|
| `build -t <name> <context> [--dockerfile PATH]` | Build a custom image from Dockerfile |
| `images list [--all]` | List Docker images with sandbox usage |
| `images local-list` | List locally built custom images |
| `images local-delete <name>` | Delete a locally built image |
| `images local-sync` | Sync metadata with actual Docker images |
| `images prune [--agentkernel-only]` | Remove unused images |
| `images pull <IMAGE>` | Pre-pull a Docker image |

## Examples

### Build a custom image

Build custom images with your tools pre-installed:

```bash
# Build from Dockerfile in current directory
agentkernel build -t my-tools .

# Build with a specific Dockerfile
agentkernel build -t python-ml ./docker --dockerfile ./docker/Dockerfile.ml
```

Example Dockerfile:

```dockerfile
FROM python:3.12-alpine
RUN pip install numpy pandas scikit-learn
```

Use custom images in sandboxes:

```bash
agentkernel create ml-sandbox --image my-tools
agentkernel start ml-sandbox
agentkernel exec ml-sandbox -- python3 -c "import numpy; print(numpy.__version__)"
```

### List locally built images

```bash
$ agentkernel images local-list
NAME       SIZE      BUILT AT             DOCKER TAG
my-tools   145MB     2025-01-20 10:30:00  agentkernel-my-tools
python-ml  312MB     2025-01-20 11:00:00  agentkernel-python-ml
```

### Delete a local image

```bash
agentkernel images local-delete my-tools
```

### Sync local image metadata

If you manually delete Docker images, sync the metadata:

```bash
agentkernel images local-sync
```

### List images

```bash
# Show agentkernel-related images only
$ agentkernel images list
No agentkernel images found. Use --all to show all images.

# Show all Docker images with usage info
$ agentkernel images list --all
REPOSITORY:TAG                           IMAGE ID        USED BY            SIZE
python:3.12-alpine                       82585c9f05cf    1 sandbox        79.7MB
alpine:3.20                              a4f4213abb84    2 sandboxes      13.7MB
node:22-alpine                           d7119ab9e005    unused            307MB

3 images, 3 sandbox references
```

The "USED BY" column shows how many sandbox configs reference each image.

### Pre-pull an image

```bash
agentkernel images pull python:3.12-alpine
agentkernel images pull node:22-alpine
```

Pre-pulling avoids download delays during `create` or `run`.

### Prune unused images

```bash
# Remove dangling Docker images
agentkernel images prune

# Remove only agentkernel-built images not used by any sandbox
agentkernel images prune --agentkernel-only
```

## See Also

- [Snapshots](cmd-snapshots) - Snapshot images appear in the images list
- [Clean](commands) - `clean --all` removes images and build cache
