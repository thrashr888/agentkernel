use std::collections::BTreeMap;

use crate::types::TemplateInfo;

/// Return the hardcoded list of built-in templates.
///
/// The data here mirrors `BUILTIN_TEMPLATES` in `src/template.rs` so the
/// desktop app does not need to link against the main agentkernel crate.
#[tauri::command(rename_all = "snake_case")]
pub async fn list_templates() -> Result<Vec<TemplateInfo>, String> {
    Ok(builtin_templates())
}

/// Helper: build a BTreeMap from pairs.
fn secrets(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn builtin_templates() -> Vec<TemplateInfo> {
    let mut templates = vec![
        // ----- Agent Sandboxes -----
        TemplateInfo {
            name: "claude-sandbox".into(),
            description: "Claude Code agent sandbox".into(),
            category: "Agent Sandboxes".into(),
            base_image: "node:22-alpine".into(),
            vcpus: 2,
            memory_mb: 1024,
            init_script: None,
            help_text: None,
            secrets: secrets(&[("ANTHROPIC_API_KEY", "api.anthropic.com")]),
        },
        TemplateInfo {
            name: "codex-sandbox".into(),
            description: "OpenAI Codex agent sandbox".into(),
            category: "Agent Sandboxes".into(),
            base_image: "node:22-alpine".into(),
            vcpus: 2,
            memory_mb: 1024,
            init_script: None,
            help_text: None,
            secrets: secrets(&[("OPENAI_API_KEY", "api.openai.com")]),
        },
        TemplateInfo {
            name: "gemini-sandbox".into(),
            description: "Gemini CLI agent sandbox".into(),
            category: "Agent Sandboxes".into(),
            base_image: "node:22-alpine".into(),
            vcpus: 2,
            memory_mb: 1024,
            init_script: None,
            help_text: None,
            secrets: secrets(&[
                ("GOOGLE_API_KEY", "generativelanguage.googleapis.com"),
                ("GEMINI_API_KEY", "generativelanguage.googleapis.com"),
            ]),
        },
        TemplateInfo {
            name: "opencode-sandbox".into(),
            description: "OpenCode agent sandbox".into(),
            category: "Agent Sandboxes".into(),
            base_image: "node:22-alpine".into(),
            vcpus: 2,
            memory_mb: 1024,
            init_script: None,
            help_text: None,
            secrets: secrets(&[
                ("ANTHROPIC_API_KEY", "api.anthropic.com"),
                ("OPENAI_API_KEY", "api.openai.com"),
            ]),
        },
        TemplateInfo {
            name: "amp-sandbox".into(),
            description: "Amp (Sourcegraph) agent sandbox".into(),
            category: "Agent Sandboxes".into(),
            base_image: "node:22-alpine".into(),
            vcpus: 2,
            memory_mb: 1024,
            init_script: None,
            help_text: None,
            secrets: secrets(&[("ANTHROPIC_API_KEY", "api.anthropic.com")]),
        },
        TemplateInfo {
            name: "pi-sandbox".into(),
            description: "Pi coding agent sandbox".into(),
            category: "Agent Sandboxes".into(),
            base_image: "node:22-alpine".into(),
            vcpus: 2,
            memory_mb: 1024,
            init_script: None,
            help_text: None,
            secrets: secrets(&[
                ("ANTHROPIC_API_KEY", "api.anthropic.com"),
                ("OPENAI_API_KEY", "api.openai.com"),
            ]),
        },
        // ----- Infrastructure / Cloud -----
        TemplateInfo {
            name: "terraform".into(),
            description: "Terraform with AWS, Azure, GCP CLIs and hcptf-cli".into(),
            category: "Infrastructure".into(),
            base_image: "python:3.12-slim".into(),
            vcpus: 2,
            memory_mb: 2048,
            init_script: Some(concat!(
                "set -e\n",
                "ARCH=$(uname -m)\n",
                "case \"$ARCH\" in\n",
                "  x86_64)  TF_ARCH=amd64; AWS_ARCH=x86_64; GCLOUD_ARCH=x86_64 ;;\n",
                "  aarch64|arm64) TF_ARCH=arm64; AWS_ARCH=aarch64; GCLOUD_ARCH=arm ;;\n",
                "  *) echo \"Unsupported architecture: $ARCH\" && exit 1 ;;\n",
                "esac\n",
                "apt-get update -qq && apt-get install -y -qq curl unzip gnupg lsb-release >/dev/null 2>&1\n",
                "curl -fsSL \"https://releases.hashicorp.com/terraform/1.14.5/terraform_1.14.5_linux_${TF_ARCH}.zip\" -o /tmp/tf.zip ",
                "&& unzip -qo /tmp/tf.zip -d /usr/local/bin && rm /tmp/tf.zip\n",
                "curl -fsSL \"https://github.com/thrashr888/hcptf-cli/releases/download/v0.3.1/hcptf-cli_0.3.1_linux_${TF_ARCH}.tar.gz\" ",
                "| tar -xz -C /usr/local/bin hcptf\n",
                "curl -fsSL \"https://awscli.amazonaws.com/awscli-exe-linux-${AWS_ARCH}.zip\" -o /tmp/awscli.zip ",
                "&& unzip -qo /tmp/awscli.zip -d /tmp && /tmp/aws/install --update >/dev/null 2>&1 && rm -rf /tmp/aws /tmp/awscli.zip\n",
                "curl -fsSL https://aka.ms/InstallAzureCLIDeb | bash >/dev/null 2>&1\n",
                "curl -fsSL \"https://dl.google.com/dl/cloudsdk/channels/rapid/downloads/google-cloud-cli-linux-${GCLOUD_ARCH}.tar.gz\" ",
                "| tar -xz -C /opt && /opt/google-cloud-sdk/install.sh --quiet --path-update=true >/dev/null 2>&1 ",
                "&& ln -sf /opt/google-cloud-sdk/bin/gcloud /usr/local/bin/gcloud ",
                "&& ln -sf /opt/google-cloud-sdk/bin/gsutil /usr/local/bin/gsutil\n",
            ).into()),
            help_text: None,
            secrets: secrets(&[
                ("TFE_TOKEN", "app.terraform.io"),
                ("HCP_CLIENT_ID", "api.hashicorp.cloud"),
                ("HCP_CLIENT_SECRET", "api.hashicorp.cloud"),
                ("AWS_ACCESS_KEY_ID", "sts.amazonaws.com"),
                ("AWS_SECRET_ACCESS_KEY", "sts.amazonaws.com"),
                ("AWS_SESSION_TOKEN", "sts.amazonaws.com"),
                ("AZURE_CLIENT_ID", "login.microsoftonline.com"),
                ("AZURE_CLIENT_SECRET", "login.microsoftonline.com"),
                ("AZURE_TENANT_ID", "login.microsoftonline.com"),
                ("GOOGLE_APPLICATION_CREDENTIALS", "oauth2.googleapis.com"),
            ]),
        },
        // ----- Datastores -----
        TemplateInfo {
            name: "sqlite".into(),
            description: "SQLite tooling for embedded durable state".into(),
            category: "Datastores".into(),
            base_image: "alpine:3.20".into(),
            vcpus: 1,
            memory_mb: 512,
            init_script: Some(
                concat!(
                    "set -e\n",
                    "apk add --no-cache sqlite\n",
                    "mkdir -p /workspace/data\n",
                )
                .into(),
            ),
            help_text: None,
            secrets: BTreeMap::new(),
        },
        TemplateInfo {
            name: "postgres".into(),
            description: "Postgres server image for local development".into(),
            category: "Datastores".into(),
            base_image: "postgres:17-alpine".into(),
            vcpus: 2,
            memory_mb: 1024,
            init_script: None,
            help_text: None,
            secrets: BTreeMap::new(),
        },
        TemplateInfo {
            name: "mysql".into(),
            description: "MySQL server image for local development".into(),
            category: "Datastores".into(),
            base_image: "mysql:8.4".into(),
            vcpus: 2,
            memory_mb: 1024,
            init_script: None,
            help_text: None,
            secrets: BTreeMap::new(),
        },
        TemplateInfo {
            name: "redis".into(),
            description: "Redis server image for caching and queues".into(),
            category: "Datastores".into(),
            base_image: "redis:7-alpine".into(),
            vcpus: 1,
            memory_mb: 512,
            init_script: None,
            help_text: None,
            secrets: BTreeMap::new(),
        },
        // ----- Languages -----
        TemplateInfo {
            name: "bash".into(),
            description: "Minimal Alpine shell sandbox".into(),
            category: "Languages".into(),
            base_image: "alpine:3.20".into(),
            vcpus: 1,
            memory_mb: 256,
            init_script: None,
            help_text: None,
            secrets: BTreeMap::new(),
        },
        TemplateInfo {
            name: "c".into(),
            description: "GCC toolchain for C/C++ development".into(),
            category: "Languages".into(),
            base_image: "gcc:14-bookworm".into(),
            vcpus: 2,
            memory_mb: 512,
            init_script: None,
            help_text: None,
            secrets: BTreeMap::new(),
        },
        TemplateInfo {
            name: "dotnet".into(),
            description: ".NET SDK for C#/F# development".into(),
            category: "Languages".into(),
            base_image: "mcr.microsoft.com/dotnet/sdk:8.0".into(),
            vcpus: 2,
            memory_mb: 1024,
            init_script: None,
            help_text: None,
            secrets: BTreeMap::new(),
        },
        TemplateInfo {
            name: "go".into(),
            description: "Go toolchain for Go development".into(),
            category: "Languages".into(),
            base_image: "golang:1.23-alpine".into(),
            vcpus: 2,
            memory_mb: 512,
            init_script: None,
            help_text: None,
            secrets: BTreeMap::new(),
        },
        TemplateInfo {
            name: "java".into(),
            description: "Eclipse Temurin JDK for Java development".into(),
            category: "Languages".into(),
            base_image: "eclipse-temurin:21-alpine".into(),
            vcpus: 2,
            memory_mb: 1024,
            init_script: None,
            help_text: None,
            secrets: BTreeMap::new(),
        },
        TemplateInfo {
            name: "node".into(),
            description: "Node.js LTS for JavaScript development".into(),
            category: "Languages".into(),
            base_image: "node:22-alpine".into(),
            vcpus: 1,
            memory_mb: 512,
            init_script: None,
            help_text: None,
            secrets: BTreeMap::new(),
        },
        TemplateInfo {
            name: "python".into(),
            description: "Python with pip for general development".into(),
            category: "Languages".into(),
            base_image: "python:3.12-alpine".into(),
            vcpus: 1,
            memory_mb: 512,
            init_script: None,
            help_text: None,
            secrets: BTreeMap::new(),
        },
        TemplateInfo {
            name: "ruby".into(),
            description: "Ruby with Bundler for Ruby development".into(),
            category: "Languages".into(),
            base_image: "ruby:3.3-alpine".into(),
            vcpus: 1,
            memory_mb: 512,
            init_script: None,
            help_text: None,
            secrets: BTreeMap::new(),
        },
        TemplateInfo {
            name: "rust".into(),
            description: "Rust toolchain for Rust development".into(),
            category: "Languages".into(),
            base_image: "rust:1.85-alpine".into(),
            vcpus: 2,
            memory_mb: 512,
            init_script: None,
            help_text: None,
            secrets: BTreeMap::new(),
        },
        TemplateInfo {
            name: "typescript".into(),
            description: "Node.js LTS for TypeScript development".into(),
            category: "Languages".into(),
            base_image: "node:22-alpine".into(),
            vcpus: 1,
            memory_mb: 512,
            init_script: None,
            help_text: None,
            secrets: BTreeMap::new(),
        },
        // ----- Browser Automation -----
        TemplateInfo {
            name: "playwright".into(),
            description: "Browser automation with Playwright (Python)".into(),
            category: "Browser Automation".into(),
            base_image: "python:3.12-slim".into(),
            vcpus: 2,
            memory_mb: 2048,
            init_script: None,
            help_text: None,
            secrets: BTreeMap::new(),
        },
        TemplateInfo {
            name: "playwright-stealth".into(),
            description: "Stealth browser automation that avoids bot detection".into(),
            category: "Browser Automation".into(),
            base_image: "python:3.12-slim".into(),
            vcpus: 2,
            memory_mb: 2048,
            init_script: None,
            help_text: None,
            secrets: BTreeMap::new(),
        },
        // ----- Specialized -----
        TemplateInfo {
            name: "python-ml".into(),
            description: "Python for machine learning / data science".into(),
            category: "Specialized".into(),
            base_image: "python:3.12".into(),
            vcpus: 4,
            memory_mb: 4096,
            init_script: None,
            help_text: None,
            secrets: BTreeMap::new(),
        },
        TemplateInfo {
            name: "node-fullstack".into(),
            description: "Full-stack JavaScript/TypeScript development".into(),
            category: "Specialized".into(),
            base_image: "node:22-alpine".into(),
            vcpus: 2,
            memory_mb: 1024,
            init_script: None,
            help_text: None,
            secrets: BTreeMap::new(),
        },
        TemplateInfo {
            name: "rust-ci".into(),
            description: "Rust build and test CI workloads".into(),
            category: "Specialized".into(),
            base_image: "rust:1.85-alpine".into(),
            vcpus: 4,
            memory_mb: 2048,
            init_script: None,
            help_text: None,
            secrets: BTreeMap::new(),
        },
        TemplateInfo {
            name: "secure".into(),
            description: "Maximum isolation: no network, read-only".into(),
            category: "Specialized".into(),
            base_image: "alpine:3.20".into(),
            vcpus: 1,
            memory_mb: 256,
            init_script: None,
            help_text: None,
            secrets: BTreeMap::new(),
        },
        TemplateInfo {
            name: "vscode".into(),
            description: "Browser-based VS Code IDE (openvscode-server)".into(),
            category: "Specialized".into(),
            base_image: "gitpod/openvscode-server:latest".into(),
            vcpus: 2,
            memory_mb: 2048,
            init_script: None,
            help_text: None,
            secrets: BTreeMap::new(),
        },
        TemplateInfo {
            name: "coder".into(),
            description: "Browser-based VS Code IDE (code-server)".into(),
            category: "Specialized".into(),
            base_image: "codercom/code-server:latest".into(),
            vcpus: 2,
            memory_mb: 2048,
            init_script: None,
            help_text: None,
            secrets: BTreeMap::new(),
        },
        TemplateInfo {
            name: "gitea".into(),
            description: "Self-hosted Git service with web UI".into(),
            category: "Specialized".into(),
            base_image: "gitea/gitea:latest".into(),
            vcpus: 1,
            memory_mb: 512,
            init_script: None,
            help_text: None,
            secrets: BTreeMap::new(),
        },
    ];

    for template in &mut templates {
        template.help_text = Some(default_help_text(template));
    }

    templates
}

fn default_help_text(template: &TemplateInfo) -> String {
    let usage = usage_for_template(&template.name);
    let binaries = binaries_for_template(&template.name);
    let services_ports = services_ports_for_template(&template.name);
    format!(
        "Description: {}\n\nHow to use: {}\n\nBinaries available: {}\n\nServices and ports: {}",
        template.description, usage, binaries, services_ports
    )
}

fn usage_for_template(name: &str) -> &'static str {
    match name {
        "sqlite" => {
            "Start the sandbox, then run `sqlite /workspace/data/app.db` to create/query a local database."
        }
        "postgres" => {
            "Start the sandbox, then run `psql -h 127.0.0.1 -U app -d app` from inside the sandbox. Note: use `psql`, not `pgsql`."
        }
        "mysql" => {
            "Start the sandbox, then run `mysql -h 127.0.0.1 -u app -p app` from inside the sandbox."
        }
        "redis" => {
            "Start the sandbox, then run `redis-cli -h 127.0.0.1 -p 6379 ping` from inside the sandbox."
        }
        "playwright" | "playwright-stealth" => {
            "Install Python deps in your project, then run Playwright scripts from /workspace."
        }
        "vscode" | "coder" => {
            "Start the sandbox and open the mapped web port in your browser to use the IDE."
        }
        "gitea" => {
            "Start the sandbox and open the mapped web port in your browser to access the Git UI."
        }
        _ => "Start the sandbox, attach with `agentkernel attach <name>`, and run commands in /workspace.",
    }
}

