//! Template system for reusable sandbox configurations.
//!
//! Templates are resolved in order:
//! 1. Local saved templates in `~/.local/share/agentkernel/templates/`
//! 2. Built-in templates bundled in the binary
//! 3. `github:` prefix → fetch from GitHub raw, cache locally
//! 4. File path (`.toml` file)

use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::setup::default_data_dir;

/// Built-in templates bundled at compile time
const BUILTIN_TEMPLATES: &[(&str, &str)] = &[
    // Agent-specific sandboxes
    (
        "claude-sandbox",
        include_str!("../templates/claude-sandbox.toml"),
    ),
    (
        "codex-sandbox",
        include_str!("../templates/codex-sandbox.toml"),
    ),
    (
        "gemini-sandbox",
        include_str!("../templates/gemini-sandbox.toml"),
    ),
    (
        "opencode-sandbox",
        include_str!("../templates/opencode-sandbox.toml"),
    ),
    (
        "amp-sandbox",
        include_str!("../templates/amp-sandbox.toml"),
    ),
    (
        "pi-sandbox",
        include_str!("../templates/pi-sandbox.toml"),
    ),
    // Language runtimes
    ("bash", include_str!("../templates/bash.toml")),
    ("c", include_str!("../templates/c.toml")),
    ("dotnet", include_str!("../templates/dotnet.toml")),
    ("go", include_str!("../templates/go.toml")),
    ("java", include_str!("../templates/java.toml")),
    ("node", include_str!("../templates/node.toml")),
    ("python", include_str!("../templates/python.toml")),
    ("ruby", include_str!("../templates/ruby.toml")),
    ("rust", include_str!("../templates/rust.toml")),
    ("typescript", include_str!("../templates/typescript.toml")),
    // Specialized
    ("python-ml", include_str!("../templates/python-ml.toml")),
    (
        "node-fullstack",
        include_str!("../templates/node-fullstack.toml"),
    ),
    ("rust-ci", include_str!("../templates/rust-ci.toml")),
    ("secure", include_str!("../templates/secure.toml")),
    // Dev tools
    ("vscode", include_str!("../templates/vscode.toml")),
    ("coder", include_str!("../templates/coder.toml")),
    ("gitea", include_str!("../templates/gitea.toml")),
];

/// Where a template was found
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateSource {
    BuiltIn,
    Local,
    GitHub,
    FilePath,
}

impl std::fmt::Display for TemplateSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BuiltIn => write!(f, "built-in"),
            Self::Local => write!(f, "local"),
            Self::GitHub => write!(f, "github"),
            Self::FilePath => write!(f, "file"),
        }
    }
}

/// A resolved template
pub struct ResolvedTemplate {
    pub name: String,
    pub source: TemplateSource,
    pub content: String,
}

impl ResolvedTemplate {
    /// Parse the template TOML into a Config
    pub fn parse(&self) -> Result<Config> {
        Config::from_str(&self.content)
            .with_context(|| format!("failed to parse template '{}'", self.name))
    }
}

/// Directory for user-saved templates
fn local_templates_dir() -> PathBuf {
    default_data_dir().join("templates")
}

/// Resolve a template by name or specifier.
///
/// Handles: built-in names, local saved names, `github:owner/repo/path`, file paths.
pub fn resolve(specifier: &str) -> Result<ResolvedTemplate> {
    // 1. File path (contains / or .toml extension, and not github:)
    if !specifier.starts_with("github:")
        && (specifier.contains('/') || specifier.ends_with(".toml"))
    {
        let path = Path::new(specifier);
        if path.exists() {
            let content = fs::read_to_string(path)
                .with_context(|| format!("failed to read template file: {}", specifier))?;
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| specifier.to_string());
            return Ok(ResolvedTemplate {
                name,
                source: TemplateSource::FilePath,
                content,
            });
        }
        // Fall through to name-based resolution if it doesn't look like a path
        if specifier.contains('/') && !specifier.starts_with('.') {
            bail!("template file not found: {}", specifier);
        }
    }

    // 2. Local saved templates
    let local_dir = local_templates_dir();
    let local_path = local_dir.join(format!("{}.toml", specifier));
    if local_path.exists() {
        let content = fs::read_to_string(&local_path)?;
        return Ok(ResolvedTemplate {
            name: specifier.to_string(),
            source: TemplateSource::Local,
            content,
        });
    }

    // 3. Built-in templates
    for (name, content) in BUILTIN_TEMPLATES {
        if *name == specifier {
            return Ok(ResolvedTemplate {
                name: name.to_string(),
                source: TemplateSource::BuiltIn,
                content: content.to_string(),
            });
        }
    }

    // 4. GitHub shorthand: github:owner/repo/path
    if let Some(gh_path) = specifier.strip_prefix("github:") {
        return resolve_github(gh_path);
    }

    bail!(
        "template '{}' not found (checked local, built-in, and file paths)",
        specifier
    )
}

