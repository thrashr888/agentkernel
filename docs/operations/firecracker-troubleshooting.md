# Firecracker troubleshooting

## Symptoms

- `agentkernel sandbox create <name> -B firecracker` records the sandbox (it
  appears in `sandbox list` as `stopped`), but `sandbox start <name>` fails with:

  ```
  500 Internal Server Error: {"success":false,"error":"Sandbox '<name>' not found"}
  ```

- The Docker backend is unaffected: its create → start → exec → remove
  lifecycle works normally.
- The error is misleading: the sandbox record exists. What is missing is the
  runtime that can start it, or the server is too old to understand the CLI.

## Cause 1: the VM pool daemon is not running

Firecracker starts go through the pool daemon (`agentkernel daemon`). If the
daemon is dead, the API server cannot resolve the sandbox to a VM and reports
it as not found. agentkernel does not install a service to supervise the
daemon, so it can die silently while the API server keeps serving. A
leftover `daemon.sock` with no listener is the usual sign.

Diagnose:

```
agentkernel daemon status
ss -x | grep daemon.sock   # 0 listeners means the socket file is stale
```

Fix:

```
rm -f ~/.agentkernel/daemon.sock   # only if nothing is listening on it
nohup agentkernel daemon start > ~/.agentkernel/daemon.log 2>&1 &
agentkernel daemon status          # expect: running, pool warm
```

Note: `agentkernel daemon start -b` (background mode) is not implemented
yet. It exits with "Background mode not yet implemented. Run in foreground
for now." Until it is, run the daemon in the foreground under `nohup` or a
service manager.

## Cause 2: the API server is running a deleted binary

If the API server process predates a rebuild of the `agentkernel` binary, it
keeps running the old (deleted) build while the CLI is new. The two disagree
about sandbox state, and start fails even with the daemon up.

Diagnose:

```
ls -l /proc/<server-pid>/exe
# ... -> /usr/local/bin/agentkernel (deleted)   a "(deleted)" suffix means stale
```

Fix: restart the server so it runs the current binary. If a service manager
runs it (for example a systemd unit with `Restart=always`), restart that
unit. Otherwise stop the old process and start `agentkernel serve` again.
Then confirm the new `/proc/<pid>/exe` link has no "(deleted)" suffix.

## Verify the fix

Run the full lifecycle with a disposable sandbox and confirm each step:

```
agentkernel sandbox create <probe> -B firecracker   # expect: created, then started
agentkernel exec <probe> echo hello-from-firecracker # expect: hello-from-firecracker
agentkernel sandbox remove <probe>                   # expect: removed
agentkernel sandbox list                            # expect: 0 probe sandboxes remain
```

Also confirm the restart did not disturb the existing registry: sandboxes
that existed before the fix should still be listed.

## Prevention

- After replacing the `agentkernel` binary, restart the API server. A version
  check between CLI and server would make this self-evident; until one exists,
  the `/proc/<pid>/exe` check above is the test.
- Run the pool daemon under a service manager (for example a systemd user
  unit) so it survives reboots. `nohup` survives a disconnect, not a reboot,
  so without a service this failure can recur silently.
