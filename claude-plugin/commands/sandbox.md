---
name: sandbox
description: Run a command in an isolated sandbox
allowed-tools: ["Bash"]
---

# /sandbox Command

Run a command in an isolated agentkernel sandbox for security.

## Usage

```
/sandbox <command>
```

## Examples

```
/sandbox python3 -c "print('Hello from sandbox!')"
/sandbox npm test
/sandbox cargo build
```

## Instructions

When the user invokes `/sandbox <command>`:

1. Parse the command from the user's input
2. Run it using agentkernel:

```bash
agentkernel run <command>
```

3. Display the output to the user
4. If the command fails, show the error message

The command will auto-detect the appropriate runtime based on what's being executed.
