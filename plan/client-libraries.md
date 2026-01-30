# Plan: Client Libraries, Homebrew, and Agent Plugins

## Scope

Three workstreams:

1. **Client SDKs** - Python, Rust, Node.js libraries wrapping the HTTP REST API
2. **Homebrew formula** - `brew install agentkernel`
3. **Agent plugins** - Integration packages for OpenCode, Codex, Gemini CLI (Claude Code already done)

---

## 1. Client SDKs

All three SDKs wrap the HTTP API at `docs/openapi.yaml`. The API surface is small (7 endpoints + 1 SSE streaming endpoint), so hand-written SDKs beat OpenAPI generation for DX.

### API Surface (from `src/http_api.rs`)

| Method | Path | Body | Response |
|--------|------|------|----------|
| GET | `/health` | - | `"ok"` |
| POST | `/run` | `RunRequest` | `{ output }` |
| POST | `/run/stream` | `RunRequest` | SSE events |
| GET | `/sandboxes` | - | `SandboxInfo[]` |
| POST | `/sandboxes` | `CreateRequest` | `SandboxInfo` (201) |
| GET | `/sandboxes/{name}` | - | `SandboxInfo` |
| DELETE | `/sandboxes/{name}` | - | `"Sandbox removed"` |
| POST | `/sandboxes/{name}/exec` | `ExecRequest` | `{ output }` |

All responses wrapped in `{ success, data?, error? }`. Auth: optional `Authorization: Bearer <key>`.

### Shared Design

All three SDKs share:

- **Config resolution**: constructor args > `AGENTKERNEL_BASE_URL` / `AGENTKERNEL_API_KEY` env vars > defaults (`http://localhost:18888`, no auth, 30s timeout)
- **Error taxonomy**: `AuthError` (401), `NotFoundError` (404), `ValidationError` (400), `ServerError` (500), `NetworkError` (connection), `StreamError` (SSE)
- **User-Agent**: `agentkernel-{lang}-sdk/{version}`
- **Sandbox session**: auto-cleanup pattern (create, use, remove on scope exit)

### 1A. Python SDK (`sdk/python/`)

**Package**: `agentkernel` on PyPI. Python >= 3.10.

**Dependencies**: `httpx` (HTTP), `httpx-sse` (SSE), `pydantic` (types).

**Files**:
```
sdk/python/
  pyproject.toml                # hatchling build, deps
  README.md
  src/agentkernel/
    __init__.py                 # re-exports
    client.py                   # AgentKernel (sync)
    async_client.py             # AsyncAgentKernel
    types.py                    # Pydantic models
    errors.py                   # Exception hierarchy
    sse.py                      # SSE iterator
    _config.py                  # Config resolution
    py.typed                    # PEP 561
  tests/
    test_client.py
    test_async_client.py
    test_sse.py
    test_integration.py         # @pytest.mark.integration
  examples/
    quickstart.py
    streaming.py
    sandbox_session.py
```

**Client API**:
```python
class AgentKernel:
    def __init__(self, base_url=None, api_key=None, timeout=30.0): ...
    def health(self) -> str: ...
    def run(self, command, *, image=None, profile=None, fast=True) -> RunOutput: ...
    def run_stream(self, command, **opts) -> Iterator[StreamEvent]: ...
    def list_sandboxes(self) -> list[SandboxInfo]: ...
    def create_sandbox(self, name, *, image=None) -> SandboxInfo: ...
    def get_sandbox(self, name) -> SandboxInfo: ...
    def remove_sandbox(self, name) -> None: ...
    def exec_in_sandbox(self, name, command) -> RunOutput: ...
    def sandbox(self, name, *, image=None) -> SandboxSession: ...

class SandboxSession:
    def exec(self, command) -> RunOutput: ...
    def __enter__(self) -> SandboxSession: ...
    def __exit__(self, *args) -> None: ...  # auto-removes sandbox

# Async mirror
class AsyncAgentKernel: ...    # async def versions of all methods
class AsyncSandboxSession: ... # async with support
```

