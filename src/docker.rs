//! Docker container management using bollard.

use anyhow::{Context, Result};
use bollard::Docker;
use bollard::container::{
    Config as ContainerConfig, CreateContainerOptions, ListContainersOptions,
    RemoveContainerOptions, StartContainerOptions, StopContainerOptions,
};
use bollard::image::CreateImageOptions;
use bollard::models::{HostConfig, Mount, MountTypeEnum, PortBinding};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::path::Path;
use tracing::{debug, info};

use crate::config::Config;

const LABEL_MANAGED_BY: &str = "agentkernel.managed";
const LABEL_SANDBOX_NAME: &str = "agentkernel.sandbox";
const LABEL_AGENT_TYPE: &str = "agentkernel.agent";

/// Docker client wrapper for managing sandbox containers.
pub struct DockerManager {
    client: Docker,
}

/// Information about a sandbox container.
#[derive(Debug)]
pub struct SandboxInfo {
    pub name: String,
    pub container_id: String,
    pub agent: String,
    pub status: String,
    pub image: String,
}

impl DockerManager {
    /// Create a new Docker manager, connecting to the local Docker daemon.
    pub async fn new() -> Result<Self> {
        let client = Docker::connect_with_local_defaults()
            .context("Failed to connect to Docker daemon. Is Docker running?")?;

        // Verify connection by pinging the daemon
        client
            .ping()
            .await
            .context("Failed to ping Docker daemon")?;

        debug!("Connected to Docker daemon");
        Ok(Self { client })
    }

    /// Pull a Docker image if not already present.
    pub async fn ensure_image(&self, image: &str) -> Result<()> {
        info!("Ensuring image {} is available...", image);

        let options = CreateImageOptions {
            from_image: image,
            ..Default::default()
        };

        let mut stream = self.client.create_image(Some(options), None, None);

        while let Some(result) = stream.next().await {
            match result {
                Ok(info) => {
                    if let Some(status) = info.status {
                        debug!("Pull status: {}", status);
                    }
                }
                Err(e) => {
                    return Err(anyhow::anyhow!("Failed to pull image {}: {}", image, e));
                }
            }
        }

        info!("Image {} is ready", image);
        Ok(())
    }

    /// Create a new sandbox container from configuration.
    pub async fn create_sandbox(
        &self,
        config: &Config,
        project_dir: Option<&Path>,
    ) -> Result<String> {
        let name = &config.sandbox.name;
        let image = &config.sandbox.base_image;

        // Ensure the base image is available
        self.ensure_image(image).await?;

        // Build container configuration
        let mut labels = HashMap::new();
        labels.insert(LABEL_MANAGED_BY.to_string(), "true".to_string());
        labels.insert(LABEL_SANDBOX_NAME.to_string(), name.clone());
        labels.insert(LABEL_AGENT_TYPE.to_string(), config.agent.preferred.clone());

        // Convert port config to exposed ports
        let exposed_ports: HashMap<String, HashMap<(), ()>> = config
            .network
            .ports
            .iter()
            .map(|p| (format!("{}/tcp", p), HashMap::new()))
            .collect();

        // Build environment variables from config
        let mut env: Vec<String> = config
            .environment
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();

        // Add agent type as env var
        env.push(format!("AGENTKERNEL_AGENT={}", config.agent.preferred));

        // Build the setup command that installs dependencies
        let setup_cmd = self.build_setup_command(config);

        // Build host configuration with isolation settings
        let host_config = self.build_host_config(config, project_dir);

        let container_config = ContainerConfig {
            image: Some(image.clone()),
            labels: Some(labels),
            exposed_ports: Some(exposed_ports),
            env: Some(env),
            cmd: Some(vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                format!("{} && sleep infinity", setup_cmd),
            ]),
            tty: Some(true),
            open_stdin: Some(true),
            working_dir: Some("/app".to_string()),
            host_config: Some(host_config),
            ..Default::default()
        };

        let options = CreateContainerOptions {
            name: format!("agentkernel-{}", name),
            ..Default::default()
        };

        let response = self
            .client
            .create_container(Some(options), container_config)
            .await
            .with_context(|| format!("Failed to create container for sandbox '{}'", name))?;

