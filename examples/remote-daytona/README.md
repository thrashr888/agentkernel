# Remote Daytona Example

```bash
npm install --prefix scripts
export DAYTONA_API_KEY=...
# Optional: export DAYTONA_ORGANIZATION_ID=... (overrides organization in agentkernel.toml)

agentkernel sandbox create remote-daytona --backend daytona -c examples/remote-daytona/agentkernel.toml
agentkernel exec remote-daytona -- sh -lc 'node -v'
agentkernel snapshot take remote-daytona --name remote-daytona-snap
```

Edit `organization` in `agentkernel.toml` before first use, or set `DAYTONA_ORGANIZATION_ID`.
