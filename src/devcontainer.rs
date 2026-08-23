//! Safe adapter for the Development Containers JSONC format.
//!
//! AgentKernel intentionally implements only fields with an unambiguous
//! sandbox equivalent. Unsupported fields fail loudly rather than being
//! silently dropped.

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::config::{BuildConfig, Config};

const CANONICAL_PATH: &str = ".devcontainer/devcontainer.json";

#[derive(Debug, Clone)]
pub struct DevContainerConfig {
    pub path: PathBuf,
    pub project_root: PathBuf,
    pub image: Option<String>,
    pub build: Option<DevContainerBuild>,
    pub workspace_folder: Option<String>,
    pub environment: Vec<(String, String)>,
    pub mounts: Vec<DevContainerMount>,
    pub post_create_commands: Vec<Vec<String>>,
    pub vscode_extensions: Vec<String>,
    pub features: BTreeMap<String, Value>,
    unsupported_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevContainerBuild {
    pub dockerfile: PathBuf,
    pub context: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevContainerMountType {
    Bind,
    Volume,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevContainerMount {
    pub source: String,
    pub target: String,
    pub mount_type: DevContainerMountType,
    pub read_only: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SupportedMounts {
    pub volume_specs: Vec<String>,
    pub workspace_host_path: Option<PathBuf>,
    pub workspace_container_path: Option<String>,
}

pub fn discover(start_dir: &Path) -> Result<Option<PathBuf>> {
    let start_dir = start_dir.canonicalize().with_context(|| {
        format!(
            "failed to resolve project directory {}",
            start_dir.display()
        )
    })?;
    let canonical = start_dir.join(CANONICAL_PATH);
    if canonical.is_file() {
        return Ok(Some(canonical));
    }
    let root_level = start_dir.join(".devcontainer.json");
    Ok(root_level.is_file().then_some(root_level))
}

pub fn load(path: &Path) -> Result<DevContainerConfig> {
    let path = path
        .canonicalize()
        .with_context(|| format!("failed to resolve devcontainer file {}", path.display()))?;
    if !path.is_file() {
        bail!("devcontainer path is not a file: {}", path.display());
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read devcontainer file {}", path.display()))?;
    let root: Value = serde_json::from_str(&strip_jsonc(&content)?)
        .with_context(|| format!("failed to parse JSONC in {}", path.display()))?;
    let object = root
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("devcontainer root must be a JSON object"))?;
    let file_dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("devcontainer file has no parent"))?;
    let project_root = if file_dir
        .file_name()
        .is_some_and(|name| name == ".devcontainer")
    {
        file_dir.parent().unwrap_or(file_dir)
    } else {
        file_dir
    }
    .canonicalize()?;
    let image = optional_string(object, "image")?;
    let build = parse_build(object.get("build"), file_dir)?;
    let workspace_folder = optional_string(object, "workspaceFolder")?;
    if let Some(folder) = &workspace_folder {
        validate_container_path(folder, "workspaceFolder")?;
    }
    let mut env = parse_environment(object.get("containerEnv"), "containerEnv")?;
    for (key, value) in parse_environment(object.get("remoteEnv"), "remoteEnv")? {
        if let Some(existing) = env.iter_mut().find(|(name, _)| name == &key) {
            existing.1 = value;
        } else {
            env.push((key, value));
        }
    }
    Ok(DevContainerConfig {
        path,
        project_root,
        image,
        build,
        workspace_folder,
        environment: env,
        mounts: parse_mounts(object.get("mounts"))?,
        post_create_commands: parse_commands(object.get("postCreateCommand"), "postCreateCommand")?,
        vscode_extensions: parse_extensions(object.get("customizations"))?,
        features: parse_features(object.get("features"))?,
        unsupported_fields: unsupported_fields(object),
    })
}

impl DevContainerConfig {
    pub fn apply_to_config(&self, config: &mut Config) {
        if let Some(image) = &self.image {
            config.sandbox.base_image = Some(image.clone());
        }
        if let Some(build) = &self.build {
            config.build = BuildConfig {
                dockerfile: Some(build.dockerfile.to_string_lossy().into_owned()),
                context: Some(build.context.to_string_lossy().into_owned()),
                ..BuildConfig::default()
            };
        }
    }

    pub fn supported_mounts(&self) -> Result<SupportedMounts> {
        let mut result = SupportedMounts::default();
        for mount in &self.mounts {
            match mount.mount_type {
                DevContainerMountType::Volume => {
                    validate_volume_slug(&mount.source)?;
                    result.volume_specs.push(format!(
                        "{}:{}{}",
                        mount.source,
                        mount.target,
                        if mount.read_only { ":ro" } else { "" }
                    ));
                }
                DevContainerMountType::Bind => {
                    if result.workspace_host_path.is_some() {
                        bail!("only one project bind mount can be represented by AgentKernel");
                    }
                    let source = mount.source.replace(
                        "${localWorkspaceFolder}",
                        self.project_root.to_string_lossy().as_ref(),
                    );
                    if source.contains("${") {
                        bail!(
                            "devcontainer bind source uses an unsupported variable; use ${{localWorkspaceFolder}} or a project-relative path"
                        );
                    }
                    if mount.read_only {
                        bail!("read-only devcontainer workspace mounts are unsupported");
                    }
                    let source = resolve_inside(&self.project_root, &self.path, &source)?;
                    result.workspace_host_path = Some(source);
                    result.workspace_container_path = Some(mount.target.clone());
                }
            }
        }
        Ok(result)
    }

    pub fn validate_supported(&self) -> Result<()> {
        if !self.features.is_empty() {
            let names = self.features.keys().cloned().collect::<Vec<_>>().join(", ");
            bail!(
                "devcontainer features are not installed by AgentKernel ({names}); bake them into the Dockerfile or remove the features field"
            );
        }
        if !self.unsupported_fields.is_empty() {
            bail!(
                "devcontainer fields are unsupported by AgentKernel: {}",
                self.unsupported_fields.join(", ")
            );
        }
        let _ = self.supported_mounts()?;
        Ok(())
    }
}

fn parse_build(value: Option<&Value>, file_dir: &Path) -> Result<Option<DevContainerBuild>> {
    let Some(value) = value else { return Ok(None) };
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("devcontainer build must be an object"))?;
    let unsupported = object
        .keys()
        .filter(|key| !matches!(key.as_str(), "dockerfile" | "context"))
        .cloned()
        .collect::<Vec<_>>();
    if !unsupported.is_empty() {
        bail!(
            "devcontainer build fields are unsupported (only dockerfile and context are mapped): {}",
            unsupported.join(", ")
        );
    }
    let dockerfile = object
        .get("dockerfile")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("devcontainer build.dockerfile must be a string"))?;
    let dockerfile = resolve_project_path(file_dir, dockerfile, "build.dockerfile")?;
    if !dockerfile.is_file() {
        bail!(
            "devcontainer build.dockerfile does not exist: {}",
            dockerfile.display()
        );
    }
    let context = object.get("context").and_then(Value::as_str).unwrap_or(".");
    let context = resolve_project_path(file_dir, context, "build.context")?;
    if !context.is_dir() {
        bail!(
            "devcontainer build.context is not a directory: {}",
            context.display()
        );
    }
    Ok(Some(DevContainerBuild {
        dockerfile,
        context,
    }))
}

fn parse_environment(value: Option<&Value>, field: &str) -> Result<Vec<(String, String)>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("devcontainer {field} must be an object"))?;
    object.iter().map(|(key, value)| {
        validate_env_key(key, field)?;
        let value = value.as_str().ok_or_else(|| anyhow::anyhow!("devcontainer {field}.{key} must be a string; substitutions are not supported"))?;
        if value.contains("${localEnv:") || value.contains("${containerEnv:") {
            bail!("devcontainer {field}.{key} uses an environment substitution; resolve it explicitly before invoking AgentKernel");
        }
        Ok((key.clone(), value.to_string()))
    }).collect()
}

