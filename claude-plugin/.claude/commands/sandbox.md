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
/sandbox --profile restrictive <command>
/sandbox --no-network <command>
```

## Examples

```
/sandbox python3 -c "print('Hello from sandbox!')"
/sandbox npm test
/sandbox cargo build
/sandbox --profile restrictive python3 untrusted_script.py
/sandbox --no-network python3 -c "import os; print(os.getcwd())"
```

## Security Profiles

- `permissive` - Network, mounts, environment passthrough enabled
- `moderate` (default) - Network enabled, no mounts, no env passthrough
- `restrictive` - No network, read-only filesystem, all capabilities dropped

## Instructions

When the user invokes `/sandbox <command>`:

1. Parse the command and any flags from the user's input
2. Run it using agentkernel:

```bash
# Basic usage
agentkernel run <command>

# With security profile
agentkernel run --profile restrictive <command>

# Disable network
agentkernel run --no-network <command>
```

3. Display the output to the user
4. If the command fails, show the error message

The command will auto-detect the appropriate runtime based on what's being executed.
