use agentkernel::backend::{
    ContainerRuntime, DockerSandbox, FullStateSnapshot, HyperlightSandbox, RemoteProvider,
    RemoteSandbox, RemoteSandboxContext, Sandbox,
};

#[cfg(target_os = "macos")]
use agentkernel::backend::AppleSandbox;
#[cfg(feature = "kubernetes")]
use agentkernel::backend::KubernetesSandbox;
#[cfg(feature = "nomad")]
use agentkernel::backend::NomadSandbox;
#[cfg(any(feature = "kubernetes", feature = "nomad"))]
use agentkernel::config::OrchestratorConfig;

fn compatibility_metadata() -> FullStateSnapshot {
    FullStateSnapshot {
        firecracker_version: "1.16.1".to_string(),
        architecture: "x86_64".to_string(),
        host_kernel_release: "6.18.45-agentkernel".to_string(),
        host_identity_sha256: "host-id".to_string(),
        cpu_fingerprint_sha256: "cpu-id".to_string(),
        guest_kernel_release: "6.18.45-agentkernel".to_string(),
    }
}

fn assert_empty(directory: &std::path::Path) {
    assert!(
        std::fs::read_dir(directory).unwrap().next().is_none(),
        "an unsupported full-state operation must not create artifacts"
    );
}

async fn assert_full_state_unsupported(mut sandbox: Box<dyn Sandbox>) {
    let backend = sandbox.backend_type().to_string();
    let checkpoint = tempfile::tempdir().unwrap();
    let expected = format!(
        "Backend '{}' does not support full-state pause/resume",
        backend
    );

    let pause_error = sandbox.pause_to(checkpoint.path()).await.unwrap_err();
    assert_eq!(pause_error.to_string(), expected);
    assert_empty(checkpoint.path());

    let restore_error = sandbox
        .restore_from(checkpoint.path(), &compatibility_metadata())
        .await
        .unwrap_err();
    assert_eq!(restore_error.to_string(), expected);
    assert_empty(checkpoint.path());
}

#[test]
fn full_state_compatibility_metadata_has_a_stable_json_contract() {
    let encoded = serde_json::to_value(compatibility_metadata()).unwrap();
    assert_eq!(
        encoded,
        serde_json::json!({
            "firecracker_version": "1.16.1",
            "architecture": "x86_64",
            "host_kernel_release": "6.18.45-agentkernel",
            "host_identity_sha256": "host-id",
            "cpu_fingerprint_sha256": "cpu-id",
            "guest_kernel_release": "6.18.45-agentkernel"
        })
    );

    let decoded: FullStateSnapshot = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, compatibility_metadata());
}

#[tokio::test]
async fn non_firecracker_backends_reject_full_state_without_artifacts() {
    let mut sandboxes: Vec<Box<dyn Sandbox>> = vec![
        Box::new(DockerSandbox::new(
            "full-state-docker-contract",
            ContainerRuntime::Docker,
        )),
        Box::new(DockerSandbox::new(
            "full-state-podman-contract",
            ContainerRuntime::Podman,
        )),
        Box::new(HyperlightSandbox::new("full-state-hyperlight-contract")),
    ];

    for provider in [
        RemoteProvider::Daytona,
        RemoteProvider::Runloop,
        RemoteProvider::E2B,
        RemoteProvider::Modal,
        RemoteProvider::AgentComputer,
    ] {
        sandboxes.push(Box::new(RemoteSandbox::new(
            provider,
            &format!("full-state-{provider}-contract"),
            RemoteSandboxContext::default(),
        )));
    }

    #[cfg(target_os = "macos")]
    sandboxes.push(Box::new(AppleSandbox::new("full-state-apple-contract")));

    #[cfg(feature = "kubernetes")]
    sandboxes.push(Box::new(KubernetesSandbox::new(
        "full-state-kubernetes-contract",
        &OrchestratorConfig::default(),
    )));

    #[cfg(feature = "nomad")]
    sandboxes.push(Box::new(NomadSandbox::new(
        "full-state-nomad-contract",
        &OrchestratorConfig::default(),
    )));

    for sandbox in sandboxes {
        assert_full_state_unsupported(sandbox).await;
    }
}
