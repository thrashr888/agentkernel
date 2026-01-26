
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

### Examples

```bash
# Attach to a sandbox
agentkernel attach my-sandbox

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
