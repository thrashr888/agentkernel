use anyhow::Result;

#[tokio::test]
async fn test_cloud_hypervisor_binary_spawns() -> Result<()> {
    // We can't fully `start` it because `Sandbox::start` requires a kernel and rootfs.
    // Instead, let's manually spawn the CH process and test the socket creation!
    
    // Using a hardcoded path since setup::default_data_dir() is not public or we can just use the known path
    let ch_bin = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".local/share/agentkernel/bin/cloud-hypervisor");
        
    if !ch_bin.exists() {
        println!("Cloud Hypervisor binary not found, skipping test");
        return Ok(());
    }

    if !std::path::Path::new("/dev/kvm").exists() {
        println!("KVM not available, skipping test");
        return Ok(());
    }

    let socket_path = std::path::PathBuf::from("/tmp/ch-api-ch-smoke-test.sock");
    let _ = std::fs::remove_file(&socket_path);

    let mut child = std::process::Command::new(&ch_bin)
        .arg("--api-socket")
        .arg(&socket_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    // Wait for socket
    let mut connected = false;
    for _ in 0..50 {
        if socket_path.exists() {
            connected = true;
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    
    // Shutdown
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&socket_path);
    
    assert!(connected, "Cloud Hypervisor API socket was not created");

    Ok(())
}