/// Resolve a github: template specifier.
///
/// Format: `github:owner/repo/path` → fetches from raw.githubusercontent.com
fn resolve_github(gh_path: &str) -> Result<ResolvedTemplate> {
    let parts: Vec<&str> = gh_path.splitn(3, '/').collect();
    if parts.len() < 3 {
        bail!(
            "github: template requires format 'github:owner/repo/path', got 'github:{}'",
            gh_path
        );
    }
    let (owner, repo, path) = (parts[0], parts[1], parts[2]);

    // Check local cache first
    let cache_dir = local_templates_dir().join("github-cache");
    let cache_key = gh_path.replace('/', "--");
    let cache_path = cache_dir.join(format!("{}.toml", cache_key));
    if cache_path.exists() {
        let content = fs::read_to_string(&cache_path)?;
        let name = path
            .strip_suffix(".toml")
            .unwrap_or(path)
            .rsplit('/')
            .next()
            .unwrap_or(path)
            .to_string();
        return Ok(ResolvedTemplate {
            name,
            source: TemplateSource::GitHub,
            content,
        });
    }

    // Fetch from GitHub raw
    let url = if path.ends_with(".toml") {
        format!(
            "https://raw.githubusercontent.com/{}/{}/main/{}",
            owner, repo, path
        )
    } else {
        format!(
            "https://raw.githubusercontent.com/{}/{}/main/{}.toml",
            owner, repo, path
        )
    };

    // Use curl for fetching (available on all platforms)
    let output = std::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "10", &url])
        .output()
        .context("failed to fetch template from GitHub (is curl installed?)")?;

    if !output.status.success() {
        bail!(
            "failed to fetch template from GitHub: {}\nURL: {}",
            String::from_utf8_lossy(&output.stderr).trim(),
            url
        );
    }

    let content =
        String::from_utf8(output.stdout).context("GitHub template response was not valid UTF-8")?;

    // Validate it parses before caching
    Config::from_str(&content).with_context(|| {
        format!(
            "fetched template from GitHub is not valid TOML config: {}",
            url
        )
    })?;

    // Cache locally
    fs::create_dir_all(&cache_dir)?;
    fs::write(&cache_path, &content)?;

    let name = path
        .strip_suffix(".toml")
        .unwrap_or(path)
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_string();

    Ok(ResolvedTemplate {
        name,
        source: TemplateSource::GitHub,
        content,
    })
}

/// A template listing entry
pub struct TemplateInfo {
    pub name: String,
    pub source: TemplateSource,
    pub image: String,
    pub profile: String,
    pub memory_mb: u64,
}

/// List all available templates (local + built-in)
pub fn list_all() -> Vec<TemplateInfo> {
    let mut templates: BTreeMap<String, TemplateInfo> = BTreeMap::new();

    // Built-in templates
    for (name, content) in BUILTIN_TEMPLATES {
        if let Ok(cfg) = Config::from_str(content) {
            let image = cfg
                .sandbox
                .base_image
                .clone()
                .unwrap_or_else(|| cfg.sandbox.runtime.clone());
            templates.insert(
                name.to_string(),
                TemplateInfo {
                    name: name.to_string(),
                    source: TemplateSource::BuiltIn,
                    image,
                    profile: format!("{:?}", cfg.security.profile).to_lowercase(),
                    memory_mb: cfg.resources.memory_mb,
                },
            );
        }
    }

    // Local templates (override built-in if same name)
    let local_dir = local_templates_dir();
    if let Ok(entries) = fs::read_dir(&local_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "toml")
                && let Some(name) = path.file_stem().map(|s| s.to_string_lossy().to_string())
                && let Ok(content) = fs::read_to_string(&path)
                && let Ok(cfg) = Config::from_str(&content)
            {
                let image = cfg
                    .sandbox
                    .base_image
                    .clone()
                    .unwrap_or_else(|| cfg.sandbox.runtime.clone());
                templates.insert(
                    name.clone(),
                    TemplateInfo {
                        name,
                        source: TemplateSource::Local,
                        image,
                        profile: format!("{:?}", cfg.security.profile).to_lowercase(),
                        memory_mb: cfg.resources.memory_mb,
                    },
                );
            }
        }
    }

    templates.into_values().collect()
}

