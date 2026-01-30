
# agentkernel exec / attach

Run commands and interact with running sandboxes.

## exec

Execute a command in a running sandbox and return the output.

### Usage

```bash
agentkernel exec [OPTIONS] <NAME> -- <COMMAND>...
```

### Options

| Option | Description |
|--------|-------------|
| `-e, --env <KEY=VALUE>` | Set environment variable (can be repeated) |

### Examples

```bash
# Run a simple command
agentkernel exec my-sandbox -- echo "hello"

# Run with arguments
agentkernel exec my-sandbox -- ls -la /workspace

# Pass environment variables
agentkernel exec my-sandbox -e API_KEY=secret -- ./script.sh

# Multiple environment variables
agentkernel exec my-sandbox \
  -e ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY \
  -e DEBUG=true \
  -- claude -p "Hello"

# Run a shell command
agentkernel exec my-sandbox -- sh -c "echo \$HOME && pwd"
```

### Output

The command's stdout is printed to your terminal. Exit code is passed through.

```bash
$ agentkernel exec my-sandbox -- python3 -c "print(1+1)"
2

$ echo $?
0
```

---

## attach

Open an interactive shell session in a running sandbox.

### Usage

```bash
agentkernel attach [OPTIONS] <NAME>
```

### Options

| Option | Description |
|--------|-------------|
| `-e, --env <KEY=VALUE>` | Set environment variable (can be repeated) |
| `--record` | Record the session in asciicast v2 format |

### Examples

```bash
# Attach to a sandbox
agentkernel attach my-sandbox

# Record the session
agentkernel attach my-sandbox --record
# Recording saved to ~/.agentkernel/recordings/my-sandbox-YYYYMMDD-HHMMSS.cast

# Attach with environment variables
agentkernel attach my-sandbox -e ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY

# Inside the sandbox, you get a shell
$ whoami
developer
$ pwd
/workspace
$ exit
```

### Running AI Agents Interactively

```bash
# Start Claude Code in a sandbox
agentkernel attach claude-sandbox -e ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY

# Inside the sandbox:
$ claude
# Claude Code starts in interactive mode
```

### Session Recording

The `--record` flag records terminal I/O in [asciicast v2](https://github.com/asciinema/asciinema/blob/develop/doc/asciicast-v2.md) format. Recordings capture:

- **Output events** — everything written to the terminal
- **Timing** — sub-millisecond timestamps relative to session start
- **Terminal dimensions** — detected from your terminal at recording start (falls back to 80x24)

Recordings are saved to `~/.agentkernel/recordings/<name>-YYYYMMDD-HHMMSS.cast`.

Play back with `agentkernel replay` or any asciicast-compatible player (e.g., [asciinema](https://asciinema.org/)):

```bash
agentkernel replay ~/.agentkernel/recordings/my-sandbox-20260130-120000.cast
agentkernel replay session.cast --speed 2.0 --max-idle 1.0
```

### PTY Support

`attach` provides a proper pseudo-terminal (PTY), supporting:

- Tab completion
- Arrow keys / history
- Ctrl+C to interrupt
- Full-screen applications (vim, less, etc.)

---

## exec vs attach

| Feature | exec | attach |
|---------|------|--------|
| Interactive | No | Yes |
| Returns output | Yes | No (direct to terminal) |
| PTY support | No | Yes |
| For scripts | Yes | No |
| For debugging | Limited | Yes |

Use `exec` for:
- Running commands in scripts
- Capturing output programmatically
- CI/CD pipelines

Use `attach` for:
- Interactive development
- Running AI agents
- Debugging
