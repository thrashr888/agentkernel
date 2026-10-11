//! Opt-in real KVM gate. Missing prerequisites fail instead of silently passing.
use agentkernel::backend::cloud_hypervisor::{CloudHypervisorSandbox, cloud_hypervisor_readiness};
use agentkernel::backend::{Sandbox, SandboxConfig};
use anyhow::Result;

#[tokio::test]
#[ignore = "requires Linux x86_64/KVM, CLOUD_HYPERVISOR_BIN, CLOUD_HYPERVISOR_KERNEL and setup base rootfs"]
async fn cloud_hypervisor_boot_exec_and_stop() -> Result<()> {
    assert_eq!(
        std::env::var("AGENTKERNEL_CH_SMOKE").as_deref(),
        Ok("1"),
        "explicit opt-in required"
    );
    let readiness = cloud_hypervisor_readiness();
    assert!(readiness.usable, "{}", readiness.reason);
    let mut sandbox = CloudHypervisorSandbox::new("ch-lifecycle-smoke")?;
    let config = SandboxConfig {
        image: "base".into(),
        network: false,
        memory_mb: 256,
        ..Default::default()
    };
    sandbox.start(&config).await?;
    assert!(sandbox.is_running());
    let result = sandbox
        .exec(&["/bin/sh", "-c", "printf agentkernel-ch-ready"])
        .await?;
    assert_eq!(result.exit_code, 0, "{}", result.stderr);
    assert_eq!(result.stdout, "agentkernel-ch-ready");
    sandbox
        .write_file("/tmp/ch-test.txt", b"round-trip")
        .await?;
    assert_eq!(sandbox.read_file("/tmp/ch-test.txt").await?, b"round-trip");
    sandbox.stop().await?;
    assert!(!sandbox.is_running());
    sandbox.stop().await?;
    // Restart exercises fresh COW state and reuse of the private runtime directory.
    sandbox.start(&config).await?;
    assert_eq!(sandbox.exec(&["/bin/true"]).await?.exit_code, 0);
    sandbox.stop().await?;
    // A read-only guest must reject writes rather than silently mounting rw.
    sandbox
        .start(&SandboxConfig {
            read_only: true,
            ..config
        })
        .await?;
    assert_ne!(
        sandbox
            .exec(&["/bin/sh", "-c", "echo denied > /ch-readonly-test"])
            .await?
            .exit_code,
        0
    );
    sandbox.stop().await?;
    Ok(())
}
