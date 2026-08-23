# AgentKernel Sandboxes for VS Code

This extension lists running [AgentKernel](https://github.com/thrashr888/agentkernel)
sandboxes in the Explorer and opens them with Microsoft’s **Remote - SSH** extension.
It is intentionally a small, separately publishable package: the AgentKernel API
client and SSH-config integration do not depend on the Rust workspace.

## Requirements

- AgentKernel running its HTTP server (the default is `http://localhost:18888`)
- The `agentkernel` CLI available on the extension host’s `PATH`
- Microsoft [Remote - SSH](https://marketplace.visualstudio.com/items?itemName=ms-vscode-remote.remote-ssh)
- The sandbox must have been created with SSH enabled (`agentkernel sandbox create NAME --ssh`)

## Use

1. Start AgentKernel’s server and create/start an SSH-enabled sandbox.
2. Open the **AgentKernel Sandboxes** view in the Explorer.
3. Select a running sandbox. The extension runs `agentkernel ssh config NAME`, updates
   the configured OpenSSH config, and asks Remote - SSH to open `agentkernel-NAME`.

Only blocks surrounded by `# >>> agentkernel-vscode ...` and
`# <<< agentkernel-vscode` are managed. Existing SSH configuration is preserved, and
reconnecting updates the block instead of adding duplicates. The generated block uses
AgentKernel’s short-lived certificate and `agentkernel ssh-proxy` command; the
extension never reads or stores private keys or API responses beyond the current refresh.

## Settings

| Setting | Default | Purpose |
| --- | --- | --- |
| `agentkernel.apiUrl` | `http://localhost:18888` | AgentKernel HTTP API URL |
| `agentkernel.apiKey` | empty | Optional API Bearer token (stored as a password setting) |
| `agentkernel.cliPath` | `agentkernel` | AgentKernel CLI path |
| `agentkernel.sshConfigPath` | `~/.ssh/config` | OpenSSH config file to update |
| `agentkernel.refreshInterval` | `30` | Refresh interval in seconds; `0` disables polling |

For a remote AgentKernel server, set `agentkernel.apiUrl` and `agentkernel.apiKey` as
needed. The SSH config command still runs locally, so the local CLI must be able to
resolve the target sandbox.

## Development

```bash
npm install
npm test
npm run package
```

The package is MIT licensed, matching AgentKernel.
