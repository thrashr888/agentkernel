//! Docker image building for custom Dockerfiles.
//!
//! Provides functionality to build Docker images from Dockerfiles
//! with caching support based on content hashing.

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::BuildConfig;
use crate::docker_backend::{ContainerRuntime, docker_available, podman_available};

/// Result of a Docker build operation
#[derive(Debug)]
pub struct BuildResult {
    /// The image name/tag that was built
    pub image: String,
    /// Whether the image was already cached (no build needed)
    #[allow(dead_code)]
    pub cached: bool,
}

/// Check if a Docker image exists locally
pub fn image_exists(image: &str, runtime: ContainerRuntime) -> bool {
    let cmd = runtime.cmd();
    Command::new(cmd)
        .args(["image", "inspect", image])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Build a Docker image from a Dockerfile
///
/// # Arguments
/// * `project_name` - Name used for the image tag
/// * `dockerfile_path` - Path to the Dockerfile
/// * `context_path` - Build context directory
/// * `config` - Build configuration (args, target, no_cache)
///
/// # Returns
/// * `BuildResult` with the image name and whether it was cached
pub fn build_image(
    project_name: &str,
    dockerfile_path: &Path,
    context_path: &Path,
    config: &BuildConfig,
) -> Result<BuildResult> {
    // Determine which runtime to use
    let runtime = if docker_available() {
        ContainerRuntime::Docker
    } else if podman_available() {
        ContainerRuntime::Podman
    } else {
        bail!("No container runtime available (need Docker or Podman)");
    };

    build_image_with_callbacks(
        project_name,
        dockerfile_path,
        context_path,
        config,
        runtime,
        &|image, runtime| image_exists(image, runtime),
        &|runtime, args| {
            Command::new(runtime.cmd())
                .args(args)
                .output()
                .context("Failed to run docker build")
                .and_then(|output| {
                    if output.status.success() {
                        Ok(())
                    } else {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        bail!("Docker build failed:\n{}", stderr);
                    }
                })
        },
    )
}

/// Build an image while allowing the runtime operations to be injected.
///
/// Keeping the cache decision and the build invocation in one function makes
/// the single-flight guarantee testable without requiring a Docker daemon.
fn build_image_with_callbacks(
    project_name: &str,
    dockerfile_path: &Path,
    context_path: &Path,
    config: &BuildConfig,
    runtime: ContainerRuntime,
    image_exists_fn: &dyn Fn(&str, ContainerRuntime) -> bool,
    build_fn: &dyn Fn(ContainerRuntime, &[String]) -> Result<()>,
) -> Result<BuildResult> {
    // Generate the tag from every input that can affect the resulting image,
    // rather than only the Dockerfile contents.
    let image_name = build_image_name(project_name, dockerfile_path, context_path, config)?;

    let _guard = BuildLock::acquire(&image_name)?;

    // Check if image already exists (cached). A waiter checks again after
    // acquiring the lock, which turns concurrent default requests into a
    // single build. Explicit no-cache requests retain their force-rebuild
    // semantics.
    if !config.no_cache && image_exists_fn(&image_name, runtime) {
        eprintln!("Using cached image: {}", image_name);
        return Ok(BuildResult {
            image: image_name,
            cached: true,
        });
    }

    eprintln!("Building image from {}...", dockerfile_path.display());

    // Build the docker build command
    let mut args = vec![
        "build".to_string(),
        "-t".to_string(),
        image_name.clone(),
        "-f".to_string(),
        dockerfile_path.to_string_lossy().to_string(),
    ];

    // Add build target if specified
    if let Some(ref target) = config.target {
        args.push("--target".to_string());
        args.push(target.clone());
    }

    // Add build args in a stable order so equivalent configurations produce
    // the same invocation as well as the same cache identity.
    let mut args_config: Vec<_> = config.args.iter().collect();
    args_config.sort_by(|left, right| left.0.cmp(right.0));
    for (key, value) in args_config {
        args.push("--build-arg".to_string());
        args.push(format!("{}={}", key, value));
    }

    // Add no-cache flag if requested
    if config.no_cache {
        args.push("--no-cache".to_string());
    }

    // Add context path
    args.push(context_path.to_string_lossy().to_string());

    // Run the build
    build_fn(runtime, &args)?;

    eprintln!("Built image: {}", image_name);

    Ok(BuildResult {
        image: image_name,
        cached: false,
    })
}

/// Cross-process lock for one image identity.
///
/// AgentKernel can have multiple server processes handling build requests. A
/// lock file keeps those processes from racing into duplicate builds, and the
/// OS releases the advisory lock automatically if a process exits.
struct BuildLock {
    #[allow(dead_code)]
    file: File,
}

impl BuildLock {
    fn acquire(image_name: &str) -> Result<Self> {
        let lock_dir = std::env::temp_dir().join("agentkernel-build-locks");
        std::fs::create_dir_all(&lock_dir).with_context(|| {
            format!(
                "failed to create build lock directory {}",
                lock_dir.display()
            )
        })?;
        let lock_path = lock_dir.join(format!("{}.lock", image_name.replace(':', "-")));
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("failed to open build lock {}", lock_path.display()))?;

        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;

            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
            if result != 0 {
                return Err(std::io::Error::last_os_error())
                    .with_context(|| format!("failed to lock {}", lock_path.display()));
            }
        }

        Ok(Self { file })
    }
}

