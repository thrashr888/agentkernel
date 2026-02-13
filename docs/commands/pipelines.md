
# agentkernel pipeline

Run a multi-step agent pipeline. Each step runs in its own sandbox, and output from one step can be piped as input to the next.

## Usage

```bash
agentkernel pipeline [OPTIONS] <FILE>
```

## Options

| Option | Description |
|--------|-------------|
| `-B, --backend <BACKEND>` | Backend to use for pipeline sandboxes |

## Pipeline File Format

Pipelines are defined in TOML:

```toml
[[step]]
name = "generate"
image = "python:3.12-alpine"
command = "python generate_data.py"
output = "/app/output/"

[[step]]
name = "process"
image = "node:22-alpine"
command = "node process.js"
input = "/app/input/"
output = "/app/results/"

[[step]]
name = "analyze"
image = "python:3.12-alpine"
command = "python analyze.py"
input = "/app/input/"
```

### Step Fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | Yes | Step name (must be unique, no `/` or `..`) |
| `image` | Yes | Docker image for this step |
| `command` | Yes | Command to execute (split on whitespace) |
| `input` | No | Directory inside sandbox to receive previous step's output |
| `output` | No | Directory inside sandbox to pass to next step |

## Examples

### Run a pipeline

```bash
$ agentkernel pipeline pipeline.toml
  [1/3] generate             done (2.1s)
  [2/3] process              done (1.5s)
  [3/3] analyze              done (0.8s)
  Done (4.4s total)
```

### With a specific backend

```bash
agentkernel pipeline pipeline.toml -B docker
```

## How It Works

1. Steps execute sequentially in order
2. Each step gets its own sandbox (created, started, executed, stopped, removed)
3. If a step defines `output`, files in that directory are copied to a temporary host directory
4. If the next step defines `input`, those files are copied into the sandbox before execution
5. Pipeline stops on first failure
6. All temporary directories are cleaned up after the pipeline completes

## Validation

- Step names must be unique
- Step names cannot contain `/`, `\`, or `..` (path traversal prevention)
- Pipeline must have at least one step

## See Also

- [Parallel](parallel.md) - Run independent jobs concurrently
- [Run](run.md) - Run a single command in a temporary sandbox
