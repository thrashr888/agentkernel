# Hetzner Cloud Deployment

Terraform example for a Docker-backed AgentKernel service on a Hetzner Cloud host. Firecracker requires separately verified Linux KVM access.

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

Choose a currently available server type for your workload and region. Check
[Hetzner Cloud](https://www.hetzner.com/cloud/) for current resources and pricing.
Dedicated vCPUs alone do not establish nested-virtualization support.

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

Do not assume a Cloud instance exposes nested virtualization. Confirm provider support and usable KVM access before selecting Firecracker:

```bash
# SSH into server
ssh root@$(terraform output -raw server_ip)

# Verify KVM
ls -la /dev/kvm

# Firecracker must also be installed and usable; otherwise select Docker explicitly
```

## Persistent Storage

A 20GB volume is created by default for sandbox data:

```hcl
create_volume = true
volume_size   = 50  # Increase if needed
```

## Costs

Confirm current compute, volume, and network charges with the provider before
applying the configuration.

## Destroy

```bash
terraform destroy
```
