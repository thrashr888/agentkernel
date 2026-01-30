# Plan: `agentkernel plugin install` Command

## Problem

Plugin files live in the git repo (`plugins/`, `claude-plugin/`). Homebrew users only get the binary. `cp -r plugins/opencode/.opencode/ .opencode/` doesn't work without the repo.

## Command UX

```
agentkernel plugin install claude       # Claude Code native plugin
agentkernel plugin install codex        # Codex MCP config
agentkernel plugin install gemini       # Gemini MCP config
agentkernel plugin install opencode     # OpenCode TypeScript plugin
agentkernel plugin install mcp          # Generic MCP config
agentkernel plugin install all          # All plugins

agentkernel plugin list                 # Show available plugins + install status

Flags:
  --global, -g     Install to user-level config (~/) instead of cwd
  --force, -f      Overwrite existing files
  --dry-run        Show what would be written
```

**Why `plugin` and not `setup`**: `setup` already handles infrastructure (kernel/rootfs/firecracker). Plugin installation is a separate concern. `plugin install` follows `kubectl plugin`, `asdf plugin` pattern and is extensible (`plugin list`, future `plugin remove`).

## What Gets Installed

| Target | Files Written | Strategy |
|--------|--------------|----------|
| claude | `.claude-plugin/plugin.json`, `commands/sandbox.md`, `skills/sandbox/SKILL.md` | Create files |
| codex | `.mcp.json` (agentkernel entry) | Merge into JSON |
| gemini | `.gemini/settings.json` (agentkernel entry) | Merge into JSON |
| opencode | `.opencode/package.json`, `.opencode/plugins/agentkernel.ts` | Create files |
| mcp | `.mcp.json` (agentkernel entry) | Merge into JSON |

**Create** = write file, skip if identical, warn if different (unless `--force`).
**Merge** = parse existing JSON, add `mcpServers.agentkernel` entry preserving all other keys.

## Embed Strategy

`include_str!()` at compile time. All plugin files are small text (~19KB total). No network needed, no version mismatch.

## Files to Create/Modify

### New: `src/plugin_installer.rs` (~250 lines)

- Embedded constants via `include_str!("../claude-plugin/...")`
- `PluginTarget` enum: Claude, Codex, Gemini, OpenCode, Mcp
- `install_plugin(target, opts)` — main entry point
- `install_create()` — write file, handle conflicts
- `install_merge_mcp()` — JSON merge for MCP configs
- `list_plugins()` — show targets + install status
- Tests: target parsing, embedded file validity, merge logic, skip-when-identical

### Modified: `src/main.rs` (~40 lines added)

- Add `mod plugin_installer;`
- Add `Plugin { action: PluginAction }` to `Commands` enum
- Add `PluginAction` subcommand enum (Install, List)
- Add match arm dispatching to `plugin_installer`

## User-Facing Output

```
$ agentkernel plugin install opencode
Installing OpenCode plugin...

  + .opencode/package.json
  + .opencode/plugins/agentkernel.ts

OpenCode plugin installed.
  Start the agentkernel server first: agentkernel serve
  Then launch OpenCode -- the plugin loads automatically.
```

```
$ agentkernel plugin install codex
Installing Codex plugin...

  ~ .mcp.json (merged)

Codex MCP config written.
  The agentkernel MCP server will be available in Codex.
```

```
$ agentkernel plugin list
TARGET       DESCRIPTION                    STATUS
------------------------------------------------------------
claude       Claude Code plugin + /sandbox  not installed
codex        Codex MCP server config        installed
gemini       Gemini CLI MCP server config   not installed
opencode     OpenCode TypeScript plugin     not installed
mcp          Generic MCP server config      installed
```

## Implementation Order

1. Create `src/plugin_installer.rs` with embedded constants + `PluginTarget` enum
2. Implement `install_create()` and `install_merge_mcp()`
3. Implement `install_plugin()` and `list_plugins()`
4. Add `Plugin` command + `PluginAction` enum to `src/main.rs`
5. Wire up match arm
6. Write tests
7. `cargo fmt -- --check && cargo clippy -- -D warnings && cargo test`
8. Update docs to reference `agentkernel plugin install <agent>` instead of `cp` commands

## Edge Cases

- File exists with same content: skip ("already up to date")
- File exists with different content: skip unless `--force`
- `--global` with claude/opencode: error (per-project only)
- Existing `.mcp.json` has other servers: merge preserves them
- Codex and MCP both target `.mcp.json`: second install sees it's configured

## Verification

```bash
# Build and test
cargo fmt -- --check && cargo clippy -- -D warnings && cargo test

# Manual test
agentkernel plugin list
agentkernel plugin install claude --dry-run
agentkernel plugin install opencode
agentkernel plugin install codex
agentkernel plugin list  # should show installed
```
