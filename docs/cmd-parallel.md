
# agentkernel parallel

Run multiple jobs concurrently. Each job runs in its own sandbox. Results are collected and reported when all jobs finish.

## Usage

```bash
agentkernel parallel --job <JOB> [--job <JOB>...] [-B <BACKEND>]
```

## Options

| Option | Description |
|--------|-------------|
| `-j, --job <JOB>` | Job spec (repeatable). Format: `name:image:command` or `name:image:tag:command` |
| `-B, --backend <BACKEND>` | Backend to use |

## Job Format

Jobs are specified as colon-separated strings:

```
name:image:command              # Image without tag (uses :latest)
name:image:tag:command          # Image with explicit tag
```

Examples:
- `lint:node:npx eslint .` — uses `node:latest`
- `test:node:22-alpine:npm test` — uses `node:22-alpine`
- `build:rust:1.85-alpine:cargo build` — uses `rust:1.85-alpine`

## Examples

### Run lint and test in parallel

```bash
$ agentkernel parallel \
    --job "lint:node:22-alpine:npx eslint ." \
    --job "test:node:22-alpine:npm test" \
    -B docker
Running 2 jobs in parallel...

  lint done (1.2s)
  test done (3.4s)

All jobs passed (3.4s wall time)
```

### Fan-out across languages

```bash
agentkernel parallel \
    --job "python:python:3.12-alpine:python3 -c 'print(1+1)'" \
    --job "node:node:22-alpine:node -e 'console.log(1+1)'" \
    --job "ruby:ruby:3.3-alpine:ruby -e 'puts 1+1'" \
    -B docker
```

### Handle failures

```bash
$ agentkernel parallel \
    --job "pass:alpine:3.20:echo ok" \
    --job "fail:alpine:3.20:false" \
    -B docker
Running 2 jobs in parallel...

  pass done (0.2s)
  fail FAILED (0.2s)

Error: Some jobs failed (0.2s wall time)
```

The exit code is non-zero if any job fails.

## How It Works

1. All jobs are spawned concurrently using `tokio::spawn`
2. Each job tries ephemeral execution first, then falls back to full sandbox lifecycle
3. Results are collected and displayed as jobs complete
4. Wall time reflects the slowest job (parallel execution)

## See Also

- [Pipeline](cmd-pipelines) - Sequential multi-step execution with data flow
- [Run](cmd-run) - Run a single command