**Usage**:
```python
from agentkernel import AgentKernel

with AgentKernel() as client:
    with client.sandbox("test", image="python:3.12-alpine") as sb:
        sb.exec(["pip", "install", "numpy"])
        result = sb.exec(["python3", "-c", "import numpy; print(numpy.__version__)"])
        print(result.output)
    # sandbox auto-removed
```

**Build**: hatchling. **Publish**: `python -m build && twine upload dist/*`
**Dev deps**: `pytest`, `pytest-asyncio`, `pytest-httpx`, `mypy`, `ruff`

### 1B. Rust SDK (`sdk/rust/`)

**Crate**: `agentkernel-sdk` on crates.io. MSRV 1.75.

**Dependencies**: `reqwest` (json + stream), `serde`/`serde_json`, `tokio`, `thiserror`, `futures`, `eventsource-stream`.

**Files**:
```
sdk/rust/
  Cargo.toml
  README.md
  src/
    lib.rs              # re-exports, crate docs
    client.rs           # AgentKernel + AgentKernelBuilder
    types.rs            # serde structs/enums
    error.rs            # thiserror enum
    sse.rs              # Stream<Item = Result<StreamEvent>>
    sandbox.rs          # SandboxGuard + with_sandbox()
  tests/
    client_test.rs
    sse_test.rs
    integration.rs      # #[ignore]
  examples/
    quickstart.rs
    streaming.rs
    sandbox_guard.rs
```

**Client API**:
```rust
let client = AgentKernel::builder()
    .base_url("http://localhost:18888")
    .api_key("sk-...")
    .build()?;

let output = client.run(&["echo", "hello"]).await?;

// Scoped sandbox (guaranteed cleanup via closure)
client.with_sandbox("test", |sb| async move {
    sb.exec(&["pip", "install", "numpy"]).await?;
    Ok(())
}).await?;

// Streaming
use futures::StreamExt;
let mut stream = client.run_stream(&["python3", "script.py"]).await?;
while let Some(event) = stream.next().await { ... }
```

**Error**: `thiserror` enum (`Auth`, `NotFound`, `Validation`, `Server`, `Network`, `Stream`, `Json`).
**Drop safety**: `SandboxGuard` warns on Drop. `with_sandbox` closure guarantees cleanup.
**Build/Publish**: `cargo publish`
**Dev deps**: `wiremock`, `tokio` (rt-multi-thread + macros)

### 1C. Node.js SDK (`sdk/nodejs/`)

**Package**: `agentkernel` on npm. TypeScript-first. Node >= 20.

**Dependencies**: Only `eventsource-parser`. Uses native `fetch` (Node 20+). Zero HTTP deps.

**Files**:
```
sdk/nodejs/
  package.json
  tsconfig.json
  tsup.config.ts        # ESM + CJS dual output
  README.md
  src/
    index.ts            # re-exports
    client.ts           # AgentKernel class
    types.ts            # interfaces + string literal unions
    errors.ts           # Error subclasses
    sse.ts              # AsyncGenerator<StreamEvent>
    sandbox.ts          # SandboxSession (AsyncDisposable)
    config.ts           # Config resolution
  tests/
    client.test.ts
    sse.test.ts
    integration.test.ts
  examples/
    quickstart.ts
    streaming.ts
    sandbox-session.ts
```

**Client API**:
```typescript
const client = new AgentKernel({ apiKey: "sk-..." });

const result = await client.run(["echo", "hello"]);

// AsyncDisposable sandbox (TS 5.2+ await using)
await using sb = await client.sandbox("test", { image: "python:3.12-alpine" });
await sb.exec(["pip", "install", "numpy"]);

// Streaming via AsyncGenerator
for await (const event of client.runStream(["python3", "script.py"])) {
    if (event.type === "output") process.stdout.write(event.data.data);
}
```

**SandboxSession** implements `Symbol.asyncDispose` for `await using`.
**Build**: `tsup` (ESM + CJS + .d.ts). **Publish**: `npm publish`
**Dev deps**: `typescript`, `tsup`, `vitest`, `msw`

### Implementation Order (per SDK)

1. Types + errors
2. Client core (all non-streaming methods)
3. SSE streaming (`run_stream`)
4. Sandbox session (context manager / guard)
5. Config resolution
6. Tests (mock HTTP)
7. Examples
8. Package metadata + README

