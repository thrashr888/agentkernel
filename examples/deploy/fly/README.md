# Fly.io Deployment

One-click deployment to Fly.io with persistent storage.

## Quick Start

```bash
# Install Fly CLI
curl -L https://fly.io/install.sh | sh

# Login
fly auth login

# Deploy (first time)
fly launch --copy-config

# Deploy updates
fly deploy
```

## Configuration

### Regions

Change `primary_region` in `fly.toml` to your nearest region:
- `ord` - Chicago
- `iad` - Virginia
- `lax` - Los Angeles
- `ams` - Amsterdam
- `lhr` - London
- `nrt` - Tokyo

Full list: https://fly.io/docs/reference/regions/

### Secrets

```bash
# Set API key for authentication
fly secrets set AGENTKERNEL_API_KEY=your-secret-key
```

### Scaling

```bash
# Scale to multiple machines
fly scale count 3

# Scale memory
fly scale memory 2048
```

### Persistent Storage

The `agentkernel_data` volume persists sandbox state:

```bash
# Create volume (done automatically on first deploy)
fly volumes create agentkernel_data --size 10

# List volumes
fly volumes list
```

## Firecracker Support

For full Firecracker microVM support, you need dedicated CPU instances with KVM:

```toml
[[vm]]
size = "dedicated-cpu-2x"
memory = "4gb"
```

Note: Dedicated CPU machines are more expensive but provide true hardware isolation.

## Monitoring

```bash
# View logs
fly logs

# SSH into machine
fly ssh console

# Dashboard
fly dashboard
```

## Costs

- Shared CPU: ~$5-10/month
- Dedicated CPU: ~$30-60/month
- Storage: $0.15/GB/month