fn binaries_for_template(name: &str) -> &'static str {
    match name {
        "bash" => "sh, ash, busybox",
        "c" => "gcc, g++, make",
        "dotnet" => "dotnet",
        "go" => "go",
        "java" => "java, javac",
        "node" | "node-fullstack" | "typescript" => "node, npm, npx",
        "python" | "python-ml" | "playwright" | "playwright-stealth" | "terraform" => {
            "python, pip"
        }
        "ruby" => "ruby, gem, bundle",
        "rust" | "rust-ci" => "rustc, cargo",
        "sqlite" => "sqlite",
        "postgres" => "postgres, psql, pg_isready",
        "mysql" => "mysql, mysqld",
        "redis" => "redis-server, redis-cli",
        "vscode" => "openvscode-server",
        "coder" => "code-server",
        "gitea" => "gitea",
        "claude-sandbox" | "codex-sandbox" | "gemini-sandbox" | "opencode-sandbox"
        | "amp-sandbox" | "pi-sandbox" => "node, npm, npx",
        "secure" => "sh, busybox",
        _ => "standard binaries from the base image",
    }
}

fn services_ports_for_template(name: &str) -> &'static str {
    match name {
        "postgres" => "PostgreSQL server on 5432/tcp.",
        "mysql" => "MySQL server on 3306/tcp.",
        "redis" => "Redis server on 6379/tcp.",
        "vscode" => "OpenVSCode web UI on 3000/tcp.",
        "coder" => "code-server web UI on 8080/tcp (unless changed by image defaults).",
        "gitea" => "Gitea web UI and SSH are image-configurable; map ports as needed.",
        _ => "No long-running service is configured by default. Only explicitly mapped ports are exposed.",
    }
}
