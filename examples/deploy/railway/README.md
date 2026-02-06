# Railway Deployment

Simple one-click deployment to Railway.

## Quick Start

### Option 1: Deploy Button

[![Deploy on Railway](https://railway.com/button.svg)](https://railway.com/deploy/v6tIeu?referralCode=gieWq1)

### Option 2: CLI

```bash
# Install Railway CLI
npm install -g @railway/cli

# Login
railway login

# Initialize project
railway init

# Deploy
railway up
```

### Option 3: GitHub Integration

1. Fork the agentkernel repository
2. Connect Railway to your GitHub
3. Select the `examples/deploy/railway` directory
4. Deploy

## Configuration

### Environment Variables

Set in Railway dashboard or via CLI:

```bash
railway variables set AGENTKERNEL_API_KEY=your-secret-key
```

| Variable | Description |
|----------|-------------|
| `PORT` | Set automatically by Railway |
| `AGENTKERNEL_API_KEY` | API authentication key |

### Persistent Storage

Railway provides ephemeral storage by default. For persistent data:

1. Add a Volume in Railway dashboard
2. Mount at `/data`

## Limitations

- Railway runs containers, not VMs
- No Firecracker support (Docker backend only)
- Ephemeral storage unless volume added

## Costs

- Hobby: $5/month (500 hours)
- Pro: $20/month (unlimited)

See [Railway pricing](https://railway.app/pricing) for details.

## Monitoring

```bash
# View logs
railway logs

# Open dashboard
railway open
```
