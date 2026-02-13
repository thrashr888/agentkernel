//! Gondolin-style secrets demo for agentkernel.
//!
//! Demonstrates the same pattern as github.com/earendil-works/gondolin:
//! secrets are injected as HTTP headers by the host proxy and never
//! enter the sandbox VM. The sandbox only sees placeholder env vars.
//!
//! Usage:
//!   export GITHUB_TOKEN="ghp_..."
//!   cargo run --example gondolin_demo
//!
//! (Or copy this file into a project that depends on agentkernel-sdk.)

use agentkernel_sdk::{AgentKernel, CreateSandboxOptions, SecurityProfile};

const SANDBOX_NAME: &str = "gondolin-demo";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let token = std::env::var("GITHUB_TOKEN").unwrap_or_default();
    if token.is_empty() {
        eprintln!("Set GITHUB_TOKEN env var first:");
        eprintln!(r#"  export GITHUB_TOKEN="ghp_...""#);
        std::process::exit(1);
    }

    let client = AgentKernel::builder().build()?;

    // Create sandbox with secret binding (Gondolin pattern).
    // The proxy intercepts HTTPS requests to api.github.com and injects
    // an Authorization header with the real token. The VM never sees it.
    println!("Creating sandbox with secret binding...");
    client
        .create_sandbox(
            SANDBOX_NAME,
            Some(CreateSandboxOptions {
                image: Some("python:3.12-slim".into()),
                profile: Some(SecurityProfile::Moderate),
                secrets: vec![format!("GITHUB_TOKEN={token}:api.github.com")],
                ..Default::default()
            }),
        )
        .await?;

    // 1. Verify the sandbox doesn't have the real token
    println!("\n--- Env check (token should be a placeholder) ---");
    let env_check = client
        .exec_in_sandbox(
            SANDBOX_NAME,
            &["sh", "-c", "echo GITHUB_TOKEN=$GITHUB_TOKEN"],
            None,
        )
        .await?;
    println!("{}", env_check.output.trim());

    // 2. Make an authenticated API call through the proxy (HTTPS with MITM).
    //    The proxy transparently injects Authorization: Bearer <real-token>.
    println!("\n--- Calling GitHub API (secret injected by proxy) ---");
    let result = client
        .exec_in_sandbox(
            SANDBOX_NAME,
            &[
                "python3",
                "-c",
                r#"import urllib.request, json
req = urllib.request.Request("https://api.github.com/user",
    headers={"Accept": "application/vnd.github+json"})
resp = urllib.request.urlopen(req, timeout=15)
print(resp.read().decode())"#,
            ],
            None,
        )
        .await?;
    let user: serde_json::Value = serde_json::from_str(&result.output)?;
    println!(
        "Authenticated as: {} ({})",
        user["login"].as_str().unwrap_or("?"),
        user["name"].as_str().unwrap_or("")
    );
    println!("Public repos: {}", user["public_repos"]);

    // 3. Try an unauthorized host — should be blocked by the proxy
    println!("\n--- Attempting unauthorized host (should fail) ---");
    let blocked = client
        .exec_in_sandbox(
            SANDBOX_NAME,
            &[
                "python3",
                "-c",
                "import urllib.request; urllib.request.urlopen('https://evil.com', timeout=5)",
            ],
            None,
        )
        .await;
    match blocked {
        Ok(_) => println!("ERROR: request to unauthorized host should have failed"),
        Err(_) => println!("Blocked as expected"),
    }

    println!("\nDone. Secret never entered the VM.");

    // Cleanup
    let _ = client.remove_sandbox(SANDBOX_NAME).await;
    println!("Sandbox removed.");

    Ok(())
}
