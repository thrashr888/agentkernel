# Remote Modal Example

```bash
npm install --prefix scripts
export MODAL_TOKEN_ID=...
export MODAL_TOKEN_SECRET=...

agentkernel sandbox create remote-modal --backend modal -c examples/remote-modal/agentkernel.toml
agentkernel exec remote-modal -- sh -lc 'pwd && ls -la /workspace'
agentkernel snapshot take remote-modal --name remote-modal-snap
```

The bundled Modal adapter supports lifecycle, `/workspace` sync, file APIs, interactive attach, tunnel-backed endpoints, and workspace snapshot restore.
