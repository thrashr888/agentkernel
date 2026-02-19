//! Durable object runtime: wake/hibernate lifecycle and background hibernation daemon.

use crate::orchestration_store::{
    CreateDurableObject, DurableObjectRecord, DurableObjectStatus, OrchestrationStore,
};
use crate::vmm::VmManager;
use anyhow::{Context, Result, bail};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{Duration, sleep};

/// Port inside the sandbox where the durable object HTTP handler listens.
const OBJECT_PORT: u16 = 9333;

/// How often the hibernation daemon checks for idle objects.
const HIBERNATION_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Default idle timeout if not specified on the object (5 minutes).
const DEFAULT_IDLE_TIMEOUT_SECS: i64 = 300;

/// Handle an incoming `call()` request for a durable object.
///
/// 1. Look up (or auto-create) the object by class + object_id.
/// 2. If hibernating, wake it: start sandbox, wait for health, push storage.
/// 3. Forward the method call to the sandbox's HTTP handler.
/// 4. Touch the object's updated_at timestamp.
pub async fn handle_object_call(
    store: &OrchestrationStore,
    manager: &Arc<RwLock<VmManager>>,
    class: &str,
    object_id: &str,
    method: &str,
    body: bytes::Bytes,
) -> Result<(u16, String)> {
    // 1. Find or auto-create the object
    let obj = match store.find_object_by_class_and_id(class, object_id)? {
        Some(obj) => obj,
        None => {
            // Auto-create with Hibernating status
            store.create_object(CreateDurableObject {
                class: class.to_string(),
                object_id: object_id.to_string(),
                sandbox: None,
                storage: None,
                idle_timeout_seconds: DEFAULT_IDLE_TIMEOUT_SECS,
            })?
        }
    };

    // 2. Wake if hibernating
    let sandbox_name = if obj.status == DurableObjectStatus::Hibernating {
        wake_object(store, manager, &obj).await?
    } else {
        obj.sandbox
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Active object {} has no sandbox", obj.id))?
    };

    // 3. Forward the call
    let sandbox_ip = {
        let mgr = manager.read().await;
        mgr.get_container_ip(&sandbox_name)
            .ok_or_else(|| anyhow::anyhow!("No IP for sandbox {}", sandbox_name))?
    };

    let url = format!("http://{}:{}/{}", sandbox_ip, OBJECT_PORT, method);
    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .body(body.to_vec())
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .with_context(|| format!("Failed to call object method {} at {}", method, url))?;

    let status = resp.status().as_u16();
    let resp_body = resp
        .text()
        .await
        .context("Failed to read object response body")?;

    // 4. Touch updated_at
    store.touch_object(&obj.id)?;

    Ok((status, resp_body))
}

/// Wake a hibernating object: start its sandbox, wait for health, push storage.
async fn wake_object(
    store: &OrchestrationStore,
    manager: &Arc<RwLock<VmManager>>,
    obj: &DurableObjectRecord,
) -> Result<String> {
    let sandbox_name = format!("do-{}-{}", obj.class, obj.object_id);

    // Start sandbox
    {
        let mut mgr = manager.write().await;

        // Create if it doesn't exist
        if !mgr.exists(&sandbox_name) {
            mgr.create(&sandbox_name, "alpine:3.20", 1, 256).await?;
        }

        // Start if not running
        if !mgr.is_running(&sandbox_name) {
            mgr.start(&sandbox_name).await?;
        }
    }

    // Wait for health check (poll :9333/__health)
    let sandbox_ip = {
        let mgr = manager.read().await;
        mgr.get_container_ip(&sandbox_name)
            .ok_or_else(|| anyhow::anyhow!("No IP for sandbox {}", sandbox_name))?
    };

    let health_url = format!("http://{}:{}/__health", sandbox_ip, OBJECT_PORT);
    let client = reqwest::Client::new();
    let mut healthy = false;
    for _ in 0..30 {
        match client
            .get(&health_url)
            .timeout(Duration::from_secs(2))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                healthy = true;
                break;
            }
            _ => sleep(Duration::from_millis(500)).await,
        }
    }
    if !healthy {
        bail!(
            "Object {}/{} sandbox {} failed health check after 15s",
            obj.class,
            obj.object_id,
            sandbox_name
        );
    }

    // Push persisted storage
    if obj.storage != serde_json::json!({}) {
        let storage_url = format!("http://{}:{}/__storage", sandbox_ip, OBJECT_PORT);
        let _ = client
            .post(&storage_url)
            .json(&obj.storage)
            .timeout(Duration::from_secs(5))
            .send()
            .await;
    }

    // Update status to Active
    store.update_object_status(&obj.id, DurableObjectStatus::Active, Some(&sandbox_name))?;

    Ok(sandbox_name)
}

