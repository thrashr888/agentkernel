//! Runtime environment helpers for installed and background processes.

/// Add the standard macOS locations for external container runtimes to the
/// process environment.
///
/// A Homebrew LaunchAgent inherits launchd's minimal PATH rather than the
/// user's interactive shell PATH.  Docker Desktop and Apple Containers are
/// commonly installed in `/usr/local/bin`, so backend discovery would fail
/// only when agentkernel was started as a service.  Keep the adjustment here
/// so every backend command (not just health checks) sees the same runtime
/// environment.
pub fn ensure_host_command_path() {
    #[cfg(target_os = "macos")]
    {
        const RUNTIME_PATHS: &[&str] = &[
            "/opt/homebrew/bin",
            "/opt/homebrew/sbin",
            "/usr/local/bin",
            "/usr/local/sbin",
            "/Applications/Docker.app/Contents/Resources/bin",
        ];

        let current = std::env::var_os("PATH")
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        let available_runtime_paths: Vec<&str> = RUNTIME_PATHS
            .iter()
            .copied()
            .filter(|path| std::path::Path::new(path).is_dir())
            .collect();
        let updated = prioritize_host_command_paths(&current, &available_runtime_paths);
        if updated != current {
            // SAFETY: this runs at process startup, before agentkernel spawns
            // any worker or request-handling tasks.
            unsafe { std::env::set_var("PATH", updated) };
        }
    }
}

/// Put managed runtime locations before inherited PATH entries. This ensures a
/// current Homebrew install wins over stale package-manager leftovers while
/// retaining every unrelated user path in its original order.
#[cfg(any(target_os = "macos", test))]
fn prioritize_host_command_paths(current: &str, runtime_paths: &[&str]) -> String {
    let mut paths: Vec<&str> = Vec::new();

    for path in runtime_paths
        .iter()
        .copied()
        .chain(current.split(':').filter(|path| !path.is_empty()))
    {
        if !paths.contains(&path) {
            paths.push(path);
        }
    }

    paths.join(":")
}

#[cfg(test)]
mod tests {
    use super::prioritize_host_command_paths;

    #[test]
    fn prioritizes_homebrew_before_stale_usr_local_binary() {
        let updated = prioritize_host_command_paths(
            "/usr/local/bin:/usr/bin:/opt/homebrew/bin",
            &["/opt/homebrew/bin", "/usr/local/bin"],
        );

        assert_eq!(updated, "/opt/homebrew/bin:/usr/local/bin:/usr/bin");
    }

    #[test]
    fn preserves_unrelated_paths_and_removes_duplicates() {
        let updated = prioritize_host_command_paths(
            "/custom/bin:/usr/bin:/custom/bin",
            &["/opt/homebrew/bin"],
        );

        assert_eq!(updated, "/opt/homebrew/bin:/custom/bin:/usr/bin");
    }
}
