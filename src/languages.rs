//! Language and runtime detection for agentkernel.
//!
//! This module handles auto-detection of the appropriate Docker image
//! based on project files or command being executed.

use std::path::Path;

/// Language/runtime definition
struct Runtime {
    /// Docker image to use
    image: &'static str,
    /// Project files that indicate this runtime
    project_files: &'static [&'static str],
    /// Commands that indicate this runtime
    commands: &'static [&'static str],
}

/// All supported runtimes
const RUNTIMES: &[Runtime] = &[
    // Node.js / JavaScript / TypeScript
    Runtime {
        image: "node:22-alpine",
        project_files: &[
            "package.json",
            "package-lock.json",
            "yarn.lock",
            "pnpm-lock.yaml",
            "bun.lockb",
        ],
        commands: &[
            "node", "npm", "npx", "yarn", "pnpm", "bun", "tsx", "ts-node",
        ],
    },
    // Rust
    Runtime {
        image: "rust:1.85-alpine",
        project_files: &["Cargo.toml", "Cargo.lock"],
        commands: &["cargo", "rustc", "rustup", "rustfmt", "clippy"],
    },
    // Go
    Runtime {
        image: "golang:1.23-alpine",
        project_files: &["go.mod", "go.sum"],
        commands: &["go", "gofmt"],
    },
    // Python
    Runtime {
        image: "python:3.12-alpine",
        project_files: &[
            "pyproject.toml",
            "requirements.txt",
            "setup.py",
            "setup.cfg",
            "Pipfile",
            "poetry.lock",
            "uv.lock",
        ],
        commands: &[
            "python", "python3", "pip", "pip3", "poetry", "uv", "pytest", "ruff",
        ],
    },
    // Ruby
    Runtime {
        image: "ruby:3.3-alpine",
        project_files: &["Gemfile", "Gemfile.lock", "*.gemspec"],
        commands: &["ruby", "gem", "bundle", "bundler", "rake", "rails"],
    },
    // Java
    Runtime {
        image: "eclipse-temurin:21-alpine",
        project_files: &[
            "pom.xml",
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
        ],
        commands: &["java", "javac", "mvn", "gradle", "gradlew"],
    },
    // Kotlin (JVM) - uses JDK image, most Kotlin projects use Gradle
    Runtime {
        image: "eclipse-temurin:21-alpine",
        project_files: &["*.kt"],
        commands: &[],
    },
    // .NET / C#
    Runtime {
        image: "mcr.microsoft.com/dotnet/sdk:8.0",
        project_files: &["*.csproj", "*.fsproj", "*.sln", "global.json"],
        commands: &["dotnet", "csc", "fsc"],
    },
    // C/C++
    Runtime {
        image: "gcc:14-bookworm",
        project_files: &[
            "Makefile",
            "CMakeLists.txt",
            "configure",
            "*.c",
            "*.cpp",
            "*.h",
        ],
        commands: &["gcc", "g++", "clang", "clang++", "make", "cmake", "cc"],
    },
    // PHP
    Runtime {
        image: "php:8.3-alpine",
        project_files: &["composer.json", "composer.lock", "*.php"],
        commands: &["php", "composer"],
    },
    // Elixir
    Runtime {
        image: "elixir:1.16-alpine",
        project_files: &["mix.exs", "mix.lock"],
        commands: &["elixir", "mix", "iex"],
    },
    // Shell scripts (uses lightweight alpine)
    Runtime {
        image: "alpine:3.20",
        project_files: &["*.sh"],
        commands: &["sh", "bash", "zsh", "ash"],
    },
    // Lua
    Runtime {
        image: "nickblah/lua:5.4-alpine",
        project_files: &["*.lua", ".luacheckrc"],
        commands: &["lua", "luajit", "luarocks"],
    },
    // HCL / Terraform
    Runtime {
        image: "hashicorp/terraform:1.10",
        project_files: &["*.tf", "*.tfvars", "terraform.tfstate"],
        commands: &["terraform"],
    },
];

/// Default image when nothing is detected
const DEFAULT_IMAGE: &str = "alpine:3.20";

/// Common Dockerfile names to detect
const DOCKERFILE_NAMES: &[&str] = &[
    "Dockerfile",
    "dockerfile",
    "Dockerfile.dev",
    "Dockerfile.development",
    "Dockerfile.prod",
    "Dockerfile.production",
];

