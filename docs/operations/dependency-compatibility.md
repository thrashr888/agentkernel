# Dependency compatibility gates

AgentKernel validates optional Rust feature sets at the package's Rust 1.89
minimum supported version. The pull-request matrix covers no-default,
Kubernetes-only, Nomad-only, enterprise-only, and the combined orchestrator
features. The scheduled Nomad matrix runs lifecycle and exec smoke tests with
the default Docker task driver against Nomad 1.10.5 (the final 1.10 LTS
community release) and Nomad 2.0.4.

## Hyperlight arm64 exception

Hyperlight remains fully required on Linux x86_64. Its check, strict Clippy, and
library tests run on every relevant pull request.

Native Linux arm64 is temporarily a visible, non-blocking probe through
**2026-11-30**. The current published `hyperlight-wasm` release is 0.14.0 and
requires `hyperlight-host ^0.14.0`. Initial KVM/aarch64 host support landed after
that line and is available in `hyperlight-host` 0.16.0, but no corresponding
`hyperlight-wasm` release has been published. Depending on an unpublished moving
Git revision would weaken lockfile reproducibility and dependency auditing, so
AgentKernel stays on the latest published Rust 1.89-compatible Wasm adapter.

The exception must be removed or renewed before its expiry. Remove it as soon as
upstream publishes a Rust 1.89-compatible `hyperlight-wasm` that consumes the
arm64-capable host line, then make the native arm64 check blocking and add its
Clippy and library-test gates.

Primary upstream references:

- [Published hyperlight-wasm releases](https://crates.io/crates/hyperlight-wasm/versions)
- [Initial Hyperlight KVM/aarch64 support](https://github.com/hyperlight-dev/hyperlight/pull/1474)
- [Open Hyperlight aarch64 support tracker](https://github.com/hyperlight-dev/hyperlight/issues/677)
- [Nomad 1.10 release notes](https://developer.hashicorp.com/nomad/docs/release-notes/v1-10-x)
- [Nomad 2.0 release notes](https://developer.hashicorp.com/nomad/docs/release-notes/v2-0-x)
