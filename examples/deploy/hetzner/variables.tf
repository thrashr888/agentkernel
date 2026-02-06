# Hetzner Cloud variables

variable "hcloud_token" {
  description = "Hetzner Cloud API token"
  type        = string
  sensitive   = true
}

variable "ssh_public_key" {
  description = "SSH public key for server access"
  type        = string
}

variable "server_name" {
  description = "Name of the server"
  type        = string
  default     = "agentkernel"
}

variable "server_type" {
  description = "Hetzner server type (cpx11, cpx21, cpx31, cpx41, cpx51)"
  type        = string
  default     = "cpx21"  # 3 vCPU, 4GB RAM, €8.98/mo
}

variable "location" {
  description = "Hetzner datacenter location"
  type        = string
  default     = "nbg1"  # Nuremberg
  # Options: nbg1, fsn1, hel1, ash, hil
}

variable "api_key" {
  description = "agentkernel API key for authentication"
  type        = string
  default     = ""
  sensitive   = true
}

variable "allowed_ips" {
  description = "IP addresses allowed to access the API"
  type        = list(string)
  default     = ["0.0.0.0/0", "::/0"]  # Open to all - restrict in production
}

variable "create_volume" {
  description = "Create a persistent volume for data"
  type        = bool
  default     = true
}

variable "volume_size" {
  description = "Size of the persistent volume in GB"
  type        = number
  default     = 20
}
