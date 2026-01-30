use std::time::Duration;

use agentkernel_sdk::AgentKernel;

#[tokio::main]
async fn main() -> agentkernel_sdk::Result<()> {
    // Explicit configuration
    let client = AgentKernel::builder()
        .base_url("http://localhost:18888")
        .api_key("sk-my-api-key")
        .timeout(Duration::from_secs(60))
        .build()?;

    let result = client.run(&["echo", "configured!"], None).await?;
    println!("{}", result.output);

    Ok(())
}
