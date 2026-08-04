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
        let mut paths: Vec<String> = current
            .split(':')
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
            .collect();

        for runtime_path in RUNTIME_PATHS {
            if std::path::Path::new(runtime_path).is_dir()
                && !paths.iter().any(|path| path == runtime_path)
            {
                paths.push((*runtime_path).to_string());
            }
        }

        let updated = paths.join(":");
        if updated != current {
            // SAFETY: this runs at process startup, before agentkernel spawns
            // any worker or request-handling tasks.
            unsafe { std::env::set_var("PATH", updated) };
        }
    }
}
