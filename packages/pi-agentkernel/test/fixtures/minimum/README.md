# Reproducible minimum Pi host

This private fixture keeps Pi 1.0.1 and its compatible codemode 1.0.1 together.
A fresh unpinned 1.0.1 installation now resolves codemode 1.2.0, whose removed
exports break the old host's unbundled entry point. The exact-host root override
and committed lockfile reproduce a working graph without raising the extension's
security floor, bypassing imports, or suppressing tests. The separate latest-host
CI lane intentionally continues fresh resolution.

From packages/pi-agentkernel:

```sh
npm ci --prefix test/fixtures/minimum --ignore-scripts --no-audit --no-fund
PI_HOST_FIXTURE=test/fixtures/minimum npm test
```

All four existing host, security, lifecycle and installer-consistency tests run
against this host. These files are development fixtures, excluded from the
published extension package. See the package README for the root-consumer recipe
and upstream incompatibility; a CI fixture does not repair existing host installs.
When updating this lockfile, retain the minimum host and scoped codemode constraint,
review the resolved graph, run the tests and audit it. No package is renamed and
no advisory is suppressed.
