variable "job_name" {
  description = "The name of the Nomad job."
  type        = string
  default     = "agentkernel"
}

variable "datacenters" {
  description = "A list of datacenters in the region which are eligible for task placement."
  type        = list(string)
  default     = ["dc1"]
}

variable "region" {
  description = "The region where the job should be placed."
  type        = string
  default     = ""
}

variable "image" {
  description = "The container image for agentkernel."
  type        = string
  default     = "ghcr.io/thrashr888/agentkernel:latest"
}

variable "image_tag" {
  description = "The image tag to deploy. Overrides the tag in the image variable."
  type        = string
  default     = ""
}

variable "count" {
  description = "Number of agentkernel instances to run."
  type        = number
  default     = 1
}

variable "http_port" {
  description = "The port agentkernel listens on."
  type        = number
  default     = 18888
}

variable "backend" {
  description = "Sandbox backend: nomad, docker, kubernetes."
  type        = string
  default     = "nomad"
}

variable "nomad_addr" {
  description = "Nomad API address for the sandbox backend."
  type        = string
  default     = ""
}

variable "nomad_token" {
  description = "Nomad ACL token. Prefer Vault integration for production."
  type        = string
  default     = ""
}

variable "resources" {
  description = "CPU and memory resources for the agentkernel task."
  type = object({
    cpu    = number
    memory = number
  })
  default = {
    cpu    = 500
    memory = 256
  }
}

variable "register_consul_service" {
  description = "Register agentkernel as a Consul service."
  type        = bool
  default     = true
}
