# Agentkernel

A Firecracker-inspired microkernel for running AI coding agents in isolated sandboxes.

## Vision

Modern AI coding agents (Claude Code, Gemini CLI, Codex, OpenCode) need secure, isolated execution environments. Agentkernel provides:

- **Minimal attack surface** - Firecracker-inspired architecture with only essential capabilities
- **Fast startup** - Sub-second container launches using pre-built images
- **Per-project isolation** - Each project gets its own sandboxed environment
- **Multi-agent support** - Run any major AI coding agent with consistent isolation

## Architecture

```
┌─────────────────────────────────────────────┐
│              Agentkernel CLI                │
├─────────────────────────────────────────────┤
│            Sandbox Manager                  │
├──────────┬──────────┬──────────┬───────────┤
│  Claude  │  Gemini  │  Codex   │ OpenCode  │
│  Code    │  CLI     │          │           │
├──────────┴──────────┴──────────┴───────────┤
│           Docker Container                  │
│  ┌─────────────────────────────────────┐   │
│  │    Isolated Filesystem + Network     │   │
│  │    Per-project config + deps         │   │
│  └─────────────────────────────────────┘   │
└─────────────────────────────────────────────┘
```

## Features (Planned)

- [ ] Firecracker-inspired microkernel design
- [ ] Docker-based container installation
- [ ] Sandbox isolation layer
- [ ] Multi-agent adapters (Claude Code, Gemini CLI, Codex, OpenCode)
- [ ] Local CLI interface
- [ ] Standalone executable
- [ ] Per-project configuration
- [ ] Skills/plugins for IDE integration

## Inspiration

Inspired by:
- [Firecracker](https://firecracker-microvm.github.io/) - Secure microVM technology
- [Ramp's Background Agent](https://builders.ramp.com/post/why-we-built-our-background-agent) - Cloud sandbox architecture for AI agents
- [Modal](https://modal.com/) - Serverless sandbox infrastructure

## Technology

Built with Rust for performance, safety, and minimal dependencies.

## License

MIT