---

## 2. Homebrew Formula

### What exists

- `install.sh` builds from source via `cargo install --git`
- `.github/workflows/release.yml` already builds binaries for macOS (x64 + arm64) and Linux (x64 + arm64)
- A `homebrew-allbeads` tap exists at `/Users/thrashr888/Workspace/homebrew-allbeads` (reference for structure)

### Plan

Create a new tap repo `thrashr888/homebrew-agentkernel`.

**File**: `Formula/agentkernel.rb`
```ruby
class Agentkernel < Formula
  desc "Run AI coding agents in secure, isolated microVMs"
  homepage "https://github.com/thrashr888/agentkernel"
  version "0.2.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/thrashr888/agentkernel/releases/download/v#{version}/agentkernel-aarch64-apple-darwin.tar.gz"
      sha256 "..."
    end
    on_intel do
      url "https://github.com/thrashr888/agentkernel/releases/download/v#{version}/agentkernel-x86_64-apple-darwin.tar.gz"
      sha256 "..."
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/thrashr888/agentkernel/releases/download/v#{version}/agentkernel-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "..."
    end
    on_intel do
      url "https://github.com/thrashr888/agentkernel/releases/download/v#{version}/agentkernel-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "..."
    end
  end

  def install
    bin.install "agentkernel"
  end

  test do
    assert_match "agentkernel", shell_output("#{bin}/agentkernel --version")
  end
end
```

**Steps**:
1. Create `thrashr888/homebrew-agentkernel` repo
2. Add `Formula/agentkernel.rb`
3. Update `release.yml` to auto-update formula SHA on new releases
4. Install: `brew tap thrashr888/agentkernel && brew install agentkernel`

---

## 3. Agent Plugins

Agent plugins let AI coding agents use agentkernel as their sandbox backend. The agent runs on the host; agentkernel provides isolated code execution.

### What already exists

- **Claude Code**: Complete. MCP server (`src/mcp.rs`) + plugin manifest (`claude-plugin/`) + SKILL.md.
- **Agent examples**: `examples/agents/{claude-code,codex,gemini}/` for running agents inside agentkernel.

### 3A. OpenCode Plugin (`plugins/opencode/`)

Based on the `opencode-daytona` reference, OpenCode plugins use a `.opencode/plugin/` directory structure, published as npm packages. The plugin hooks into OpenCode's session lifecycle to route code execution through agentkernel sandboxes.

**Package**: `opencode-agentkernel` on npm.

**Structure** (following opencode-daytona pattern):
```
plugins/opencode/
  .opencode/
    plugin/
      agentkernel/
        index.ts          # Plugin entry point
        sandbox.ts        # Sandbox lifecycle management
        sync.ts           # File sync between sandbox and local
      index.ts            # Plugin registration
  dist/                   # Build output (compiled JS)
    .opencode/
      ...
  package.json
  tsconfig.json
  tsconfig.build.json
  README.md
```

**Plugin entry** (`.opencode/plugin/index.ts`):
```typescript
import type { Plugin } from "@opencode-ai/plugin";
import { AgentKernel, SandboxSession } from "agentkernel";

export const agentkernel: Plugin = async ({ project, directory }) => {
  const client = new AgentKernel();
  let sandbox: SandboxSession | null = null;

  return {
    hooks: {
      "session.created": async ({ session }) => {
        const name = `opencode-${session.id}`;
        sandbox = await client.sandbox(name, { image: "node:22-alpine" });
      },
      "session.deleted": async ({ session }) => {
        if (sandbox) await sandbox.remove();
      },
    },
    tools: {
      sandbox_run: {
        description: "Run a command in an isolated agentkernel sandbox",
        args: z.object({
          command: z.array(z.string()),
          profile: z.enum(["permissive", "moderate", "restrictive"]).optional(),
        }),
        async run({ command, profile }) {
          if (!sandbox) throw new Error("No active sandbox session");
          const result = await sandbox.exec(command);
          return result.output;
        },
      },
    },
  };
};
```

