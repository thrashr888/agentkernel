
# agentkernel sandbox start / stop / pause / resume / fork

Manage the lifecycle of persistent sandboxes.

`start` and `stop` are the portable lifecycle operations. Firecracker sandboxes
on x86_64 Linux/KVM also support full-state `pause`, `resume`, and `fork`, which
preserve guest memory and process state.

## start

Start a stopped sandbox.

Firecracker sandboxes are started through the long-running local API server so
the VM process survives after the CLI exits. Run `agentkernel serve` first;
`AGENTKERNEL_PORT` selects a non-default local port. If the server requires an
API key, set `AGENTKERNEL_API_KEY` for delegated CLI requests.

Before delegating, the CLI resolves the saved `agentkernel.toml` permissions
and startup file injections. It stores that resolved configuration in a private
host manifest and sends only an unguessable one-shot reference in the local
start request. The manifest is bound to the sandbox UUID and owner, consumed on
the first attempt, expires after five minutes, and is scrubbed on removal. A
valid reference can authorize the server's first claim of an unowned local
sandbox even after the server restarted or previously inspected it. This
preserves the selected profile, network and mount behavior, resource limits, and
file contents while the server retains VM ownership, without exposing a public
capability-escalation payload.

### Usage

```bash
agentkernel sandbox start [OPTIONS] <NAME>
```

### Options

| Option | Description |
|--------|-------------|
| `-B, --backend <BACKEND>` | Override backend (usually auto-detected from saved state) |

### Examples

```bash
# Start a sandbox
agentkernel sandbox start my-sandbox

# Verify it's running
agentkernel sandbox list
```

### What Happens

1. Loads sandbox configuration from disk
2. Resolves its permissions and startup files
3. Uses the backend that was used when creating the sandbox
4. Starts the container/VM (through the local server for Firecracker)
5. Sandbox is ready for `exec`; backends with interactive terminal support also allow `attach`

Server-owned Firecracker sandboxes do not support interactive `attach`. Use
`agentkernel exec <name> -- <command>` or `agentkernel ssh <name>` when SSH is
configured.

---

## stop

Stop a running sandbox. Container/provider backends preserve their normal
backend state and can be started again.

For Firecracker, the CLI delegates stop to the local API server that owns the
VM process. A Firecracker VM restored or forked from a full-state checkpoint
rejects ordinary `stop`, because its writable disk is currently tied to that
runtime and a cold `start` would reset it. Use `pause` to preserve it or
`remove` to discard it. Ordinary stop on a disposable Firecracker sandbox does
not preserve writable disk changes.

### Usage

```bash
agentkernel sandbox stop <NAME>
```

### Examples

```bash
# Stop a sandbox
agentkernel sandbox stop my-sandbox

# Verify it's stopped
agentkernel sandbox list
# NAME          STATUS     BACKEND
# my-sandbox    stopped    docker
```

### What Happens

1. Sends stop signal to the container/VM
2. Waits for graceful shutdown
3. Sandbox configuration is preserved on disk; backend persistence rules apply
4. Can be started again when the backend's persistence contract allows it;
   full-state Firecracker lineages use `pause` and `resume` instead

---

## pause (alias: suspend)

Pause a running Firecracker sandbox into a durable full-VM checkpoint.

### Usage

```bash
agentkernel sandbox pause <NAME>
agentkernel sandbox suspend <NAME>
```

### Example

```bash
agentkernel sandbox pause experiment-a
agentkernel sandbox list
# NAME             STATUS    BACKEND
# experiment-a     paused    firecracker
```

Unlike `stop`, pause captures guest memory, process state, virtual-device
state, and an immutable disk checkpoint. The sandbox is not available for
`exec` or `attach` while paused.

This operation requires the Firecracker backend on x86_64 Linux/KVM. Other backends
return an unsupported-operation error. CLI pause, resume, and fork commands
also require a healthy local `agentkernel serve` process because it owns the
Firecracker VM.

---

## resume

Resume a paused Firecracker sandbox from its full-VM checkpoint.

### Usage

```bash
agentkernel sandbox resume <NAME>
```

### Example

```bash
agentkernel sandbox resume experiment-a
```

The sandbox continues from the captured memory and process state instead of
performing a fresh boot.

---

## fork

Create a new running sandbox from a paused Firecracker sandbox.

### Usage

```bash
agentkernel sandbox fork <SOURCE> --as <CHILD>
```

### Example

```bash
agentkernel sandbox pause experiment-a
agentkernel sandbox fork experiment-a --as experiment-b
```

`experiment-b` starts immediately from the checkpoint. `experiment-a` stays
paused, so it can be resumed or forked again. After the fork, source and child
disk state diverge independently. The child inherits the source owner and tenant
metadata atomically; cross-owner or cross-tenant forks are rejected before the
child VM is restored.

> **Security warning:** A fork copies guest memory and filesystem state. Any
> credentials captured in the checkpoint are duplicated into the child;
> rotate or revoke them when appropriate.

---

## remove

Permanently delete a sandbox and its state.

For Firecracker, removal is delegated to the local API server so a live VM is
stopped before its persisted state is deleted.

### Usage

```bash
agentkernel sandbox remove <NAME>
```

### Examples

```bash
# Remove a stopped sandbox
agentkernel sandbox remove my-sandbox

# Force remove a running sandbox (stops it first)
agentkernel sandbox remove my-sandbox
```

### What Happens

1. Stops the sandbox if running
2. Removes the container/VM
3. Deletes saved state from `~/.local/share/agentkernel/sandboxes/`

---

## Lifecycle Summary

```text
create -> start -> stop -> start -> ... -> remove
            |
            +-> pause -> resume
                   |
                   +-> fork -> running child
```

| State | Can exec/attach? | Full guest memory preserved? | Persisted? |
|-------|------------------|------------------------------|------------|
| Created (not started) | No | No | Yes |
| Running | Yes | In memory | Yes |
| Paused (Firecracker) | No | Yes | Yes |
| Stopped | No | No | Configuration only; filesystem persistence is backend-specific |
| Removed | - | No | No |
