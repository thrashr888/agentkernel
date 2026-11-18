# Agent template release audit — 2026-11-18

This audit updates executable pins in `templates/`, not the independently
smoke-tested image manifest. A registry lookup is not a successful image build
or inference test. `examples/agents/tested-versions.json` retains its historical
verification date; it must not be relabeled as freshly tested.

| Template | Selected release | Source |
|---|---|---|
| Claude Code | 2.1.327 | npm `@anthropic-ai/claude-code` latest |
| Codex | 0.170.0 | npm `@openai/codex` latest |
| Pi | 1.5.2 | npm `@earendil-works/pi-coding-agent` latest |
| Copilot | 1.0.114 | npm `@github/copilot` latest |
| Amp | 0.0.1794974444-g035056 | npm `@ampcode/cli` latest |
| Gemini CLI (legacy) | 0.68.0 | npm `@google/gemini-cli` latest |
| OpenCode (compatibility) | 1.19.6 | npm `opencode-ai` latest |
| OpenClaw | 2026.11.5 (pnpm 12.21.0, Node 24) | npm `openclaw` / `pnpm` latest; OpenClaw requires Node >=24.16.0 |
| Hermes | v0.23.2 | [official release](https://github.com/NousResearch/hermes-agent/releases/tag/v0.23.2) |
| Antigravity (experimental) | 1.5.10 | [official installer](https://antigravity.google/cli/install.sh) and its Linux updater manifests |

OpenCode's [current README](https://github.com/anomalyco/opencode) installs
`@opencode/cli` (2.3.2 at audit time), not `opencode-ai`. The current AgentKernel
proxy and agent adapter have not been validated against v2. The template retains
the latest 1.x package rather than silently changing protocol behavior.

Symphony is removed from built-in discovery, with its explicit template and
historical docs retained for compatibility. This is a product support decision,
not a claim that upstream is abandoned.

## Recommended expansion

The next candidates are [Cursor CLI](https://cursor.com/cli),
[Factory Droid](https://docs.factory.ai/),
[Goose](https://github.com/aaif-goose/goose), and
[Qwen Code](https://github.com/QwenLM/qwen-code). Each offers a terminal/headless
workflow, making it a better worker candidate than an IDE-only integration.
They are candidates, not advertised as validated AgentKernel integrations.

## Verification boundary

The initial Antigravity ARM64 archive's checksum and `antigravity` member were
verified. Both architecture URLs/hashes are copied from Google's official updater
manifests. No model credentials were used. No Antigravity inference claim is made.
Native agent images, language/service base image tags, and transitive package
versions need their own compatibility/build verification; a major OS/runtime
upgrade should not be bundled into an agent lineup change without that evidence.

Migration: `--template symphony-sandbox` no longer resolves as a built-in name.
Existing users must explicitly reference `--config templates/symphony-sandbox.toml`
from a checkout, or save a local custom template before upgrading.
