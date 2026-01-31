job "agentkernel" {
  datacenters = ["dc1"]
  type        = "service"

  group "api" {
    count = 1

    network {
      port "http" {
        static = 18888
      }
    }

    service {
      name = "agentkernel"
      port = "http"

      check {
        type     = "http"
        path     = "/health"
        interval = "10s"
        timeout  = "2s"
      }
    }

    task "server" {
      driver = "docker"

      config {
        image = "ghcr.io/thrashr888/agentkernel:latest"
        args  = ["serve", "--host", "0.0.0.0", "--port", "18888", "--backend", "nomad"]
        ports = ["http"]
      }

      env {
        NOMAD_ADDR = "http://${attr.unique.network.ip-address}:4646"
        # NOMAD_TOKEN = "" # Set via Vault or Nomad Variables for production
      }

      resources {
        cpu    = 500
        memory = 256
      }
    }
  }
}
