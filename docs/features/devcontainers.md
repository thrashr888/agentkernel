# Development Container configuration

AgentKernel can use a project's `.devcontainer/devcontainer.json` as the
source for a sandbox image and workspace. This is opt-in so an existing
`agentkernel.toml` remains unchanged unless requested:

```bash
agentkernel sandbox create my-project --auto-devcontainer
agentkernel sandbox create my-project --devcontainer .devcontainer/devcontainer.json
agentkernel run --auto-devcontainer -- npm test
```

The explicit command-line image, directory, backend, ports, volumes, and
security options remain authoritative over values read from the file. Relative
Dockerfile and build-context paths are resolved against the devcontainer file
and must stay inside the project. The supported build keys are `dockerfile`
and `context`; `args`, `target`, `options`, and other build extensions fail with
an actionable error instead of being ignored.

Supported fields are `image`, `build.dockerfile`, `build.context`,
`workspaceFolder`, `containerEnv`, `remoteEnv`, the official Docker
`--mount` string/object forms, and string or argv-array `postCreateCommand`.
`remoteEnv` values override duplicate `containerEnv` values. Commands are
passed to the sandbox as argv; string commands use an explicit `sh -c` argv
entry and are never concatenated with another command.
`postCreateCommand` runs once after the sandbox is first started successfully;
its completion is persisted, and a failed command remains retryable on the next
start. The command metadata is retained in sandbox state without rerunning it
on ordinary subsequent starts.

The workspace bind mount is restricted to the project directory (including the
`${localWorkspaceFolder}` token). Named volumes use AgentKernel's existing
volume store and must already exist. Other host bind mounts, path traversal,
environment substitutions such as `${localEnv:SECRET}`, and unsupported
devcontainer lifecycle fields are rejected.

`customizations.vscode.extensions` is parsed and reported as metadata. It is
not installed because AgentKernel sandboxes do not include a VS Code server.
`features` are parsed for diagnostics but are intentionally rejected with a
message directing the user to bake the dependency into the Dockerfile. Feature
installation has substantial build-time and supply-chain semantics that the
current image builder does not implement.

Environment values are never printed in create/run diagnostics. Avoid placing
secrets directly in the file; use AgentKernel's secrets system for credentials.
