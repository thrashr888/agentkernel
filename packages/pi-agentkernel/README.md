# pi-agentkernel

Route [Pi coding agent](https://github.com/earendil-works/pi) commands through [agentkernel](https://github.com/thrashr888/agentkernel) microVM sandboxes.

## Install

```bash
# 1. Install and start agentkernel
brew tap thrashr888/tap && brew install agentkernel
brew services start thrashr888/agentkernel/agentkernel

# 2. Install the extension
pi install npm:pi-agentkernel
```

## What it does

This extension overrides Pi's built-in `bash` tool so every shell command runs in an isolated sandbox instead of on your machine.

- **Session sandbox** — A persistent sandbox is created when Pi starts and removed when it ends
- **State persists** — Installed packages and files carry over between commands
- **Fallback** — If agentkernel isn't running, commands run locally

## Tools

| Tool | Description |
|------|-------------|
| `bash` | Built-in tool override — routes all shell commands through the sandbox |
| `sandbox_run` | One-shot command in a fresh sandbox (clean environment each time) |

## Commands

| Command | Description |
|---------|-------------|
| `/sandbox` | Show current sandbox status |

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `AGENTKERNEL_BASE_URL` | `http://localhost:18888` | agentkernel API endpoint |
| `AGENTKERNEL_API_KEY` | - | Optional Bearer token for API auth |

## Requirements

- [agentkernel](https://github.com/thrashr888/agentkernel) running locally or remotely
- Pi >=1.0.1 <2 (Node.js >=22.19.0 for npm installations)

## Host security and upgrades

Support now starts at Pi 1.0.1; Pi 0.x and 1.0.0 are no longer supported.
[Pi 1.0.1 fixes the vulnerable `brace-expansion` dependency](https://github.com/earendil-works/pi/releases/tag/v1.0.1).
Updating this extension does not update an already-installed Pi host: Pi's package
manager suppresses automatic host-peer installation.

Upgrade Pi with the installer or package manager you originally used. For a global
npm installation:

```bash
npm install -g --ignore-scripts '@earendil-works/pi-coding-agent@^1.0.1'
pi --version
```

Verify the reported host version is at least 1.0.1 and below 2 before loading this
extension. Managed installations can use `pi update` instead.

## Known upstream 1.0.1 dependency incompatibility

A fresh Pi 1.0.1 npm installation can select `pi-codemode@1.2.0`, which removed
exports still imported by the 1.0.1 **unbundled library entry point**. This fails
before extension code loads; it is not an AgentKernel API incompatibility. The
published bundled `pi --version` entry point still works in that graph; this is
not a claim that every bundled CLI operation has been exercised.

Normal installations should use the current supported 1.x host (the upgrade
command above), rather than pinning a new installation to 1.0.1. The minimum
supported host remains 1.0.1 with a coherent dependency graph.

If an application must embed exactly Pi 1.0.1, add this scoped override to the
**consuming application's root** package.json, alongside its exact 1.0.1 host
dependency. Merge it with any existing overrides; then generate/review and commit
the application's lockfile and use `npm ci` for subsequent installations:

```json
{
  "overrides": {
    "@earendil-works/pi-coding-agent@1.0.1": {
      "@earendil-works/pi-codemode": "1.0.1"
    }
  }
}
```

Overrides in a published extension dependency do not propagate to consumers.
This override targets only the affected old host, not current 1.x hosts. Our
[locked minimum fixture](https://github.com/thrashr888/agentkernel/tree/main/packages/pi-agentkernel/test/fixtures/minimum)
provides a reproducible example. Keep the existing security checks when updating
its lockfile. Upstream source change:
[24382bed](https://github.com/earendil-works/pi/commit/24382bedd03c61c0994628907a017b4491f8fa76).

## Development

Install a supported host, then run the offline contract tests:

```bash
npm install --ignore-scripts --no-audit
npm test
```

CI checks a locked, coherent minimum host (1.0.1) and a freshly resolved current
supported 1.x host. To reproduce the minimum lane from this directory:

```bash
npm ci --prefix test/fixtures/minimum --ignore-scripts --no-audit --no-fund
PI_HOST_FIXTURE=test/fixtures/minimum npm test
```

The
tests use Pi's public extension loader with an isolated configuration directory and
mock the AgentKernel API; no credentials, live sandboxes, or registry audits are
needed during test execution.

## License

Apache-2.0
