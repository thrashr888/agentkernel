job "[[ .agentkernel.job_name ]]" {
  [[ if .agentkernel.region -]]
  region = "[[ .agentkernel.region ]]"
  [[- end ]]
  datacenters = [[ .agentkernel.datacenters | toStringList ]]
  type        = "service"

  group "api" {
    count = [[ .agentkernel.count ]]

    network {
      port "http" {
        static = [[ .agentkernel.http_port ]]
      }
    }

    [[ if .agentkernel.register_consul_service -]]
    service {
      name = "[[ .agentkernel.job_name ]]"
      port = "http"

      check {
        type     = "http"
        path     = "/health"
        interval = "10s"
        timeout  = "2s"
      }
    }
    [[- end ]]

    task "server" {
      driver = "docker"

      config {
        image = "[[ .agentkernel.image ]][[ if .agentkernel.image_tag ]]:[[ .agentkernel.image_tag ]][[ end ]]"
        args  = ["serve", "--host", "0.0.0.0", "--port", "[[ .agentkernel.http_port ]]", "--backend", "[[ .agentkernel.backend ]]"]
        ports = ["http"]
      }

      env {
        [[ if .agentkernel.nomad_addr -]]
        NOMAD_ADDR = "[[ .agentkernel.nomad_addr ]]"
        [[- end ]]
        [[ if .agentkernel.nomad_token -]]
        NOMAD_TOKEN = "[[ .agentkernel.nomad_token ]]"
        [[- end ]]
      }

      resources {
        cpu    = [[ .agentkernel.resources.cpu ]]
        memory = [[ .agentkernel.resources.memory ]]
      }
    }
  }
}