fn parse_commands(value: Option<&Value>, field: &str) -> Result<Vec<Vec<String>>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    match value {
        Value::String(command) => Ok(vec![vec!["sh".into(), "-c".into(), command.clone()]]),
        Value::Array(values) if values.iter().all(Value::is_string) => Ok(vec![
            values
                .iter()
                .map(|value| value.as_str().unwrap().to_string())
                .collect(),
        ]),
        Value::Array(_) => bail!("devcontainer {field} arrays must contain only command arguments"),
        Value::Object(_) => bail!(
            "devcontainer {field} object form expresses parallel lifecycle commands; AgentKernel requires an explicit sequential command array"
        ),
        _ => bail!("devcontainer {field} must be a string or argv array"),
    }
}

fn parse_extensions(value: Option<&Value>) -> Result<Vec<String>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Some(vscode) = value.as_object().and_then(|object| object.get("vscode")) else {
        return Ok(Vec::new());
    };
    let Some(extensions) = vscode
        .as_object()
        .and_then(|object| object.get("extensions"))
    else {
        return Ok(Vec::new());
    };
    extensions
        .as_array()
        .ok_or_else(|| {
            anyhow::anyhow!("devcontainer customizations.vscode.extensions must be an array")
        })?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(ToString::to_string)
                .ok_or_else(|| anyhow::anyhow!("VS Code extensions must be strings"))
        })
        .collect()
}

