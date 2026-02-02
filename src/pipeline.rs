//! Agent pipelines — chain sandboxes with data flowing between steps.
//!
//! Each step runs in its own sandbox. Output from step N is volume-mounted
//! as input in step N+1.
//!
//! Pipeline files use TOML:
//! ```toml
//! [[step]]
//! name = "generate"
//! image = "python:3.12-alpine"
//! command = "python generate_data.py"
//! output = "/app/output/"
//!
//! [[step]]
//! name = "process"
//! image = "node:22-alpine"
//! command = "node process.js"
//! input = "/app/input/"
//! output = "/app/results/"
//! ```

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// A pipeline definition loaded from TOML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pipeline {
    /// Pipeline steps in execution order.
    pub step: Vec<PipelineStep>,
}

/// A single pipeline step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStep {
    /// Step name (used for display and sandbox naming).
    pub name: String,
    /// Docker image to use for this step.
    pub image: String,
    /// Command to execute (shell-style string, split on whitespace).
    pub command: String,
    /// Input directory inside the sandbox (mounted from previous step's output).
    #[serde(default)]
    pub input: Option<String>,
    /// Output directory inside the sandbox (mounted to next step's input).
    #[serde(default)]
    pub output: Option<String>,
}

/// Result of a single step execution.
#[derive(Debug)]
#[allow(dead_code)]
pub struct StepResult {
    pub name: String,
    pub duration: std::time::Duration,
    pub success: bool,
    pub output: String,
}

/// Load a pipeline from a TOML file.
pub fn load(path: &Path) -> Result<Pipeline> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let pipeline: Pipeline =
        toml::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))?;

    if pipeline.step.is_empty() {
        bail!("Pipeline has no steps");
    }

    // Validate step names are unique and safe
    let mut seen = std::collections::HashSet::new();
    for step in &pipeline.step {
        if !seen.insert(&step.name) {
            bail!("Duplicate step name: '{}'", step.name);
        }
        // Prevent path traversal in step names (used in sandbox names and temp dirs)
        if step.name.contains('/')
            || step.name.contains('\\')
            || step.name.contains("..")
            || step.name.is_empty()
        {
            bail!(
                "Invalid step name '{}': must not contain '/', '\\', or '..'",
                step.name
            );
        }
    }

    Ok(pipeline)
}

