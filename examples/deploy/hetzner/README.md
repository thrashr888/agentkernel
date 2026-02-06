# Hetzner Cloud Deployment

Bare metal-like performance at low cost. Ideal for production Firecracker deployments.

## Quick Start

```bash
# Set credentials
export HCLOUD_TOKEN="your-hetzner-api-token"
export TF_VAR_ssh_public_key="$(cat ~/.ssh/id_rsa.pub)"
export TF_VAR_api_key="your-agentkernel-api-key"

# Deploy
terraform init
terraform apply
```

## Server Types

| Type | vCPU | RAM | Price/mo | Notes |
|------|------|-----|----------|-------|
| `cpx11` | 2 | 2GB | €4.49 | Development |
| `cpx21` | 3 | 4GB | €8.98 | Small production |
| `cpx31` | 4 | 8GB | €16.99 | Medium production |
| `cpx41` | 8 | 16GB | €32.99 | Large production |
| `cpx51` | 16 | 32GB | €65.99 | High performance |

AMD EPYC CPUs with dedicated vCPUs - excellent for Firecracker.

## Locations

| ID | Location |
|----|----------|
| `nbg1` | Nuremberg, Germany |
| `fsn1` | Falkenstein, Germany |
| `hel1` | Helsinki, Finland |
| `ash` | Ashburn, USA |
| `hil` | Hillsboro, USA |

## Configuration

Create `terraform.tfvars`:

```hcl
hcloud_token    = "your-token"
ssh_public_key  = "ssh-rsa AAAA..."
server_type     = "cpx31"
location        = "ash"
api_key         = "your-api-key"
allowed_ips     = ["1.2.3.4/32"]  # Restrict to your IP
```

## Firecracker Support

Hetzner dedicated vCPU instances support nested virtualization:

```bash
# SSH into server
ssh root@$(terraform output -raw server_ip)

# Verify KVM
ls -la /dev/kvm

# agentkernel will auto-detect and use Firecracker
```

## Persistent Storage

A 20GB volume is created by default for sandbox data:

```hcl
create_volume = true
volume_size   = 50  # Increase if needed
```

## Costs

- Server: €4.49 - €65.99/month
- Volume: €0.052/GB/month
- Traffic: 20TB included, then €1/TB

## Destroy

```bash
terraform destroy
```