/// Generate a deterministic tag from the complete Docker build input.
///
/// The context is walked in sorted relative-path order and includes file
/// contents, file modes, and symlink targets. Dockerfile contents, target, and
/// build arguments are also included because the Dockerfile may live outside
/// the context and build configuration changes the resulting image.
fn build_image_name(
    project_name: &str,
    dockerfile_path: &Path,
    context_path: &Path,
    config: &BuildConfig,
) -> Result<String> {
    let mut hasher = Sha256::new();
    hash_bytes(
        &mut hasher,
        b"dockerfile\0",
        &std::fs::read(dockerfile_path)
            .with_context(|| format!("failed to read Dockerfile {}", dockerfile_path.display()))?,
    );

    let dockerignore_path = context_path.join(".dockerignore");
    if dockerignore_path.is_file() {
        hash_bytes(
            &mut hasher,
            b"dockerignore\0",
            &std::fs::read(&dockerignore_path)
                .with_context(|| format!("failed to read {}", dockerignore_path.display()))?,
        );
    }

    let ignore = DockerIgnore::load(context_path)?;
    let mut files = Vec::new();
    collect_context_files(context_path, Path::new(""), &ignore, &mut files)?;
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    for entry in files {
        let relative_path = entry.relative_path;
        let metadata = entry.metadata;
        let path = entry.path;
        hash_bytes(&mut hasher, b"path\0", relative_path.as_bytes());
        hash_bytes(
            &mut hasher,
            b"mode\0",
            &metadata_mode(&metadata).to_le_bytes(),
        );
        if metadata.file_type().is_dir() {
            hash_bytes(&mut hasher, b"directory\0", &[]);
        } else if metadata.file_type().is_symlink() {
            let target = std::fs::read_link(&path)
                .with_context(|| format!("failed to read symlink {}", path.display()))?;
            hash_bytes(
                &mut hasher,
                b"symlink\0",
                target.to_string_lossy().as_bytes(),
            );
        } else {
            hash_bytes(
                &mut hasher,
                b"file\0",
                &std::fs::read(&path).with_context(|| {
                    format!("failed to read build context file {}", path.display())
                })?,
            );
        }
    }

    if let Some(target) = &config.target {
        hash_bytes(&mut hasher, b"target\0", target.as_bytes());
    }
    let mut args: Vec<_> = config.args.iter().collect();
    args.sort_by(|left, right| left.0.cmp(right.0));
    for (key, value) in args {
        hash_bytes(&mut hasher, b"arg-key\0", key.as_bytes());
        hash_bytes(&mut hasher, b"arg-value\0", value.as_bytes());
    }

    let digest = hasher.finalize();
    let hash = hex::encode(digest);
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
    Ok(format!("agentkernel-{}:{}", safe_name, &hash[..16]))
}

#[cfg(unix)]
fn metadata_mode(metadata: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::MetadataExt;

    metadata.mode()
}

#[cfg(not(unix))]
fn metadata_mode(metadata: &std::fs::Metadata) -> u32 {
    u32::from(metadata.permissions().readonly())
}

#[derive(Debug)]
struct ContextEntry {
    relative_path: String,
    metadata: std::fs::Metadata,
    path: PathBuf,
}

#[derive(Debug, Default)]
struct DockerIgnore {
    rules: Vec<DockerIgnoreRule>,
}

#[derive(Debug)]
struct DockerIgnoreRule {
    pattern: String,
    negated: bool,
    directory_only: bool,
}

impl DockerIgnore {
    fn load(context_path: &Path) -> Result<Self> {
        let path = context_path.join(".dockerignore");
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;

        let mut rules = Vec::new();
        for raw_line in contents.lines() {
            let line = raw_line.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (negated, pattern) = line
                .strip_prefix('!')
                .map_or((false, line), |pattern| (true, pattern));
            let pattern = pattern.trim_matches('/');
            if pattern.is_empty() {
                continue;
            }
            let directory_only = line.ends_with('/');
            rules.push(DockerIgnoreRule {
                pattern: pattern.to_string(),
                negated,
                directory_only,
            });
        }
        Ok(Self { rules })
    }

