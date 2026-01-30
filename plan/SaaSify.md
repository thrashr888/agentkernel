# Minimum Viable SaaS Wrapper

You already have 95% of what you need:
- HTTP API (`http_api.rs`) - production-ready REST endpoints
- Daemon mode (`daemon/`) - pre-warmed VM pool for 4x faster execution

## Performance (Measured on Linux/KVM)

| Mode | Latency | Use Case |
|------|---------|----------|
| Ephemeral Firecracker | ~810ms | One-off executions |
| Daemon (warm pool) | ~195ms | API requests, interactive use |
| Container pool (Docker) | ~250ms | macOS/non-KVM environments |

The daemon maintains 3-5 pre-warmed Firecracker VMs, reducing cold start from ~800ms to ~200ms.

## What's Missing

### 1. Add Authentication Layer (30 minutes)

```rust
// Add to http_api.rs
- API key validation middleware
- Per-user resource limits
- Usage tracking
```

### 2. Add Multi-Tenancy (1 hour)

```rust
// Namespace sandboxes by user/account
- Prefix sandbox names with account ID
- Track quotas per account
- Isolate sandbox lists
```

### 3. Add Billing Hooks (30 minutes)

```rust
// Track execution time/resources
- Log executions to DB/file
- Export usage metrics endpoint
```

### 4. Integrate Daemon with HTTP API (1 hour)

```rust
// Modify http_api.rs to use daemon pool
- Check if daemon is available
- Route /run requests through daemon for ~200ms response
- Fall back to ephemeral if daemon unavailable
```

## Hosting Options

### Home Linux Box (Free - $500 one-time)

**Hardware Requirements:**
- Mini PC with KVM support (Intel NUC, Beelink SER7)
- 32GB RAM, 500GB SSD
- ~$300-500

**Stack:**

```bash
# Run daemon for warm VM pool
agentkernel daemon start &

# Run HTTP API (will auto-use daemon)
agentkernel serve --host 0.0.0.0 --port 18888

# Reverse proxy (Caddy for auto-HTTPS)
caddy reverse-proxy --from api.yourdomain.com --to localhost:18888

# Database (SQLite or PostgreSQL)
# For usage tracking, auth tokens
```

**Pros:**
- No monthly costs beyond electricity (~$5/mo)
- Full control
- Firecracker microVMs work perfectly (KVM native)
- ~200ms response times with daemon

**Cons:**
- Your internet goes down = service down
- You manage everything
- Limited bandwidth (home ISP)

**Best for:** Personal use, small team (1-10 users), testing

### Cloud Hosting ($20-200+/month)

**Option A: Bare Metal (Firecracker + Daemon)**

- **Hetzner Dedicated** - €40/mo (~$45)
  - AMD Ryzen 5 3600, 64GB RAM
  - Native KVM, fast network
  - Best price/performance
  - Run daemon for ~200ms latency

- **OVH Bare Metal** - $50-100/mo
  - Similar specs, US/EU locations

**Option B: VMs with Nested Virtualization**

- **DigitalOcean Droplet** - $48/mo
  - 8GB RAM, nested KVM enabled
  - Simple, but slower than bare metal

- **Linode Dedicated CPU** - $36/mo
  - KVM support, good for microVMs

**Option C: Containers Only (Docker backend)**

- **Fly.io** - $0.02/s compute + $0.15/GB RAM
  - ~$20-50/mo for low traffic
  - No KVM, uses Docker backend (~250ms with pool)
  - Excellent for testing

**Cloud Stack:**

```bash
# /etc/systemd/system/agentkernel-daemon.service
[Unit]
Description=Agentkernel Daemon (VM Pool)
After=network.target

[Service]
Type=simple
User=agentkernel
ExecStart=/usr/local/bin/agentkernel daemon start
Restart=on-failure

[Install]
WantedBy=multi-user.target

# /etc/systemd/system/agentkernel-api.service
[Unit]
Description=Agentkernel HTTP API
After=agentkernel-daemon.service
Requires=agentkernel-daemon.service

[Service]
Type=simple
User=agentkernel
ExecStart=/usr/local/bin/agentkernel serve --host 0.0.0.0 --port 18888
Restart=on-failure

[Install]
WantedBy=multi-user.target

# Caddy/nginx reverse proxy
# PostgreSQL for users/billing
# Redis for rate limiting (optional)
```

## Pricing Models