/// Save a config as a local template
pub fn save(name: &str, config: &Config) -> Result<PathBuf> {
    let local_dir = local_templates_dir();
    fs::create_dir_all(&local_dir)?;

    let path = local_dir.join(format!("{}.toml", name));
    let content = toml::to_string_pretty(config)?;
    fs::write(&path, content)?;
    Ok(path)
}

/// Remove a local template
pub fn remove(name: &str) -> Result<()> {
    let local_dir = local_templates_dir();
    let path = local_dir.join(format!("{}.toml", name));
    if !path.exists() {
        // Check if it's a built-in — can't remove those
        if BUILTIN_TEMPLATES.iter().any(|(n, _)| *n == name) {
            bail!("'{}' is a built-in template and cannot be removed", name);
        }
        bail!("template '{}' not found in local templates", name);
    }
    fs::remove_file(&path)?;
    Ok(())
}

/// Fetch and cache a github: template locally
pub fn add_github(gh_specifier: &str) -> Result<ResolvedTemplate> {
    let gh_path = gh_specifier.strip_prefix("github:").unwrap_or(gh_specifier);

    // Clear cache for this entry so we re-fetch
    let cache_dir = local_templates_dir().join("github-cache");
    let cache_key = gh_path.replace('/', "--");
    let cache_path = cache_dir.join(format!("{}.toml", cache_key));
    let _ = fs::remove_file(&cache_path);

    resolve_github(gh_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_templates_parse() {
        for (name, content) in BUILTIN_TEMPLATES {
            let result = Config::from_str(content);
            assert!(
                result.is_ok(),
                "built-in template '{}' failed to parse: {:?}",
                name,
                result.err()
            );
        }
    }

    #[test]
    fn test_builtin_template_count() {
        assert_eq!(BUILTIN_TEMPLATES.len(), 18);
    }

    #[test]
    fn test_resolve_builtin() {
        let t = resolve("claude-sandbox").unwrap();
        assert_eq!(t.name, "claude-sandbox");
        assert_eq!(t.source, TemplateSource::BuiltIn);
        let cfg = t.parse().unwrap();
        assert_eq!(cfg.agent.preferred, "claude");
    }

    #[test]
    fn test_resolve_builtin_secure() {
        let t = resolve("secure").unwrap();
        assert_eq!(t.source, TemplateSource::BuiltIn);
        let cfg = t.parse().unwrap();
        assert_eq!(cfg.security.network, Some(false));
    }

    #[test]
    fn test_resolve_not_found() {
        let result = resolve("nonexistent-template-xyz");
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_file_not_found() {
        let result = resolve("./does-not-exist.toml");
        assert!(result.is_err());
    }

    #[test]
    fn test_list_includes_builtins() {
        let templates = list_all();
        assert!(templates.len() >= 18);
        let names: Vec<&str> = templates.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"claude-sandbox"));
        assert!(names.contains(&"secure"));
        assert!(names.contains(&"python-ml"));
    }

    #[test]
    fn test_save_and_remove() {
        let cfg = Config::from_str(BUILTIN_TEMPLATES[0].1).unwrap();
        let name = "test-save-remove-template";
        let path = save(name, &cfg).unwrap();
        assert!(path.exists());

        // Should resolve as local now
        let t = resolve(name).unwrap();
        assert_eq!(t.source, TemplateSource::Local);

        // Clean up
        remove(name).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn test_remove_builtin_fails() {
        let result = remove("claude-sandbox");
        assert!(result.is_err());
    }

    #[test]
    fn test_template_source_display() {
        assert_eq!(TemplateSource::BuiltIn.to_string(), "built-in");
        assert_eq!(TemplateSource::Local.to_string(), "local");
        assert_eq!(TemplateSource::GitHub.to_string(), "github");
        assert_eq!(TemplateSource::FilePath.to_string(), "file");
    }
}