**Capabilities**:
- Creates a persistent sandbox per OpenCode session
- Routes tool calls through agentkernel sandboxes
- File sync between local project and sandbox (git-based, like opencode-daytona)
- Sandbox cleanup on session end

**User config** (`opencode.json`):
```json
{
  "$schema": "https://opencode.ai/config.json",
  "plugins": ["opencode-agentkernel"]
}
```

**Env**: Requires `agentkernel` binary installed. No remote API key needed (runs locally).

**Build**: TypeScript compiled to JS. Publish: `npm publish --access public`.

**Key difference from Daytona plugin**: agentkernel runs locally (no remote API key needed), so setup is simpler. No account required.

### 3B. Codex MCP Config (`plugins/codex/`)

Codex supports MCP servers. The existing `agentkernel mcp-server` command works directly. This is a docs/config package.

```
plugins/codex/
  README.md             # Setup instructions
  mcp.json              # MCP server config
```

**mcp.json**:
```json
{
  "mcpServers": {
    "agentkernel": {
      "command": "agentkernel",
      "args": ["mcp-server"],
      "env": {}
    }
  }
}
```

### 3C. Gemini CLI MCP Config (`plugins/gemini/`)

Same pattern as Codex.

```
plugins/gemini/
  README.md
  mcp.json
```

### 3D. Universal MCP Guide (`plugins/mcp/`)

Single guide covering all MCP-compatible agents.

```
plugins/mcp/
  README.md             # Setup for any MCP agent
  mcp.json              # Generic config
  settings.json         # Recommended permission settings
```

---

## Implementation Sequence

All three workstreams are independent. Prioritize by impact:

### Phase 1: Node.js SDK + OpenCode Plugin
- Node.js SDK is a dependency of the OpenCode plugin
- OpenCode is the newest agent with the most open plugin ecosystem

### Phase 2: Python SDK + Homebrew
- Python is the most common language for AI agent users
- Homebrew makes installation trivial on macOS

### Phase 3: Rust SDK + Agent Plugin Docs
- Rust SDK completes language coverage
- Agent plugin docs (Codex, Gemini MCP configs) are low effort

---

## Files to Create

### New directories in agentkernel repo
- `sdk/python/` - Python SDK (~10 files)
- `sdk/rust/` - Rust SDK (~10 files)
- `sdk/nodejs/` - Node.js SDK (~10 files)
- `plugins/opencode/` - OpenCode plugin (~8 files)
- `plugins/codex/` - Codex config (~2 files)
- `plugins/gemini/` - Gemini config (~2 files)
- `plugins/mcp/` - Universal MCP guide (~3 files)

### New repo
- `thrashr888/homebrew-agentkernel` - Homebrew tap (~2 files)

### Key reference files (read-only)
- `src/http_api.rs` - Ground truth for API types
- `docs/openapi.yaml` - OpenAPI 3.1 spec
- `src/permissions.rs` - SecurityProfile enum
- `src/mcp.rs` - MCP server (reference for plugin integration)
- `claude-plugin/skills/sandbox/SKILL.md` - Claude Code plugin (reference)
- `.github/workflows/release.yml` - Binary build targets (for Homebrew SHA)

---

## Verification

### SDK Tests
```bash
# Python
cd sdk/python && pip install -e ".[dev]" && pytest && mypy --strict src/

# Rust
cd sdk/rust && cargo fmt -- --check && cargo clippy -- -D warnings && cargo test

# Node.js
cd sdk/nodejs && npm install && npm test && npx tsc --noEmit
```

### Integration Tests (requires running server)
```bash
agentkernel serve --host 127.0.0.1 --port 18888 &

cd sdk/python && pytest -m integration
cd sdk/rust && cargo test -- --ignored
cd sdk/nodejs && AGENTKERNEL_URL=http://localhost:18888 npm run test:integration
```

### Homebrew
```bash
brew tap thrashr888/agentkernel && brew install agentkernel
agentkernel --version
```

### OpenCode Plugin
```bash
npm install -g opencode-agentkernel
# Add to opencode.json: { "plugins": ["opencode-agentkernel"] }
opencode  # Plugin loads at startup, sandbox tools available
```
