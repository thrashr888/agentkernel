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
it as not found. The daemon has no service supervising it, so it can die
silently (observed: dead since a stale `daemon.sock` dated months earlier)
while the API server keeps serving.

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

Note: `agentkernel daemon start -b` (background mode) is unimplemented in
0.21.0 — it prints "Background mode not yet implemented. Run in foreground
for now." Foreground under nohup is the working shape until then.

## Cause 2: the API server is running a deleted binary

If the API server process predates a rebuild of the `agentkernel` binary, it
keeps running the old (deleted) build while the CLI is new. The two disagree
about sandbox state, and start fails even with the daemon up.

Diagnose:

```
ls -l /proc/<server-pid>/exe
# ... -> /home/paul/.cargo/bin/agentkernel (deleted)   means stale
```

Fix: restart the server so it runs the current binary. The
`agentkernel.service` unit has `Restart=always`, so stopping the old process
brings it back on the new binary:

```
kill <server-pid>   # systemd restarts it; confirm the new exe link has no "(deleted)" suffix
```

## Verify the fix

Run the full lifecycle with a disposable sandbox and confirm each step:

```
agentkernel sandbox create <probe> -B firecracker   # expect: created, then started
agentkernel exec <probe> echo hello-from-firecracker # expect: hello-from-firecracker
agentkernel sandbox remove <probe>                   # expect: removed
agentkernel sandbox list                            # expect: 0 probe sandboxes remain
```

Also confirm the restart did not disturb the existing registry: the
pre-existing sandboxes (e.g. stopped `gym-*` entries) should still list.

## Prevention

- After replacing the `agentkernel` binary, restart the API server. A version
  check between CLI and server would make this self-evident; until one exists,
  the `/proc/<pid>/exe` check above is the test.
- The pool daemon still wants a real service (systemd user unit, or folded
  into `agentkernel.service`) so it survives reboots. Nohup survives
  disconnect, not reboot — without a service, this failure can recur silently.
