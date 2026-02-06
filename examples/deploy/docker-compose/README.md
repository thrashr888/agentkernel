# Docker Compose Deployment

Local multi-container setup for development and small teams.

## Quick Start

```bash
# Start agentkernel
docker-compose up -d

# Check health
curl http://localhost:18888/health

# View logs
docker-compose logs -f
```

## With HTTPS Proxy

```bash
# Edit Caddyfile with your domain
vim Caddyfile

# Start with Caddy reverse proxy
docker-compose --profile with-proxy up -d
```

## Configuration

Environment variables in `docker-compose.yml`:

| Variable | Description |
|----------|-------------|
| `AGENTKERNEL_API_KEY` | Enable API key authentication |
| `AGENTKERNEL_TLS_CERT` | Path to TLS certificate |
| `AGENTKERNEL_TLS_KEY` | Path to TLS private key |

## Volumes

- `agentkernel_data` - Sandbox state and metadata
- Docker socket mounted for container backend

## Production Notes

- Set `AGENTKERNEL_API_KEY` for authentication
- Use the Caddy proxy for automatic HTTPS
- Consider resource limits for the container
