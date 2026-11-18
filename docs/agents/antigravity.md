# Antigravity CLI

Google has transitioned its terminal offering to Antigravity CLI (`agy`). The
`antigravity-sandbox` template installs the native **1.5.10** Linux build using
architecture-specific release URLs and SHA-512 verification from Google's
updater manifests (checked 2026-11-18). It uses Debian/glibc, not the old Gemini
npm package.

```bash
agentkernel create google-worker --template antigravity-sandbox
agentkernel exec google-worker -- agy --version
```

This is an **experimental executable template**, not yet a native AgentKernel
AgentType or plugin. Run `agy` explicitly. Installation and inference have not
yet been smoke-tested in every backend. The client can self-update, so pinning
the initial archive alone does not establish immutable execution.

Google documents `GEMINI_API_KEY` for headless API-key use. Explicitly supply it
through your approved secret delivery mechanism; never copy a host keyring,
OAuth profile or home directory into an untrusted sandbox. Keep model access
separate from service credentials. Domain configuration alone is not proof
that a backend enforces egress isolation.

Existing `gemini-sandbox` remains for compatibility; no credentials or settings
are migrated implicitly.

Sources: [Quickstart](https://antigravity.google/docs/getting-started),
[authentication](https://antigravity.google/docs/install/),
[transition announcement](https://github.com/google-gemini/gemini-cli/discussions/27274).