    fn ignores(&self, relative_path: &str, is_dir: bool) -> bool {
        let mut ignored = false;
        for rule in &self.rules {
            if rule.directory_only && !is_dir {
                continue;
            }
            if dockerignore_pattern_matches(&rule.pattern, relative_path) {
                ignored = !rule.negated;
            }
        }
        ignored
    }
}

fn dockerignore_pattern_matches(pattern: &str, relative_path: &str) -> bool {
    let pattern = pattern.trim_matches('/');
    if pattern.is_empty() {
        return false;
    }
    if !pattern.contains('/') {
        return relative_path
            .split('/')
            .any(|component| wildcard_matches(pattern, component));
    }
    path_pattern_matches(
        &pattern.split('/').collect::<Vec<_>>(),
        &relative_path.split('/').collect::<Vec<_>>(),
    )
}

fn path_pattern_matches(pattern: &[&str], path: &[&str]) -> bool {
    match (pattern.first(), path.first()) {
        (None, None) => true,
        (Some(&"**"), _) => {
            path_pattern_matches(&pattern[1..], path)
                || (!path.is_empty() && path_pattern_matches(pattern, &path[1..]))
        }
        (Some(pattern_component), Some(path_component)) => {
            wildcard_matches(pattern_component, path_component)
                && path_pattern_matches(&pattern[1..], &path[1..])
        }
        _ => false,
    }
}

fn wildcard_matches(pattern: &str, value: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let value: Vec<char> = value.chars().collect();
    let mut matches = vec![vec![false; value.len() + 1]; pattern.len() + 1];
    matches[0][0] = true;
    for pattern_index in 0..pattern.len() {
        for value_index in 0..=value.len() {
            if !matches[pattern_index][value_index] {
                continue;
            }
            match pattern[pattern_index] {
                '*' => {
                    matches[pattern_index + 1][value_index] = true;
                    if value_index < value.len() {
                        matches[pattern_index][value_index + 1] = true;
                    }
                }
                '?' if value_index < value.len() => {
                    matches[pattern_index + 1][value_index + 1] = true;
                }
                character if value_index < value.len() && character == value[value_index] => {
                    matches[pattern_index + 1][value_index + 1] = true;
                }
                _ => {}
            }
        }
    }
    matches[pattern.len()][value.len()]
}

