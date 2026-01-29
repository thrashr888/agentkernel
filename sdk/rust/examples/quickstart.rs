use agentkernel_sdk::AgentKernel;

#[tokio::main]
async fn main() -> agentkernel_sdk::Result<()> {
    let client = AgentKernel::builder().build()?;

    // Health check
    let status = client.health().await?;
    println!("Health: {status}");

    // Run a command
    let result = client
        .run(&["echo", "Hello from agentkernel!"], None)
        .await?;
    println!("Output: {}", result.output);

    // List sandboxes
    let sandboxes = client.list_sandboxes().await?;
    println!("Sandboxes: {sandboxes:?}");

    Ok(())
}
