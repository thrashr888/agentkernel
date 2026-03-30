# Remote Runloop Example

```bash
npm install --prefix scripts
export RUNLOOP_API_KEY=...

agentkernel sandbox create remote-runloop --backend runloop -c examples/remote-runloop/agentkernel.toml
agentkernel exec remote-runloop -- sh -lc 'pwd && ls -la /workspace'
agentkernel snapshot take remote-runloop --name remote-runloop-snap
```

The bundled Runloop adapter supports lifecycle, `/workspace` sync, file APIs, attach, tunnel-backed endpoints, and workspace-level snapshot restore.