fn parse_features(value: Option<&Value>) -> Result<BTreeMap<String, Value>> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    Ok(value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("devcontainer features must be an object"))?
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect())
}

fn parse_mounts(value: Option<&Value>) -> Result<Vec<DevContainerMount>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("devcontainer mounts must be an array"))?
        .iter()
        .map(parse_mount)
        .collect()
}

fn parse_mount(value: &Value) -> Result<DevContainerMount> {
    if let Some(raw) = value.as_str() {
        if raw.contains('=') && raw.contains(',') {
            let mut kind = "bind";
            let mut source = None;
            let mut target = None;
            let mut read_only = false;
            for entry in raw.split(',') {
                let (key, value) = entry.split_once('=').ok_or_else(|| {
                    anyhow::anyhow!("invalid devcontainer --mount entry '{entry}'")
                })?;
                match key {
                    "type" => kind = value,
                    "source" | "src" => source = Some(value),
                    "target" | "dst" | "destination" => target = Some(value),
                    "readonly" | "read-only" => read_only = true,
                    "consistency" => {}
                    other => bail!("devcontainer mount option '{other}' is unsupported"),
                }
            }
            return make_mount(source, target, kind, read_only);
        }
        let parts = raw.split(':').collect::<Vec<_>>();
        if !(2..=3).contains(&parts.len()) || (parts.len() == 3 && parts[2] != "ro") {
            bail!(
                "devcontainer mount '{raw}' must use type=...,source=...,target=... or source:target[:ro]"
            );
        }
        let kind = if parts[0].contains('/') || parts[0].starts_with('.') {
            "bind"
        } else {
            "volume"
        };
        return make_mount(
            Some(parts[0]),
            Some(parts[1]),
            kind,
            parts.get(2) == Some(&"ro"),
        );
    }
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("devcontainer mount must be a string or object"))?;
    let source = object
        .get("source")
        .or_else(|| object.get("src"))
        .and_then(Value::as_str);
    let target = object
        .get("target")
        .or_else(|| object.get("dst"))
        .or_else(|| object.get("destination"))
        .and_then(Value::as_str);
    let kind = object.get("type").and_then(Value::as_str).unwrap_or("bind");
    let read_only = object
        .get("readOnly")
        .or_else(|| object.get("readonly"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    make_mount(source, target, kind, read_only)
}

fn make_mount(
    source: Option<&str>,
    target: Option<&str>,
    kind: &str,
    read_only: bool,
) -> Result<DevContainerMount> {
    let source = source.ok_or_else(|| anyhow::anyhow!("devcontainer mount is missing source"))?;
    let target = target.ok_or_else(|| anyhow::anyhow!("devcontainer mount is missing target"))?;
    validate_container_path(target, "mount target")?;
    let mount_type = match kind {
        "bind" => DevContainerMountType::Bind,
        "volume" => DevContainerMountType::Volume,
        other => bail!("devcontainer mount type '{other}' is unsupported"),
    };
    Ok(DevContainerMount {
        source: source.to_string(),
        target: target.to_string(),
        mount_type,
        read_only,
    })
}

fn optional_string(object: &Map<String, Value>, field: &str) -> Result<Option<String>> {
    match object.get(field) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => bail!("devcontainer {field} must be a string"),
    }
}

