# Hetzner Cloud deployment for agentkernel
# Bare metal-like performance at low cost
#
# Usage:
#   export HCLOUD_TOKEN="your-token"
#   terraform init
#   terraform apply

terraform {
  required_providers {
    hcloud = {
      source  = "hetznercloud/hcloud"
      version = "~> 1.45"
    }
  }
}

provider "hcloud" {
  token = var.hcloud_token
}

# SSH key for access
resource "hcloud_ssh_key" "default" {
  name       = "agentkernel-key"
  public_key = var.ssh_public_key
}

# Firewall
resource "hcloud_firewall" "agentkernel" {
  name = "agentkernel-fw"

  rule {
    direction = "in"
    protocol  = "tcp"
    port      = "22"
    source_ips = ["0.0.0.0/0", "::/0"]
  }

  rule {
    direction = "in"
    protocol  = "tcp"
    port      = "18888"
    source_ips = var.allowed_ips
  }

  rule {
    direction = "in"
    protocol  = "tcp"
    port      = "443"
    source_ips = ["0.0.0.0/0", "::/0"]
  }
}

# Server
resource "hcloud_server" "agentkernel" {
  name        = var.server_name
  image       = "ubuntu-24.04"
  server_type = var.server_type
  location    = var.location
  ssh_keys    = [hcloud_ssh_key.default.id]
  firewall_ids = [hcloud_firewall.agentkernel.id]

  user_data = <<-EOF
    #!/bin/bash
    set -e

    # Install Docker
    curl -fsSL https://get.docker.com | sh
    systemctl enable docker
    systemctl start docker

    # Install agentkernel
    curl -fsSL https://raw.githubusercontent.com/thrashr888/agentkernel/main/install.sh | sh

    # Create systemd service
    cat > /etc/systemd/system/agentkernel.service <<'SERVICE'
    [Unit]
    Description=agentkernel sandbox runtime
    After=network.target docker.service
    Requires=docker.service

    [Service]
    Type=simple
    ExecStart=/usr/local/bin/agentkernel serve --host 0.0.0.0 --port 18888
    Restart=always
    RestartSec=5
    Environment=AGENTKERNEL_API_KEY=${var.api_key}

    [Install]
    WantedBy=multi-user.target
    SERVICE

    systemctl daemon-reload
    systemctl enable agentkernel
    systemctl start agentkernel
  EOF

  labels = {
    app = "agentkernel"
  }
}

# Optional: Persistent volume
resource "hcloud_volume" "data" {
  count    = var.create_volume ? 1 : 0
  name     = "agentkernel-data"
  size     = var.volume_size
  location = var.location
  format   = "ext4"
}

resource "hcloud_volume_attachment" "data" {
  count     = var.create_volume ? 1 : 0
  volume_id = hcloud_volume.data[0].id
  server_id = hcloud_server.agentkernel.id
  automount = true
}

# Outputs
output "server_ip" {
  value = hcloud_server.agentkernel.ipv4_address
}

output "api_url" {
  value = "http://${hcloud_server.agentkernel.ipv4_address}:18888"
}

output "ssh_command" {
  value = "ssh root@${hcloud_server.agentkernel.ipv4_address}"
}
