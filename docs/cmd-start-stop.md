---
title: start / stop
permalink: /cmd-start-stop.html
sidebar: agentkernel_sidebar
topnav: topnav
---

# agentkernel start / stop

Manage the lifecycle of persistent sandboxes.

## start

Start a stopped sandbox.

### Usage

```bash
agentkernel start [OPTIONS] <NAME>
```

### Options

| Option | Description |
|--------|-------------|
| `--backend <BACKEND>` | Override backend (usually auto-detected from saved state) |

### Examples

```bash
# Start a sandbox
agentkernel start my-sandbox

# Verify it's running
agentkernel list
```

### What Happens

1. Loads sandbox configuration from disk
2. Uses the backend that was used when creating the sandbox
3. Starts the container/VM
4. Sandbox is now ready for `exec` or `attach`

---

## stop

Stop a running sandbox. The sandbox state is preserved and can be started again.

### Usage

```bash
agentkernel stop <NAME>
```

### Examples

```bash
# Stop a sandbox
agentkernel stop my-sandbox

# Verify it's stopped
agentkernel list
# NAME          STATUS     BACKEND
# my-sandbox    stopped    docker
```

### What Happens

1. Sends stop signal to the container/VM
2. Waits for graceful shutdown
3. Sandbox state is preserved on disk
4. Can be started again with `agentkernel start`

---

## remove

Permanently delete a sandbox and its state.

### Usage

```bash
agentkernel remove <NAME>
```

### Examples

```bash
# Remove a stopped sandbox
agentkernel remove my-sandbox

# Force remove a running sandbox (stops it first)
agentkernel remove my-sandbox
```

### What Happens

1. Stops the sandbox if running
2. Removes the container/VM
3. Deletes saved state from `~/.local/share/agentkernel/sandboxes/`

---

## Lifecycle Summary

```
create  ──>  start  ──>  stop  ──>  start  ──>  ...  ──>  remove
              │                       │
              └──── exec/attach ──────┘
```

| State | Can exec/attach? | Persisted? |
|-------|------------------|------------|
| Created (not started) | No | Yes |
| Running | Yes | Yes |
| Stopped | No | Yes |
| Removed | - | No |
