
# agentkernel session

Manage agent sessions. A session ties together a sandbox, an agent type, and lifecycle metadata (creation time, exec count, status). Sessions can be stopped, saved (snapshotted), and resumed.

## Subcommands

| Command | Description |
|---------|-------------|
| `session start --name NAME --agent AGENT [-B BACKEND]` | Start a new session |
| `session list` | List all sessions |
| `session stop <NAME>` | Stop a session (stops sandbox, keeps metadata) |
| `session save <NAME>` | Save a session (snapshot sandbox + mark saved) |
| `session resume <NAME>` | Resume a stopped or saved session |
| `session delete <NAME>` | Delete a session and clean up |

## Examples

### Start a session

```bash
agentkernel session start --name feature-x --agent claude -B docker
# Output:
# Session 'feature-x' started.
#   Sandbox: session-feature-x
# Attach with: agentkernel attach session-feature-x
```

The sandbox is named `session-<name>`. Use `exec` or `attach` to interact with it.

### List sessions

```bash
$ agentkernel session list
NAME                 AGENT      STATUS     DURATION      EXECS
feature-x            claude     running    2h 14m            5
bugfix-123           gemini     saved      1d                12
old-session          codex      stopped    3d                0
```

### Work in a session

```bash
agentkernel exec session-feature-x -- python3 -c "print('working')"
agentkernel attach session-feature-x
```

### Save a session (snapshot)

```bash
# Creates a snapshot and marks the session as saved
agentkernel session save feature-x
# Output: Session 'feature-x' saved (snapshot: feature-x-20260201)
```

### Resume a session

```bash
# Restarts the sandbox and marks session as running
agentkernel session resume feature-x
```

### Stop a session

```bash
# Stops the sandbox but keeps session metadata for later resume
agentkernel session stop feature-x
```

### Delete a session

```bash
# Removes session metadata (does not remove snapshots)
agentkernel session delete feature-x
```

## Session Status

| Status | Description |
|--------|-------------|
| `running` | Sandbox is active |
| `stopped` | Sandbox stopped, can be resumed |
| `saved` | Sandbox snapshotted, can be resumed from snapshot |

## Storage

- Session metadata: `~/.local/share/agentkernel/sessions/*.json`
- Sandbox state: managed by the standard sandbox system

## See Also

- [Snapshots](cmd-snapshots) - Lower-level snapshot management
- [Agents](agents) - Supported agent types