/// Hibernate an active object: fetch storage, stop sandbox, update status.
pub async fn hibernate_object(
    store: &OrchestrationStore,
    manager: &Arc<RwLock<VmManager>>,
    obj: &DurableObjectRecord,
) -> Result<()> {
    let sandbox_name = match &obj.sandbox {
        Some(name) => name.clone(),
        None => return Ok(()), // Nothing to hibernate
    };

    // Fetch storage from sandbox
    let sandbox_ip = {
        let mgr = manager.read().await;
        mgr.get_container_ip(&sandbox_name)
    };

    if let Some(ip) = sandbox_ip {
        let storage_url = format!("http://{}:{}/__storage", ip, OBJECT_PORT);
        let client = reqwest::Client::new();
        if let Ok(resp) = client
            .get(&storage_url)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            && let Ok(storage) = resp.json::<serde_json::Value>().await
        {
            store.update_object_storage(&obj.id, &storage)?;
        }
    }

    // Stop sandbox
    {
        let mut mgr = manager.write().await;
        if mgr.is_running(&sandbox_name) {
            mgr.stop(&sandbox_name).await?;
        }
    }

    // Update status to Hibernating
    store.update_object_status(&obj.id, DurableObjectStatus::Hibernating, None)?;

    eprintln!(
        "[object-runtime] Hibernated {}/{} (sandbox: {})",
        obj.class, obj.object_id, sandbox_name
    );

    Ok(())
}

/// Background daemon that hibernates idle objects.
///
/// Runs in a loop, checking every 30s for active objects whose `updated_at`
/// is older than their `idle_timeout_seconds`.
pub async fn hibernation_daemon(store: Arc<OrchestrationStore>, manager: Arc<RwLock<VmManager>>) {
    eprintln!("[object-runtime] Hibernation daemon started (poll interval: 30s)");
    loop {
        sleep(HIBERNATION_POLL_INTERVAL).await;

        let active = match store.list_active_objects() {
            Ok(objects) => objects,
            Err(e) => {
                eprintln!("[object-runtime] Error listing active objects: {}", e);
                continue;
            }
        };

        let now = chrono::Utc::now();
        for obj in active {
            let idle_timeout = if obj.idle_timeout_seconds > 0 {
                obj.idle_timeout_seconds
            } else {
                DEFAULT_IDLE_TIMEOUT_SECS
            };

            let updated = match chrono::DateTime::parse_from_rfc3339(&obj.updated_at) {
                Ok(dt) => dt.with_timezone(&chrono::Utc),
                Err(_) => continue,
            };

            let idle_secs = (now - updated).num_seconds();
            if idle_secs >= idle_timeout {
                eprintln!(
                    "[object-runtime] Hibernating {}/{} (idle {}s >= {}s)",
                    obj.class, obj.object_id, idle_secs, idle_timeout
                );
                if let Err(e) = hibernate_object(&store, &manager, &obj).await {
                    eprintln!(
                        "[object-runtime] Failed to hibernate {}/{}: {}",
                        obj.class, obj.object_id, e
                    );
                }
            }
        }
    }
}
