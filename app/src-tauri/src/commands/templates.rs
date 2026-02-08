use crate::types::TemplateInfo;

/// Return the hardcoded list of built-in templates.
///
/// The data here mirrors `BUILTIN_TEMPLATES` in `src/template.rs` so the
/// desktop app does not need to link against the main agentkernel crate.
#[tauri::command(rename_all = "snake_case")]
pub async fn list_templates() -> Result<Vec<TemplateInfo>, String> {
    Ok(builtin_templates())
}

fn builtin_templates() -> Vec<TemplateInfo> {
    vec![
        // ----- Agent Sandboxes -----
        TemplateInfo {
            name: "claude-sandbox".into(),
            description: "Claude Code agent sandbox".into(),
            category: "Agent Sandboxes".into(),
            base_image: "node:22-alpine".into(),
            vcpus: 2,
            memory_mb: 1024,
        },
        TemplateInfo {
            name: "codex-sandbox".into(),
            description: "OpenAI Codex agent sandbox".into(),
            category: "Agent Sandboxes".into(),
            base_image: "node:22-alpine".into(),
            vcpus: 2,
            memory_mb: 1024,
        },
        TemplateInfo {
            name: "gemini-sandbox".into(),
            description: "Gemini CLI agent sandbox".into(),
            category: "Agent Sandboxes".into(),
            base_image: "node:22-alpine".into(),
            vcpus: 2,
            memory_mb: 1024,
        },
        TemplateInfo {
            name: "opencode-sandbox".into(),
            description: "OpenCode agent sandbox".into(),
            category: "Agent Sandboxes".into(),
            base_image: "node:22-alpine".into(),
            vcpus: 2,
            memory_mb: 1024,
        },
        TemplateInfo {
            name: "amp-sandbox".into(),
            description: "Amp (Sourcegraph) agent sandbox".into(),
            category: "Agent Sandboxes".into(),
            base_image: "node:22-alpine".into(),
            vcpus: 2,
            memory_mb: 1024,
        },
        TemplateInfo {
            name: "pi-sandbox".into(),
            description: "Pi coding agent sandbox".into(),
            category: "Agent Sandboxes".into(),
            base_image: "node:22-alpine".into(),
            vcpus: 2,
            memory_mb: 1024,
        },
        // ----- Languages -----
        TemplateInfo {
            name: "bash".into(),
            description: "Minimal Alpine shell sandbox".into(),
            category: "Languages".into(),
            base_image: "alpine:3.20".into(),
            vcpus: 1,
            memory_mb: 256,
        },
        TemplateInfo {
            name: "c".into(),
            description: "GCC toolchain for C/C++ development".into(),
            category: "Languages".into(),
            base_image: "gcc:14-bookworm".into(),
            vcpus: 2,
            memory_mb: 512,
        },
        TemplateInfo {
            name: "dotnet".into(),
            description: ".NET SDK for C#/F# development".into(),
            category: "Languages".into(),
            base_image: "mcr.microsoft.com/dotnet/sdk:8.0".into(),
            vcpus: 2,
            memory_mb: 1024,
        },
        TemplateInfo {
            name: "go".into(),
            description: "Go toolchain for Go development".into(),
            category: "Languages".into(),
            base_image: "golang:1.23-alpine".into(),
            vcpus: 2,
            memory_mb: 512,
        },
        TemplateInfo {
            name: "java".into(),
            description: "Eclipse Temurin JDK for Java development".into(),
            category: "Languages".into(),
            base_image: "eclipse-temurin:21-alpine".into(),
            vcpus: 2,
            memory_mb: 1024,
        },
        TemplateInfo {
            name: "node".into(),
            description: "Node.js LTS for JavaScript development".into(),
            category: "Languages".into(),
            base_image: "node:22-alpine".into(),
            vcpus: 1,
            memory_mb: 512,
        },
        TemplateInfo {
            name: "python".into(),
            description: "Python with pip for general development".into(),
            category: "Languages".into(),
            base_image: "python:3.12-alpine".into(),
            vcpus: 1,
            memory_mb: 512,
        },
        TemplateInfo {
            name: "ruby".into(),
            description: "Ruby with Bundler for Ruby development".into(),
            category: "Languages".into(),
            base_image: "ruby:3.3-alpine".into(),
            vcpus: 1,
            memory_mb: 512,
        },
        TemplateInfo {
            name: "rust".into(),
            description: "Rust toolchain for Rust development".into(),
            category: "Languages".into(),
            base_image: "rust:1.85-alpine".into(),
            vcpus: 2,
            memory_mb: 512,
        },
        TemplateInfo {
            name: "typescript".into(),
            description: "Node.js LTS for TypeScript development".into(),
            category: "Languages".into(),
            base_image: "node:22-alpine".into(),
            vcpus: 1,
            memory_mb: 512,
        },
        // ----- Specialized -----
        TemplateInfo {
            name: "python-ml".into(),
            description: "Python for machine learning / data science".into(),
            category: "Specialized".into(),
            base_image: "python:3.12".into(),
            vcpus: 4,
            memory_mb: 4096,
        },
        TemplateInfo {
            name: "node-fullstack".into(),
            description: "Full-stack JavaScript/TypeScript development".into(),
            category: "Specialized".into(),
            base_image: "node:22-alpine".into(),
            vcpus: 2,
            memory_mb: 1024,
        },
        TemplateInfo {
            name: "rust-ci".into(),
            description: "Rust build and test CI workloads".into(),
            category: "Specialized".into(),
            base_image: "rust:1.85-alpine".into(),
            vcpus: 4,
            memory_mb: 2048,
        },
        TemplateInfo {
            name: "secure".into(),
            description: "Maximum isolation: no network, read-only".into(),
            category: "Specialized".into(),
            base_image: "alpine:3.20".into(),
            vcpus: 1,
            memory_mb: 256,
        },
        TemplateInfo {
            name: "vscode".into(),
            description: "Browser-based VS Code IDE (openvscode-server)".into(),
            category: "Specialized".into(),
            base_image: "gitpod/openvscode-server:latest".into(),
            vcpus: 2,
            memory_mb: 2048,
        },
        TemplateInfo {
            name: "coder".into(),
            description: "Browser-based VS Code IDE (code-server)".into(),
            category: "Specialized".into(),
            base_image: "codercom/code-server:latest".into(),
            vcpus: 2,
            memory_mb: 2048,
        },
        TemplateInfo {
            name: "gitea".into(),
            description: "Self-hosted Git service with web UI".into(),
            category: "Specialized".into(),
            base_image: "gitea/gitea:latest".into(),
            vcpus: 1,
            memory_mb: 512,
        },
    ]
}