        info!(
            "Created sandbox '{}' (container: {})",
            name,
            &response.id[..12]
        );
        Ok(response.id)
    }

    /// Build host configuration with isolation settings.
    fn build_host_config(&self, config: &Config, project_dir: Option<&Path>) -> HostConfig {
        // Build bind mounts from config
        let mut mounts = Vec::new();

        // Check if config already has a mount to /app
        let has_app_mount = config.mounts.values().any(|v| v == "/app");

        // Add project directory mount if provided and config doesn't already mount to /app
        if let Some(proj_dir) = project_dir
            && !has_app_mount
        {
            mounts.push(Mount {
                target: Some("/app".to_string()),
                source: Some(proj_dir.to_string_lossy().to_string()),
                typ: Some(MountTypeEnum::BIND),
                read_only: Some(false),
                ..Default::default()
            });
        }

        // Add mounts from config
        for (host_path, container_path) in &config.mounts {
            // Resolve relative paths if we have a project dir
            let resolved_path = if let Some(proj_dir) = project_dir {
                if host_path == "." {
                    proj_dir.to_string_lossy().to_string()
                } else if !Path::new(host_path).is_absolute() {
                    proj_dir.join(host_path).to_string_lossy().to_string()
                } else {
                    host_path.clone()
                }
            } else {
                host_path.clone()
            };

            mounts.push(Mount {
                target: Some(container_path.clone()),
                source: Some(resolved_path),
                typ: Some(MountTypeEnum::BIND),
                read_only: Some(false),
                ..Default::default()
            });
        }

        // Build port bindings (map container ports to same host ports)
        let port_bindings: HashMap<String, Option<Vec<PortBinding>>> = config
            .network
            .ports
            .iter()
            .map(|p| {
                (
                    format!("{}/tcp", p),
                    Some(vec![PortBinding {
                        host_ip: Some("127.0.0.1".to_string()),
                        host_port: Some(p.to_string()),
                    }]),
                )
            })
            .collect();

        // Convert CPU limit to nano CPUs (1 CPU = 1e9 nano CPUs)
        let nano_cpus = (config.resources.cpu_limit * 1e9) as i64;

        // Convert memory MB to bytes
        let memory_bytes = (config.resources.memory_mb * 1024 * 1024) as i64;

        // Security: drop dangerous capabilities
        let cap_drop = vec![
            "NET_RAW".to_string(),    // Prevent raw socket access
            "SYS_ADMIN".to_string(),  // Prevent container escapes
            "SYS_PTRACE".to_string(), // Prevent process tracing
            "MKNOD".to_string(),      // Prevent device node creation
        ];

        HostConfig {
            mounts: Some(mounts),
            port_bindings: Some(port_bindings),
            nano_cpus: Some(nano_cpus),
            memory: Some(memory_bytes),
            cap_drop: Some(cap_drop),
            // Prevent privilege escalation
            security_opt: Some(vec!["no-new-privileges:true".to_string()]),
            // Use default seccomp profile
            ..Default::default()
        }
    }

    /// Build the setup command that installs dependencies.
    fn build_setup_command(&self, config: &Config) -> String {
        let mut commands = Vec::new();

        // Install system dependencies
        if !config.dependencies.system.is_empty() {
            let pkgs = config.dependencies.system.join(" ");
            commands.push(format!(
                "apt-get update && apt-get install -y {} && rm -rf /var/lib/apt/lists/*",
                pkgs
            ));
        }

        // Install Python dependencies
        if !config.dependencies.python.is_empty() {
            let pkgs = config.dependencies.python.join(" ");
            commands.push(format!("pip install --no-cache-dir {}", pkgs));
        }

        // Install Node dependencies
        if !config.dependencies.node.is_empty() {
            let pkgs = config.dependencies.node.join(" ");
            commands.push(format!("npm install -g {}", pkgs));
        }

        // Run custom setup script if provided
        if let Some(setup) = &config.scripts.setup {
            commands.push(setup.clone());
        }

        if commands.is_empty() {
            "true".to_string()
        } else {
            commands.join(" && ")
        }
    }

    /// Start a sandbox container.
    pub async fn start_sandbox(&self, name: &str) -> Result<()> {
        let container_name = format!("agentkernel-{}", name);

        self.client
            .start_container(&container_name, None::<StartContainerOptions<String>>)
            .await
            .with_context(|| format!("Failed to start sandbox '{}'", name))?;

        info!("Started sandbox '{}'", name);
        Ok(())
    }

    /// Stop a sandbox container.
    pub async fn stop_sandbox(&self, name: &str) -> Result<()> {
        let container_name = format!("agentkernel-{}", name);

        self.client
            .stop_container(&container_name, Some(StopContainerOptions { t: 10 }))
            .await
            .with_context(|| format!("Failed to stop sandbox '{}'", name))?;

        info!("Stopped sandbox '{}'", name);
        Ok(())
    }

    /// Remove a sandbox container.
    pub async fn remove_sandbox(&self, name: &str) -> Result<()> {
        let container_name = format!("agentkernel-{}", name);

        self.client
            .remove_container(
                &container_name,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
            .with_context(|| format!("Failed to remove sandbox '{}'", name))?;

        info!("Removed sandbox '{}'", name);
        Ok(())
    }

    /// List all agentkernel-managed sandbox containers.
    pub async fn list_sandboxes(&self) -> Result<Vec<SandboxInfo>> {
        let mut filters = HashMap::new();
        filters.insert("label", vec![LABEL_MANAGED_BY]);

        let options = ListContainersOptions {
            all: true,
            filters,
            ..Default::default()
        };

        let containers = self
            .client
            .list_containers(Some(options))
            .await
            .context("Failed to list containers")?;

        let sandboxes = containers
            .into_iter()
            .filter_map(|c| {
                let labels = c.labels?;
                let name = labels.get(LABEL_SANDBOX_NAME)?.clone();
                let agent = labels
                    .get(LABEL_AGENT_TYPE)
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());

                Some(SandboxInfo {
                    name,
                    container_id: c.id?.chars().take(12).collect(),
                    agent,
                    status: c.status.unwrap_or_else(|| "unknown".to_string()),
                    image: c.image.unwrap_or_else(|| "unknown".to_string()),
                })
            })
            .collect();

        Ok(sandboxes)
    }

    /// Get the container name for a sandbox.
    pub fn container_name(name: &str) -> String {
        format!("agentkernel-{}", name)
    }

    /// Check if a sandbox exists and is running.
    pub async fn is_sandbox_running(&self, name: &str) -> Result<bool> {
        let sandboxes = self.list_sandboxes().await?;
        Ok(sandboxes
            .iter()
            .any(|s| s.name == name && s.status.starts_with("Up")))
    }
}
