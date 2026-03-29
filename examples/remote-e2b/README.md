# Remote E2B Example

```bash
npm install --prefix scripts
export E2B_API_KEY=...

agentkernel sandbox create remote-e2b --backend e2b -c examples/remote-e2b/agentkernel.toml
agentkernel exec remote-e2b -- sh -lc 'python3 --version || node -v'
agentkernel snapshot take remote-e2b --name remote-e2b-snap
```

The bundled E2B adapter supports lifecycle, `/workspace` sync, file APIs, PTY attach, port endpoints, and snapshot restore.
