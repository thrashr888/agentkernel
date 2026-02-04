use agentkernel_sdk::{AgentKernel, CreateSandboxOptions};

#[tokio::main]
async fn main() -> agentkernel_sdk::Result<()> {
    let client = AgentKernel::builder().build()?;

    let opts = CreateSandboxOptions {
        image: Some("python:3.12-alpine".to_string()),
        ..Default::default()
    };

    // Create a sandbox session — auto-removed when closure returns
    client
        .with_sandbox("demo", Some(opts), |sb| async move {
            // Install a package
            sb.run(&["pip", "install", "numpy"]).await?;

            // Run code
            let result = sb
                .run(&[
                    "python3",
                    "-c",
                    "import numpy; print(f'numpy {numpy.__version__}')",
                ])
                .await?;
            println!("{}", result.output);

            Ok(())
        })
        .await?;
    // Sandbox is automatically removed here

    Ok(())
}
