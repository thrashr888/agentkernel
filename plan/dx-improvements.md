# Sandbox DX Improvements (P2 Gaps)

Three features to close the biggest DX gaps identified in the competitive analysis. All touch the same code paths (create, exec, file ops) across CLI, HTTP API, and MCP.

## Feature 1: Per-command workdir/env/sudo (`agentkernel-dd8`)

Add `--workdir`, `--sudo` to exec (CLI already has `--env`). Propagate all three to HTTP API and MCP.

### Changes

**`src/docker_backend.rs`** — `exec()` method (line ~142):
- Add `workdir: Option<&str>`, `env: &[String]`, `user: Option<&str>` params
- Map to `docker exec -w <workdir> -e K=V -u <user>` flags
- Keep existing `exec()` signature as a convenience wrapper that calls the new one with defaults

**`src/vmm.rs`** — `exec_cmd_with_env()`:
- Add `workdir` and `user` params, pass through to backend
- Add new `exec_cmd_full(&self, name, cmd, env, workdir, user)` method

**`src/main.rs`** — Exec command (line ~148):
- Add `--workdir` (`-w`) and `--sudo` flags to `Exec` struct
- `--sudo` maps to `user: Some("root")`
- Pass through to `manager.exec_cmd_full()`

**`src/http_api.rs`** — ExecRequest (line ~113):
- Add `workdir: Option<String>`, `env: Option<Vec<String>>`, `sudo: Option<bool>` fields
- Pass to manager in exec handler

**`src/mcp.rs`** — sandbox_exec tool (line ~244):
- Add `workdir`, `env`, `sudo` to input schema
- Parse and pass to manager

## Feature 2: Git source cloning on create (`agentkernel-1lk`)

Support `agentkernel create mybox --source git:URL` to clone a repo into the sandbox at creation time.

### Changes

**`src/main.rs`** — Create command (line ~87):
- Add `--source` arg: accepts `git:URL` or plain URL (defaults to git)
- Add `--git-ref` for checkout ref (separate from existing `--branch` which is for auto-naming)
- After sandbox starts, run `git clone URL /workspace` inside the container
- If `--git-ref` specified, run `git checkout ref` after clone

**`src/http_api.rs`** — CreateRequest (line ~56):
- Add `source: Option<SourceSpec>` where `SourceSpec` is `{ type: "git", url: String, ref: Option<String> }`
- After sandbox start, clone repo inside container

**`src/mcp.rs`** — sandbox_create tool:
- Add `source_url` and `source_ref` to input schema
- Clone after creation

No new module needed — this is orchestration around existing exec.

## Feature 3: Batch file API (`agentkernel-4t7`)

Add file operations to HTTP API and MCP. CLI already has `agentkernel cp`.

### Changes

**`src/docker_backend.rs`** — New methods:
- `write_file(name, path, content)` — Write content to file in container
- `read_file(name, path)` — Read file content from container

**`src/vmm.rs`** — Expose through manager:
- `write_file(&self, name, path, content)` and `read_file(&self, name, path)`

**`src/http_api.rs`** — New endpoints:
- `POST /sandboxes/{name}/files/write` — Write multiple files
  - Body: `{ "files": { "/path/file1": "content1", "/path/file2": "content2" } }`
- `GET /sandboxes/{name}/files/read?path=/path/to/file` — Read a single file
- `GET /sandboxes/{name}/files/download?path=/path/to/file` — Download as octet-stream

**`src/mcp.rs`** — New tools:
- `sandbox_write_files` — Write multiple files (takes `name` + `files` map)
- `sandbox_read_file` — Read single file (takes `name` + `path`)

## Not in this batch

- **Detached commands + log streaming** (`agentkernel-stc`) — Larger feature, separate branch.
- **Snapshot HTTP/MCP exposure** (`agentkernel-a7z`) — P3, separate concern.

## Implementation order

1. Per-command options (smallest, touches exec path that other features use)
2. File operations (standalone, no deps on other features)
3. Git source cloning (uses the exec path internally)

## Verification

```bash
cargo fmt -- --check && cargo clippy -- -D warnings && cargo test
```
