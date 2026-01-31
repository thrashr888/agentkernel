//! Kubernetes warm pool manager.
//!
//! Pre-creates pods labeled `agentkernel.io/pool=warm` that can be quickly
//! relabeled to `active` when acquired. When released, pods are deleted and
//! the pool replenishes to maintain the target warm count.
//!
//! Compile with `--features kubernetes` to enable.

#![cfg(feature = "kubernetes")]

use anyhow::{Context, Result, bail};
use k8s_openapi::api::core::v1::{Container, Pod, PodSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::Client;
use kube::api::{Api, DeleteParams, ListParams, Patch, PatchParams, PostParams};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Configuration for the Kubernetes warm pool
pub struct KubernetesPoolConfig {
    /// Target number of warm pods
    pub warm_pool_size: usize,
    /// Maximum number of total pods (warm + active)
    pub max_pool_size: usize,
    /// Container image for warm pods
    pub image: String,
    /// Kubernetes namespace
    pub namespace: String,
    /// Optional runtime class
    pub runtime_class: Option<String>,
    /// Resource limits (memory in Mi)
    pub memory_mb: u64,
    /// Resource limits (CPU in vcpus)
    pub vcpus: u32,
}

impl Default for KubernetesPoolConfig {
    fn default() -> Self {
        Self {
            warm_pool_size: 10,
            max_pool_size: 50,
            image: "alpine:3.20".to_string(),
            namespace: "agentkernel".to_string(),
            runtime_class: None,
            memory_mb: 512,
            vcpus: 1,
        }
    }
}

/// Statistics about the warm pool state
#[derive(Debug, Clone)]
pub struct PoolStats {
    /// Number of warm (available) pods
    pub warm: usize,
    /// Number of active (in-use) pods
    pub active: usize,
    /// Target warm pool size
    pub target_warm: usize,
    /// Maximum total pods
    pub max_total: usize,
}

/// Manages a pool of pre-warmed Kubernetes pods for fast sandbox acquisition.
pub struct KubernetesPool {
    config: KubernetesPoolConfig,
    client: Client,
    /// Guard concurrent pool operations
    lock: Arc<Mutex<()>>,
}

impl KubernetesPool {
    /// Create a new pool manager with the given client and configuration
    pub fn new(client: Client, config: KubernetesPoolConfig) -> Self {
        Self {
            config,
            client,
            lock: Arc::new(Mutex::new(())),
        }
    }

    /// Standard labels for warm pool pods
    fn warm_labels() -> BTreeMap<String, String> {
        let mut labels = BTreeMap::new();
        labels.insert(
            "agentkernel.io/managed-by".to_string(),
            "agentkernel".to_string(),
        );
        labels.insert("agentkernel.io/pool".to_string(), "warm".to_string());
        labels
    }

    /// Build a warm pool Pod spec
    fn build_warm_pod(&self, index: usize) -> Pod {
        let pod_name = format!("agentkernel-warm-{}", index);
        let mut labels = Self::warm_labels();
        labels.insert("agentkernel.io/warm-index".to_string(), index.to_string());

        // Minimal security context
        let security_context = k8s_openapi::api::core::v1::SecurityContext {
            privileged: Some(false),
            allow_privilege_escalation: Some(false),
            run_as_non_root: Some(true),
            run_as_user: Some(1000),
            capabilities: Some(k8s_openapi::api::core::v1::Capabilities {
                drop: Some(vec!["ALL".to_string()]),
                ..Default::default()
            }),
            ..Default::default()
        };

        // Resource limits
        let mut resource_limits = BTreeMap::new();
        resource_limits.insert(
            "memory".to_string(),
            k8s_openapi::apimachinery::pkg::api::resource::Quantity(format!(
                "{}Mi",
                self.config.memory_mb
            )),
        );
        resource_limits.insert(
            "cpu".to_string(),
            k8s_openapi::apimachinery::pkg::api::resource::Quantity(format!(
                "{}m",
                self.config.vcpus * 1000
            )),
        );

        let container = Container {
            name: "sandbox".to_string(),
            image: Some(self.config.image.clone()),
            command: Some(vec![
                "sh".to_string(),
                "-c".to_string(),
                "sleep infinity".to_string(),
            ]),
            security_context: Some(security_context),
            resources: Some(k8s_openapi::api::core::v1::ResourceRequirements {
                limits: Some(resource_limits),
                ..Default::default()
            }),
            ..Default::default()
        };

        Pod {
            metadata: ObjectMeta {
                name: Some(pod_name),
                namespace: Some(self.config.namespace.clone()),
                labels: Some(labels),
                ..Default::default()
            },
            spec: Some(PodSpec {
                containers: vec![container],
                restart_policy: Some("Never".to_string()),
                automount_service_account_token: Some(false),
                runtime_class_name: self.config.runtime_class.clone(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Initialize the warm pool by creating pods up to the target size
    pub async fn initialize(&self) -> Result<()> {
        let _guard = self.lock.lock().await;
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.config.namespace);

        // Count existing warm pods
        let lp = ListParams::default().labels("agentkernel.io/pool=warm");
        let existing = pods.list(&lp).await?;
        let existing_count = existing.items.len();

        // Create additional pods to reach target
        let needed = self.config.warm_pool_size.saturating_sub(existing_count);
        let start_index = existing_count;

        for i in 0..needed {
            let pod = self.build_warm_pod(start_index + i);
            match pods.create(&PostParams::default(), &pod).await {
                Ok(_) => {}
                Err(e) => {
                    eprintln!(
                        "Warning: Failed to create warm pod {}: {}",
                        start_index + i,
                        e
                    );
                }
            }
        }

        Ok(())
    }

    /// Acquire a warm pod from the pool, relabeling it to active.
    ///
    /// Returns the pod name if successful. The caller takes ownership and is
    /// responsible for calling `release()` when done.
    pub async fn acquire(&self, sandbox_name: &str) -> Result<String> {
        let _guard = self.lock.lock().await;
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.config.namespace);

        // Find a warm pod that is Running
        let lp = ListParams::default().labels("agentkernel.io/pool=warm");
        let warm_pods = pods.list(&lp).await?;

        let warm_pod = warm_pods
            .items
            .into_iter()
            .find(|p| p.status.as_ref().and_then(|s| s.phase.as_deref()) == Some("Running"))
            .ok_or_else(|| anyhow::anyhow!("No warm pods available in pool"))?;

        let pod_name = warm_pod
            .metadata
            .name
            .ok_or_else(|| anyhow::anyhow!("Warm pod has no name"))?;

        // Relabel from warm -> active with sandbox name
        let patch = json!({
            "metadata": {
                "labels": {
                    "agentkernel.io/pool": "active",
                    "agentkernel.io/sandbox": sandbox_name,
                }
            }
        });

        pods.patch(&pod_name, &PatchParams::default(), &Patch::Merge(&patch))
            .await
            .context("Failed to relabel warm pod to active")?;

        Ok(pod_name)
    }

    /// Release a pod back to the pool (deletes it and replenishes).
    pub async fn release(&self, pod_name: &str) -> Result<()> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.config.namespace);

        // Delete the pod
        let _ = pods.delete(pod_name, &DeleteParams::default()).await;

        // Replenish the pool
        self.replenish().await?;

        Ok(())
    }

    /// Replenish the warm pool to maintain the target warm count.
    pub async fn replenish(&self) -> Result<()> {
        let _guard = self.lock.lock().await;
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.config.namespace);

        // Count current warm pods
        let warm_lp = ListParams::default().labels("agentkernel.io/pool=warm");
        let warm_pods = pods.list(&warm_lp).await?;
        let warm_count = warm_pods.items.len();

        // Count total active pods
        let active_lp = ListParams::default().labels("agentkernel.io/pool=active");
        let active_pods = pods.list(&active_lp).await?;
        let active_count = active_pods.items.len();

        // Don't exceed max pool size
        let total = warm_count + active_count;
        if total >= self.config.max_pool_size {
            return Ok(());
        }

        // Calculate how many to create
        let needed = self.config.warm_pool_size.saturating_sub(warm_count);
        let available_capacity = self.config.max_pool_size.saturating_sub(total);
        let to_create = needed.min(available_capacity);

        // Find next available index
        let max_index = warm_pods
            .items
            .iter()
            .filter_map(|p| {
                p.metadata
                    .labels
                    .as_ref()
                    .and_then(|l| l.get("agentkernel.io/warm-index"))
                    .and_then(|v| v.parse::<usize>().ok())
            })
            .max()
            .unwrap_or(0);

        for i in 0..to_create {
            let pod = self.build_warm_pod(max_index + 1 + i);
            match pods.create(&PostParams::default(), &pod).await {
                Ok(_) => {}
                Err(e) => {
                    eprintln!("Warning: Failed to replenish warm pod: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Get current pool statistics
    pub async fn stats(&self) -> Result<PoolStats> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.config.namespace);

        let warm_lp = ListParams::default().labels("agentkernel.io/pool=warm");
        let warm_count = pods.list(&warm_lp).await?.items.len();

        let active_lp = ListParams::default().labels("agentkernel.io/pool=active");
        let active_count = pods.list(&active_lp).await?.items.len();

        Ok(PoolStats {
            warm: warm_count,
            active: active_count,
            target_warm: self.config.warm_pool_size,
            max_total: self.config.max_pool_size,
        })
    }

    /// Spawn a background task that periodically replenishes the pool
    pub fn spawn_replenish_task(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                interval.tick().await;
                if let Err(e) = self.replenish().await {
                    eprintln!("Warm pool replenish error: {}", e);
                }
            }
        })
    }

    /// Clean up all warm pool pods (for shutdown)
    pub async fn cleanup(&self) -> Result<()> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.config.namespace);

        let lp = ListParams::default()
            .labels("agentkernel.io/managed-by=agentkernel,agentkernel.io/pool=warm");
        let warm_pods = pods.list(&lp).await?;

        for pod in warm_pods.items {
            if let Some(name) = pod.metadata.name {
                let _ = pods.delete(&name, &DeleteParams::default()).await;
            }
        }

        Ok(())
    }
}