/// Run a pipeline by executing each step sequentially.
///
/// Returns results for each step. Stops on first failure.
pub async fn run(
    pipeline: &Pipeline,
    manager: &mut crate::vmm::VmManager,
    prefix: &str,
) -> Result<Vec<StepResult>> {
    let total_steps = pipeline.step.len();
    let mut results = Vec::new();
    let total_start = Instant::now();

    // Track output directories for chaining and cleanup
    let mut prev_output_host: Option<PathBuf> = None;
    let mut temp_dirs: Vec<PathBuf> = Vec::new();

    for (i, step) in pipeline.step.iter().enumerate() {
        let step_num = i + 1;
        let sandbox_name = format!("{}-{}", prefix, step.name);

        eprint!("  [{}/{}] {:<20}", step_num, total_steps, step.name);

        let step_start = Instant::now();

        // Create sandbox
        manager.create(&sandbox_name, &step.image, 1, 512).await?;

        // Start the sandbox
        manager.start(&sandbox_name).await?;

        // If this step has input and there's output from a previous step, copy it in
        if let (Some(input_dir), Some(host_output)) = (&step.input, &prev_output_host) {
            // Copy output files from previous step into this sandbox's input dir
            if host_output.exists() {
                for entry in std::fs::read_dir(host_output)? {
                    let entry = entry?;
                    if entry.file_type()?.is_file() {
                        let content = std::fs::read(entry.path())?;
                        let dest = format!(
                            "{}/{}",
                            input_dir.trim_end_matches('/'),
                            entry.file_name().to_string_lossy()
                        );
                        // Ensure input directory exists
                        let mkdir_cmd =
                            vec!["mkdir".to_string(), "-p".to_string(), input_dir.clone()];
                        let _ = manager.exec_cmd(&sandbox_name, &mkdir_cmd).await;
                        manager.write_file(&sandbox_name, &dest, &content).await?;
                    }
                }
            }
        }

        // Execute the step command (split on whitespace)
        let cmd: Vec<String> = step.command.split_whitespace().map(String::from).collect();
        let exec_result = manager.exec_cmd(&sandbox_name, &cmd).await;

        let success = exec_result.is_ok();
        let output = match exec_result {
            Ok(out) => out,
            Err(e) => format!("Error: {}", e),
        };

        // If this step has output, copy it to a host temp dir for the next step
        if let Some(ref output_dir) = step.output {
            let host_dir =
                std::env::temp_dir().join(format!("agentkernel-pipeline-{}", sandbox_name));
            let _ = std::fs::create_dir_all(&host_dir);

            // List and copy output files
            let ls_cmd = vec!["ls".to_string(), "-1".to_string(), output_dir.clone()];
            if let Ok(ls_output) = manager.exec_cmd(&sandbox_name, &ls_cmd).await {
                for filename in ls_output.lines().filter(|l| !l.is_empty()) {
                    let src = format!("{}/{}", output_dir.trim_end_matches('/'), filename);
                    if let Ok(content) = manager.read_file(&sandbox_name, &src).await {
                        let _ = std::fs::write(host_dir.join(filename), content);
                    }
                }
            }
            temp_dirs.push(host_dir.clone());
            prev_output_host = Some(host_dir);
        }

        let duration = step_start.elapsed();

        // Clean up sandbox
        let _ = manager.stop(&sandbox_name).await;
        let _ = manager.remove(&sandbox_name).await;

        if success {
            eprintln!(" done ({:.1}s)", duration.as_secs_f64());
        } else {
            eprintln!(" FAILED ({:.1}s)", duration.as_secs_f64());
        }

        results.push(StepResult {
            name: step.name.clone(),
            duration,
            success,
            output,
        });

        if !success {
            bail!(
                "Pipeline failed at step '{}' ({}/{})",
                step.name,
                step_num,
                total_steps
            );
        }
    }

    let total_duration = total_start.elapsed();
    eprintln!("  Done ({:.1}s total)", total_duration.as_secs_f64());

    // Clean up all temp dirs
    for dir in &temp_dirs {
        let _ = std::fs::remove_dir_all(dir);
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_pipeline() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("pipeline.toml");
        std::fs::write(
            &path,
            r#"
[[step]]
name = "generate"
image = "python:3.12-alpine"
command = "python generate.py"
output = "/app/output/"

[[step]]
name = "process"
image = "node:22-alpine"
command = "node process.js"
input = "/app/input/"
output = "/app/results/"

[[step]]
name = "analyze"
image = "python:3.12-alpine"
command = "python analyze.py"
input = "/app/input/"
"#,
        )
        .unwrap();

        let pipeline = load(&path).unwrap();
        assert_eq!(pipeline.step.len(), 3);
        assert_eq!(pipeline.step[0].name, "generate");
        assert_eq!(pipeline.step[0].image, "python:3.12-alpine");
        assert_eq!(pipeline.step[1].input, Some("/app/input/".to_string()));
        assert_eq!(pipeline.step[1].output, Some("/app/results/".to_string()));
        assert!(pipeline.step[2].output.is_none());
    }

    #[test]
    fn test_load_empty_pipeline() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("empty.toml");
        std::fs::write(&path, "step = []\n").unwrap();

        let result = load(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_load_duplicate_names() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("dup.toml");
        std::fs::write(
            &path,
            r#"
[[step]]
name = "step1"
image = "alpine"
command = "echo a"

[[step]]
name = "step1"
image = "alpine"
command = "echo b"
"#,
        )
        .unwrap();

        let result = load(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_load_missing_file() {
        let result = load(Path::new("/nonexistent/pipeline.toml"));
        assert!(result.is_err());
    }

    #[test]
    fn test_load_invalid_step_name_path_traversal() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(
            &path,
            r#"
[[step]]
name = "../escape"
image = "alpine"
command = "echo bad"
"#,
        )
        .unwrap();

        let result = load(&path);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid step name")
        );
    }

    #[test]
    fn test_load_invalid_step_name_slash() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("slash.toml");
        std::fs::write(
            &path,
            r#"
[[step]]
name = "step/one"
image = "alpine"
command = "echo bad"
"#,
        )
        .unwrap();

        let result = load(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_pipeline_step_serde() {
        let step = PipelineStep {
            name: "test".to_string(),
            image: "alpine:3.20".to_string(),
            command: "echo hello".to_string(),
            input: Some("/in".to_string()),
            output: Some("/out".to_string()),
        };

        let json = serde_json::to_string(&step).unwrap();
        let parsed: PipelineStep = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "test");
        assert_eq!(parsed.input, Some("/in".to_string()));
    }

    #[test]
    fn test_pipeline_step_defaults() {
        let toml_str = r#"
[[step]]
name = "simple"
image = "alpine"
command = "echo hi"
"#;
        let pipeline: Pipeline = toml::from_str(toml_str).unwrap();
        assert!(pipeline.step[0].input.is_none());
        assert!(pipeline.step[0].output.is_none());
    }

    #[test]
    fn test_step_result() {
        let result = StepResult {
            name: "test".to_string(),
            duration: std::time::Duration::from_millis(1500),
            success: true,
            output: "hello".to_string(),
        };
        assert!(result.success);
        assert_eq!(result.duration.as_millis(), 1500);
    }
}