fn unsupported_fields(object: &Map<String, Value>) -> Vec<String> {
    [
        "dockerComposeFile",
        "service",
        "workspaceMount",
        "runArgs",
        "remoteUser",
        "containerUser",
        "updateRemoteUserUID",
        "initializeCommand",
        "postStartCommand",
        "postAttachCommand",
        "forwardPorts",
        "portsAttributes",
        "overrideCommand",
        "shutdownAction",
    ]
    .iter()
    .filter(|field| object.contains_key(**field))
    .map(|field| (*field).to_string())
    .collect()
}

fn resolve_project_path(base: &Path, value: &str, field: &str) -> Result<PathBuf> {
    let path = if Path::new(value).is_absolute() {
        PathBuf::from(value)
    } else {
        base.join(value)
    };
    let canonical = path.canonicalize().with_context(|| {
        format!(
            "devcontainer {field} path does not exist: {}",
            path.display()
        )
    })?;
    let project_root = if base.file_name().is_some_and(|name| name == ".devcontainer") {
        base.parent().unwrap_or(base).canonicalize()?
    } else {
        base.canonicalize()?
    };
    if !canonical.starts_with(&project_root) {
        bail!("devcontainer {field} escapes the project directory: {value}");
    }
    Ok(canonical)
}

fn resolve_inside(project_root: &Path, file: &Path, value: &str) -> Result<PathBuf> {
    let path = if Path::new(value).is_absolute() {
        PathBuf::from(value)
    } else {
        file.parent().unwrap_or(project_root).join(value)
    };
    let canonical = path.canonicalize().with_context(|| {
        format!(
            "devcontainer bind source does not exist: {}",
            path.display()
        )
    })?;
    if !canonical.starts_with(project_root) {
        bail!("devcontainer bind source escapes the project directory: {value}");
    }
    Ok(canonical)
}

fn validate_container_path(value: &str, field: &str) -> Result<()> {
    if !Path::new(value).is_absolute()
        || Path::new(value)
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        bail!("devcontainer {field} must be an absolute path without '..': {value}");
    }
    Ok(())
}

fn validate_volume_slug(value: &str) -> Result<()> {
    if value.is_empty()
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        bail!("devcontainer volume source '{value}' is not a safe volume name");
    }
    Ok(())
}

fn validate_env_key(value: &str, field: &str) -> Result<()> {
    if value.is_empty()
        || value.contains('=')
        || value
            .as_bytes()
            .iter()
            .any(|byte| *byte == 0 || *byte == b'\n')
    {
        bail!("devcontainer {field} contains an invalid environment variable name");
    }
    Ok(())
}

