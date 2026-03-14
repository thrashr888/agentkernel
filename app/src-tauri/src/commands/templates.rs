use std::collections::BTreeMap;

use serde::Deserialize;

use crate::types::TemplateInfo;

// ---------------------------------------------------------------------------
// Lightweight TOML parse structs (avoids depending on the main crate's Config)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct TemplateToml {
    sandbox: SandboxSection,
    #[serde(default)]
    resources: ResourcesSection,
    #[serde(default)]
    secrets: BTreeMap<String, String>,
    #[serde(default)]
    ports: BTreeMap<String, u16>,
    #[serde(default)]
    template: TemplateSection,
}

#[derive(Deserialize)]
struct SandboxSection {
    name: String,
    #[serde(default)]
    base_image: Option<String>,
    #[serde(default)]
    init_script: Option<String>,
}

fn default_vcpus() -> u32 {
    1
}
fn default_memory_mb() -> u64 {
    512
}

#[derive(Deserialize)]
struct ResourcesSection {
    #[serde(default = "default_vcpus")]
    vcpus: u32,
    #[serde(default = "default_memory_mb")]
    memory_mb: u64,
}

impl Default for ResourcesSection {
    fn default() -> Self {
        Self {
            vcpus: default_vcpus(),
            memory_mb: default_memory_mb(),
        }
    }
}

#[derive(Deserialize, Default)]
struct TemplateSection {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    help_text: Option<String>,
    #[serde(default)]
    secret_files: Vec<String>,
}

// ---------------------------------------------------------------------------
// Embedded TOML sources (compiled into the binary)
// ---------------------------------------------------------------------------

const BUILTIN_TOML: &[&str] = &[
    // Agent Sandboxes
    include_str!("../../../../templates/claude-sandbox.toml"),
    include_str!("../../../../templates/codex-sandbox.toml"),
    include_str!("../../../../templates/copilot-sandbox.toml"),
    include_str!("../../../../templates/gemini-sandbox.toml"),
    include_str!("../../../../templates/opencode-sandbox.toml"),
    include_str!("../../../../templates/amp-sandbox.toml"),
    include_str!("../../../../templates/pi-sandbox.toml"),
    include_str!("../../../../templates/hermes-sandbox.toml"),
    include_str!("../../../../templates/symphony-sandbox.toml"),
    // Languages
    include_str!("../../../../templates/bash.toml"),
    include_str!("../../../../templates/c.toml"),
    include_str!("../../../../templates/dotnet.toml"),
    include_str!("../../../../templates/go.toml"),
    include_str!("../../../../templates/java.toml"),
    include_str!("../../../../templates/node.toml"),
    include_str!("../../../../templates/python.toml"),
    include_str!("../../../../templates/ruby.toml"),
    include_str!("../../../../templates/rust.toml"),
    include_str!("../../../../templates/typescript.toml"),
    // Browser Automation
    include_str!("../../../../templates/playwright.toml"),
    include_str!("../../../../templates/playwright-stealth.toml"),
    // Specialized
    include_str!("../../../../templates/python-ml.toml"),
    include_str!("../../../../templates/node-fullstack.toml"),
    include_str!("../../../../templates/rust-ci.toml"),
    include_str!("../../../../templates/secure.toml"),
    // Infrastructure
    include_str!("../../../../templates/terraform.toml"),
    // Datastores
    include_str!("../../../../templates/sqlite.toml"),
    include_str!("../../../../templates/postgres.toml"),
    include_str!("../../../../templates/mysql.toml"),
    include_str!("../../../../templates/redis.toml"),
    // Dev Tools
    include_str!("../../../../templates/vscode.toml"),
    include_str!("../../../../templates/coder.toml"),
    include_str!("../../../../templates/gitea.toml"),
    include_str!("../../../../templates/openclaw.toml"),
];

fn parse_template(toml_str: &str) -> Option<TemplateInfo> {
    let parsed: TemplateToml = toml::from_str(toml_str).ok()?;
    let ports = parsed
        .ports
        .iter()
        .map(|(host, container)| format!("{host}:{container}"))
        .collect();
    Some(TemplateInfo {
        name: parsed.sandbox.name,
        description: parsed.template.description.unwrap_or_default(),
        category: parsed.template.category.unwrap_or_else(|| "Other".into()),
        base_image: parsed.sandbox.base_image.unwrap_or_default(),
        vcpus: parsed.resources.vcpus,
        memory_mb: parsed.resources.memory_mb,
        init_script: parsed.sandbox.init_script,
        help_text: parsed.template.help_text,
        ports,
        secret_files: parsed.template.secret_files,
        secrets: parsed.secrets,
    })
}

fn builtin_templates() -> Vec<TemplateInfo> {
    BUILTIN_TOML
        .iter()
        .filter_map(|s| parse_template(s))
        .collect()
}

#[tauri::command(rename_all = "snake_case")]
pub async fn list_templates() -> Result<Vec<TemplateInfo>, String> {
    Ok(builtin_templates())
}