**Tier 1: Free (Personal)**
- 100 executions/month
- 512MB RAM per sandbox
- Network disabled
- API access only

**Tier 2: Developer ($10/mo)**
- 1,000 executions/month
- 1GB RAM per sandbox
- Network enabled
- Priority queue (daemon pool)

**Tier 3: Team ($50/mo)**
- 10,000 executions/month
- 2GB RAM per sandbox
- Persistent sandboxes (24h)
- SLA

**Tier 4: Enterprise ($200+/mo)**
- Unlimited executions
- Custom resource limits
- Private deployment option
- White-label

**Pay-as-you-go Alternative:**
- $0.01 per execution
- $0.001 per GB-second of RAM
- Similar to AWS Lambda pricing

## Implementation Roadmap

### Week 1: MVP Wrapper

- [x] Daemon mode for VM pooling (done!)
- [ ] Integrate daemon with HTTP API `/run` endpoint
- [ ] Add API key auth to http_api.rs
- [ ] Add user namespace to sandbox names
- [ ] SQLite DB for users, API keys, usage
- [ ] Usage tracking middleware

### Week 2: Deployment

- [ ] Caddy reverse proxy config
- [ ] systemd service files (daemon + API)
- [ ] PostgreSQL migration (from SQLite)
- [ ] Monitoring (simple health checks)
- [ ] Daemon health endpoint for load balancers

### Week 3: Billing

- [ ] Stripe integration
- [ ] Usage limits enforcement
- [ ] Email notifications (quota warnings)
- [ ] Basic admin dashboard

## Quick Start: Home Box Setup

```bash
# 1. Install on your Linux box
cargo build --release
sudo cp target/release/agentkernel /usr/local/bin/

# 2. Create systemd services
sudo tee /etc/systemd/system/agentkernel-daemon.service <<EOF
[Unit]
Description=Agentkernel Daemon (VM Pool)
After=network.target

[Service]
Type=simple
User=agentkernel
ExecStart=/usr/local/bin/agentkernel daemon start
Restart=on-failure

[Install]
WantedBy=multi-user.target
EOF

sudo tee /etc/systemd/system/agentkernel-api.service <<EOF
[Unit]
Description=Agentkernel HTTP API
After=agentkernel-daemon.service

[Service]
Type=simple
User=agentkernel
ExecStart=/usr/local/bin/agentkernel serve --host 0.0.0.0 --port 18888
Restart=on-failure

[Install]
WantedBy=multi-user.target
EOF

# 3. Start services
sudo systemctl enable --now agentkernel-daemon
sudo systemctl enable --now agentkernel-api

# 4. Verify daemon is running
agentkernel daemon status
# Should show: Warm VMs: 3, In use: 0

# 5. Install Caddy (auto-HTTPS)
sudo caddy reverse-proxy --from api.agentkernel.com --to localhost:18888
```

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                     Internet                             │
└─────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────┐
│                  Caddy (HTTPS + Auth)                    │
└─────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────┐
│              agentkernel serve (HTTP API)                │
│                                                          │
│  /run  ─────► Daemon Client ─────► Unix Socket          │
│  /create                              │                  │
│  /exec                                ▼                  │
│  /status          ┌─────────────────────────────────┐   │
│                   │     agentkernel daemon          │   │
│                   │                                  │   │
│                   │  ┌────┐ ┌────┐ ┌────┐ ┌────┐   │   │
│                   │  │ VM │ │ VM │ │ VM │ │ VM │   │   │
│                   │  │warm│ │warm│ │warm│ │use │   │   │
│                   │  └────┘ └────┘ └────┘ └────┘   │   │
│                   │       Firecracker Pool          │   │
│                   └─────────────────────────────────┘   │
└─────────────────────────────────────────────────────────┘
```

## Recommendation

Start with home Linux box for these reasons:

1. You already have the HTTP API + daemon
2. Firecracker works perfectly on Linux
3. ~200ms response times with daemon pool
4. Zero monthly costs while testing pricing
5. Easy to migrate to cloud later (same systemd setup)

**Migrate to Hetzner bare metal when:**
- You have paying customers
- Home bandwidth becomes limiting
- You want 99.9% uptime SLA

**The only code you need to write:**
- Daemon/HTTP integration (~50 lines)
- API key middleware (~50 lines)
- Usage tracking (~100 lines)
- User namespace (~20 lines)

**Total:** ~2-3 hours of work for a functional SaaS with ~200ms response times.