/// Detect Docker image from a Procfile by parsing its commands
/// Procfile format: `process_type: command`
fn detect_from_procfile(dir: &Path) -> Option<String> {
    let procfile_path = dir.join("Procfile");
    let content = std::fs::read_to_string(procfile_path).ok()?;

    // Parse each line and look for known commands
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Extract command part after the colon
        if let Some((_process_type, command)) = line.split_once(':') {
            let command = command.trim();
            // Extract first word as the command
            let cmd = command.split_whitespace().next()?;

            // Check if it matches any known runtime command (exact match)
            for runtime in RUNTIMES {
                if runtime.commands.contains(&cmd) {
                    return Some(runtime.image.to_string());
                }
            }
        }
    }
    None
}

/// Detect if a Dockerfile exists in the given directory
///
/// Returns the path to the Dockerfile if found, preferring exact "Dockerfile" name.
/// Checks common variations like Dockerfile.dev, Dockerfile.prod, etc.
pub fn detect_dockerfile(dir: &Path) -> Option<std::path::PathBuf> {
    for name in DOCKERFILE_NAMES {
        let path = dir.join(name);
        if path.exists() && path.is_file() {
            return Some(path);
        }
    }
    None
}

/// Calculate a content hash for a Dockerfile (for caching)
///
/// Uses the first 12 characters of a SHA256-like hash of the content.
pub fn dockerfile_content_hash(dockerfile_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(dockerfile_path).ok()?;
    // Simple hash: sum of bytes mod large prime, formatted as hex
    let hash: u64 = content
        .bytes()
        .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
    // Truncate to 12 hex characters for shorter, readable image tags
    let full_hash = format!("{:016x}", hash);
    Some(full_hash[..12].to_string())
}

/// Generate a deterministic image name for a built Dockerfile
///
/// Format: agentkernel-{project}:{content-hash}
pub fn dockerfile_image_name(project_name: &str, dockerfile_path: &Path) -> String {
    let hash = dockerfile_content_hash(dockerfile_path).unwrap_or_else(|| "unknown".to_string());
    // Sanitize project name for Docker image naming
    let safe_name: String = project_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    format!("agentkernel-{}:{}", safe_name, hash)
}

/// Detect Docker image based on project files in the given directory
pub fn detect_from_project(dir: &Path) -> Option<String> {
    for runtime in RUNTIMES {
        for pattern in runtime.project_files {
            if let Some(suffix) = pattern.strip_prefix('*') {
                // Glob pattern - check if any file matches the suffix
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        if entry.file_name().to_string_lossy().ends_with(suffix) {
                            return Some(runtime.image.to_string());
                        }
                    }
                }
            } else if dir.join(pattern).exists() {
                return Some(runtime.image.to_string());
            }
        }
    }
    None
}

/// Detect Docker image based on the command being executed
pub fn detect_from_command(command: &[String]) -> Option<String> {
    let cmd = command.first()?;

    // Extract the base command name (handle paths like /usr/bin/python)
    let base_cmd = Path::new(cmd)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| cmd.clone());

    for runtime in RUNTIMES {
        if runtime.commands.contains(&base_cmd.as_str()) {
            return Some(runtime.image.to_string());
        }
    }
    None
}

/// Detect Docker image using all available methods
/// Priority: project files > Procfile > command > default
pub fn detect_image(command: &[String]) -> String {
    let current_dir = Path::new(".");

    // Try project files in current directory first
    if let Some(image) = detect_from_project(current_dir) {
        return image;
    }

    // Try Procfile
    if let Some(image) = detect_from_procfile(current_dir) {
        return image;
    }

    // Try command-based detection
    if let Some(image) = detect_from_command(command) {
        return image;
    }

    // Fall back to default
    DEFAULT_IMAGE.to_string()
}

