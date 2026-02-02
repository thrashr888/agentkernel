
# agentkernel images

Manage Docker image cache. List images with sandbox usage info, prune unused images, and pre-pull images.

## Subcommands

| Command | Description |
|---------|-------------|
| `images list [--all]` | List Docker images with sandbox usage |
| `images prune [--agentkernel-only]` | Remove unused images |
| `images pull <IMAGE>` | Pre-pull a Docker image |

## Examples

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