fn hash_bytes(hasher: &mut Sha256, label: &[u8], bytes: &[u8]) {
    hasher.update(label);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn collect_context_files(
    root: &Path,
    relative_dir: &Path,
    ignore: &DockerIgnore,
    files: &mut Vec<ContextEntry>,
) -> Result<()> {
    let directory = root.join(relative_dir);
    for entry in std::fs::read_dir(&directory)
        .with_context(|| format!("failed to read build context {}", directory.display()))?
    {
        let entry = entry?;
        let name = entry.file_name();
        let relative_path = relative_dir.join(&name);
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        let relative = relative_path.to_string_lossy().replace('\\', "/");
        if metadata.file_type().is_dir() {
            if ignore.ignores(&relative, true) {
                continue;
            }
            files.push(ContextEntry {
                relative_path: relative,
                metadata,
                path,
            });
            collect_context_files(root, &relative_path, ignore, files)?;
        } else if !ignore.ignores(&relative, false) {
            files.push(ContextEntry {
                relative_path: relative,
                metadata,
                path,
            });
        }
    }
    Ok(())
}

/// Build image if Dockerfile exists, otherwise return the base image
///
/// This is the main entry point for the build system. It handles:
/// - Auto-detection of Dockerfiles
/// - Building with caching
/// - Falling back to base image if no Dockerfile
pub fn build_or_use_image(
    project_name: &str,
    base_image: &str,
    base_dir: &Path,
    config: &crate::config::Config,
) -> Result<String> {
    // Check if we need to build
    if let Some(dockerfile_path) = config.dockerfile_path(base_dir) {
        let context_path = config.build_context(base_dir, &dockerfile_path);
        let result = build_image(project_name, &dockerfile_path, &context_path, &config.build)?;
        Ok(result.image)
    } else {
        // No Dockerfile, use the base image
        Ok(base_image.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn test_image_name_changes_when_effective_inputs_change() {
        let dir = tempdir().unwrap();
        let dockerfile_path = dir.path().join("Dockerfile");
        let source_path = dir.path().join("src.txt");

        let mut file = std::fs::File::create(&dockerfile_path).unwrap();
        writeln!(file, "FROM alpine:3.24\nCOPY src.txt /src.txt").unwrap();
        std::fs::write(&source_path, "first").unwrap();

        let config = BuildConfig::default();
        let initial =
            build_image_name("my-project", &dockerfile_path, dir.path(), &config).unwrap();
        let unchanged =
            build_image_name("my-project", &dockerfile_path, dir.path(), &config).unwrap();
        assert_eq!(
            initial, unchanged,
            "unchanged inputs should reuse the same tag"
        );

        std::fs::write(&source_path, "changed").unwrap();
        let source_changed =
            build_image_name("my-project", &dockerfile_path, dir.path(), &config).unwrap();
        assert_ne!(
            initial, source_changed,
            "context changes must invalidate the tag"
        );

        let mut with_args = config.clone();
        with_args
            .args
            .insert("VERSION".to_string(), "2".to_string());
        let args_changed =
            build_image_name("my-project", &dockerfile_path, dir.path(), &with_args).unwrap();
        assert_ne!(
            source_changed, args_changed,
            "build args must affect the tag"
        );

        with_args.target = Some("production".to_string());
        let target_changed =
            build_image_name("my-project", &dockerfile_path, dir.path(), &with_args).unwrap();
        assert_ne!(
            args_changed, target_changed,
            "build target must affect the tag"
        );
    }

    #[test]
    fn test_dockerignore_excludes_ineffective_inputs_and_keeps_empty_dirs() {
        let dir = tempdir().unwrap();
        let dockerfile_path = dir.path().join("Dockerfile");
        std::fs::write(&dockerfile_path, "FROM alpine:3.24\nCOPY . /app\n").unwrap();
        std::fs::write(
            dir.path().join(".dockerignore"),
            "ignored.txt\nlogs/\n!kept.txt\n!logs/keep.log\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("ignored.txt"), "ignored").unwrap();
        std::fs::write(dir.path().join("kept.txt"), "kept").unwrap();

        let config = BuildConfig::default();
        let initial =
            build_image_name("ignored-project", &dockerfile_path, dir.path(), &config).unwrap();

        std::fs::write(dir.path().join("ignored.txt"), "changed but ignored").unwrap();
        let ignored_changed =
            build_image_name("ignored-project", &dockerfile_path, dir.path(), &config).unwrap();
        assert_eq!(initial, ignored_changed);

        std::fs::write(dir.path().join("kept.txt"), "changed and included").unwrap();
        let kept_changed =
            build_image_name("ignored-project", &dockerfile_path, dir.path(), &config).unwrap();
        assert_ne!(ignored_changed, kept_changed);

        std::fs::create_dir(dir.path().join("logs")).unwrap();
        std::fs::write(dir.path().join("logs/server.log"), "ignored").unwrap();
        // Docker does not re-include a child when its parent directory is
        // excluded, even when a later negation names that child.
        std::fs::write(dir.path().join("logs/keep.log"), "still ignored").unwrap();
        let ignored_directory_changed =
            build_image_name("ignored-project", &dockerfile_path, dir.path(), &config).unwrap();
        assert_eq!(kept_changed, ignored_directory_changed);

        std::fs::create_dir(dir.path().join("empty")).unwrap();
        let empty_directory_added =
            build_image_name("ignored-project", &dockerfile_path, dir.path(), &config).unwrap();
        assert_ne!(ignored_directory_changed, empty_directory_added);
    }

    #[test]
    fn test_concurrent_requests_build_an_image_once() {
        let dir = tempdir().unwrap();
        let dockerfile_path = dir.path().join("Dockerfile");
        std::fs::write(&dockerfile_path, "FROM alpine:3.24\n").unwrap();

        let built = Arc::new(AtomicBool::new(false));
        let build_count = Arc::new(AtomicUsize::new(0));
        let start = Arc::new(std::sync::Barrier::new(8));
        let config = BuildConfig::default();

        thread::scope(|scope| {
            let mut handles = Vec::new();
            for _ in 0..8 {
                let dockerfile_path = dockerfile_path.clone();
                let context_path = dir.path().to_path_buf();
                let config = config.clone();
                let built = Arc::clone(&built);
                let build_count = Arc::clone(&build_count);
                let start = Arc::clone(&start);
                handles.push(scope.spawn(move || {
                    start.wait();
                    let image_exists = |_: &str, _: ContainerRuntime| built.load(Ordering::Acquire);
                    let run_build = |_: ContainerRuntime, _: &[String]| {
                        build_count.fetch_add(1, Ordering::AcqRel);
                        thread::sleep(Duration::from_millis(25));
                        built.store(true, Ordering::Release);
                        Ok(())
                    };
                    build_image_with_callbacks(
                        "concurrent-project",
                        &dockerfile_path,
                        &context_path,
                        &config,
                        ContainerRuntime::Docker,
                        &image_exists,
                        &run_build,
                    )
                    .unwrap()
                }));
            }

            for handle in handles {
                let result = handle.join().unwrap();
                assert!(result.image.starts_with("agentkernel-concurrent-project:"));
            }
        });

        assert_eq!(build_count.load(Ordering::Acquire), 1);
    }
}