fn strip_jsonc(input: &str) -> Result<String> {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0;
    let mut string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if string {
            output.push(byte);
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                string = false;
            }
            index += 1;
        } else if byte == b'"' {
            string = true;
            output.push(b'"');
            index += 1;
        } else if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
        } else if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index += 2;
            let mut closed = false;
            while index + 1 < bytes.len() {
                if bytes[index] == b'*' && bytes[index + 1] == b'/' {
                    index += 2;
                    closed = true;
                    break;
                }
                if bytes[index] == b'\n' {
                    output.push(b'\n');
                }
                index += 1;
            }
            if !closed {
                bail!("unterminated block comment in devcontainer JSONC");
            }
        } else {
            output.push(byte);
            index += 1;
        }
    }
    if string {
        bail!("unterminated string in devcontainer JSONC");
    }
    let output = String::from_utf8(output)
        .context("devcontainer JSONC contained invalid UTF-8 after comment removal")?;
    let chars = output.chars().collect::<Vec<_>>();
    let mut cleaned = String::with_capacity(output.len());
    for (index, character) in chars.iter().enumerate() {
        if *character == ',' {
            let mut next = index + 1;
            while next < chars.len() && chars[next].is_whitespace() {
                next += 1;
            }
            if next < chars.len() && matches!(chars[next], '}' | ']') {
                continue;
            }
        }
        cleaned.push(*character);
    }
    Ok(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture(contents: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".devcontainer")).unwrap();
        fs::write(dir.path().join("Dockerfile"), "FROM alpine:3.24\n").unwrap();
        fs::write(dir.path().join("workspace.txt"), "ok\n").unwrap();
        fs::write(dir.path().join(CANONICAL_PATH), contents).unwrap();
        dir
    }

    #[test]
    fn parses_jsonc_and_supported_fields() {
        let dir = fixture(
            r#"{ // comment
          "image":"alpine:3.24", "workspaceFolder":"/workspace",
          "containerEnv":{"A":"one"}, "remoteEnv":{"A":"two","B":"three"},
          "postCreateCommand":["echo","safe"],
          "customizations":{"vscode":{"extensions":["rust-lang.rust-analyzer"]}}, "features":{},
        }"#,
        );
        let config = load(&dir.path().join(CANONICAL_PATH)).unwrap();
        assert_eq!(config.image.as_deref(), Some("alpine:3.24"));
        assert_eq!(
            config.environment,
            vec![("A".into(), "two".into()), ("B".into(), "three".into())]
        );
        assert_eq!(config.post_create_commands.len(), 1);
        assert_eq!(config.vscode_extensions, vec!["rust-lang.rust-analyzer"]);
        config.validate_supported().unwrap();
    }

    #[test]
    fn rejects_feature_installation_with_actionable_error() {
        let dir = fixture(r#"{"features":{"ghcr.io/devcontainers/features/node:1":{}}}"#);
        let error = load(&dir.path().join(CANONICAL_PATH))
            .unwrap()
            .validate_supported()
            .unwrap_err()
            .to_string();
        assert!(error.contains("features") && error.contains("Dockerfile"));
    }

    #[test]
    fn parses_official_mount_syntax_and_workspace_token() {
        let dir = fixture(
            r#"{"mounts":["type=bind,source=${localWorkspaceFolder},target=/workspace"],"workspaceFolder":"/workspace"}"#,
        );
        let config = load(&dir.path().join(CANONICAL_PATH)).unwrap();
        let mounts = config.supported_mounts().unwrap();
        assert_eq!(
            mounts.workspace_host_path,
            Some(dir.path().canonicalize().unwrap())
        );
        assert_eq!(
            mounts.workspace_container_path.as_deref(),
            Some("/workspace")
        );
    }

    #[test]
    fn rejects_parallel_post_create_object_form() {
        let dir = fixture(r#"{"postCreateCommand":{"install":"npm install"}}"#);
        assert!(
            load(&dir.path().join(CANONICAL_PATH))
                .unwrap_err()
                .to_string()
                .contains("parallel")
        );
    }

    #[test]
    fn rejects_unsupported_build_options_instead_of_ignoring_them() {
        let dir =
            fixture(r#"{"build":{"dockerfile":"../Dockerfile","context":"..","args":{"A":"B"}}}"#);
        assert!(
            load(&dir.path().join(CANONICAL_PATH))
                .unwrap_err()
                .to_string()
                .contains("only dockerfile and context")
        );
    }

    #[test]
    fn strips_comment_markers_inside_strings() {
        let dir =
            fixture(r#"{"image":"alpine//latest", /* comment */ "workspaceFolder":"/workspace"}"#);
        assert_eq!(
            load(&dir.path().join(CANONICAL_PATH))
                .unwrap()
                .image
                .as_deref(),
            Some("alpine//latest")
        );
    }

    #[test]
    fn preserves_unicode_jsonc_values() {
        let dir = fixture(
            r#"{"containerEnv":{"GREETING":"héllo 世界"}, // keep UTF-8
                "workspaceFolder":"/workspace"}"#,
        );
        let config = load(&dir.path().join(CANONICAL_PATH)).unwrap();
        assert_eq!(
            config.environment,
            vec![("GREETING".into(), "héllo 世界".into())]
        );
    }

    #[test]
    fn discovers_canonical_file() {
        let dir = fixture("{}");
        assert_eq!(
            discover(dir.path()).unwrap().unwrap(),
            dir.path().join(CANONICAL_PATH).canonicalize().unwrap()
        );
    }
}