/// Map a Docker image name to a Firecracker rootfs runtime name
///
/// Firecracker uses pre-built rootfs images with specific runtimes,
/// while Docker uses standard container images. This function maps between them.
pub fn docker_image_to_firecracker_runtime(image: &str) -> &'static str {
    // Map based on image prefix
    if image.starts_with("python:") || image.starts_with("python") {
        "python"
    } else if image.starts_with("node:") || image.starts_with("node") {
        "node"
    } else if image.starts_with("golang:") || image.starts_with("go:") || image.starts_with("go") {
        "go"
    } else if image.starts_with("rust:") || image.starts_with("rust") {
        "rust"
    } else if image.starts_with("ruby:") || image.starts_with("ruby") {
        "ruby"
    } else if image.starts_with("eclipse-temurin:") || image.starts_with("openjdk:") {
        "java"
    } else if image.starts_with("gcc:") || image.starts_with("g++:") {
        "c"
    } else if image.contains("dotnet") {
        "dotnet"
    } else {
        // Default to base Alpine for anything else
        "base"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_from_command() {
        assert_eq!(
            detect_from_command(&["npm".to_string(), "test".to_string()]),
            Some("node:22-alpine".to_string())
        );
        assert_eq!(
            detect_from_command(&["cargo".to_string(), "build".to_string()]),
            Some("rust:1.85-alpine".to_string())
        );
        assert_eq!(
            detect_from_command(&[
                "python3".to_string(),
                "-c".to_string(),
                "print(1)".to_string()
            ]),
            Some("python:3.12-alpine".to_string())
        );
        assert_eq!(detect_from_command(&["unknown-command".to_string()]), None);
    }

    #[test]
    fn test_detect_image_fallback() {
        // Unknown command in a directory without project files should return default
        // (This test assumes we're not in a project directory)
        let result = detect_from_command(&["some-random-command".to_string()]);
        assert_eq!(result, None);
    }

    #[test]
    fn test_detect_shell_commands() {
        assert_eq!(
            detect_from_command(&["bash".to_string(), "-c".to_string(), "echo hi".to_string()]),
            Some("alpine:3.20".to_string())
        );
        assert_eq!(
            detect_from_command(&["sh".to_string(), "script.sh".to_string()]),
            Some("alpine:3.20".to_string())
        );
    }

    #[test]
    fn test_detect_from_procfile() {
        use std::io::Write;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();

        // Create a Procfile with Ruby command
        let procfile_path = dir.path().join("Procfile");
        let mut file = std::fs::File::create(&procfile_path).unwrap();
        writeln!(file, "web: bundle exec rails server -p $PORT").unwrap();
        writeln!(file, "worker: rake jobs:work").unwrap();

        let result = detect_from_procfile(dir.path());
        assert_eq!(result, Some("ruby:3.3-alpine".to_string()));
    }

    #[test]
    fn test_detect_from_procfile_python() {
        use std::io::Write;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();

        let procfile_path = dir.path().join("Procfile");
        let mut file = std::fs::File::create(&procfile_path).unwrap();
        writeln!(file, "web: python manage.py runserver").unwrap();

        let result = detect_from_procfile(dir.path());
        assert_eq!(result, Some("python:3.12-alpine".to_string()));
    }

    #[test]
    fn test_detect_dockerfile() {
        use std::io::Write;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();

        // No Dockerfile initially
        assert!(detect_dockerfile(dir.path()).is_none());

        // Create a Dockerfile
        let dockerfile_path = dir.path().join("Dockerfile");
        let mut file = std::fs::File::create(&dockerfile_path).unwrap();
        writeln!(file, "FROM alpine:3.20").unwrap();
        writeln!(file, "RUN apk add --no-cache python3").unwrap();

        let result = detect_dockerfile(dir.path());
        assert!(result.is_some());
        assert_eq!(result.unwrap().file_name().unwrap(), "Dockerfile");
    }

    #[test]
    fn test_dockerfile_content_hash() {
        use std::io::Write;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let dockerfile_path = dir.path().join("Dockerfile");

        // Create a Dockerfile
        let mut file = std::fs::File::create(&dockerfile_path).unwrap();
        writeln!(file, "FROM alpine:3.20").unwrap();

        let hash = dockerfile_content_hash(&dockerfile_path);
        assert!(hash.is_some());
        assert_eq!(hash.as_ref().unwrap().len(), 12);

        // Same content should produce same hash
        let hash2 = dockerfile_content_hash(&dockerfile_path);
        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_dockerfile_image_name() {
        use std::io::Write;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let dockerfile_path = dir.path().join("Dockerfile");

        let mut file = std::fs::File::create(&dockerfile_path).unwrap();
        writeln!(file, "FROM alpine:3.20").unwrap();

        let name = dockerfile_image_name("my-project", &dockerfile_path);
        assert!(name.starts_with("agentkernel-my-project:"));
        assert_eq!(name.split(':').count(), 2);
    }
}
