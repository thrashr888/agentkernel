
# agentkernel secret

Manage secrets (API keys and credentials) in the secret vault.

## Subcommands

| Command | Description |
|---------|-------------|
| `secret set <KEY> [VALUE]` | Store a secret (reads from stdin if value not provided) |
| `secret get <KEY>` | Retrieve a secret value |
| `secret list` | List all stored secret keys |
| `secret delete <KEY>` | Delete a secret |

## Examples

### Store a secret

```bash
# Set directly
agentkernel secret set ANTHROPIC_API_KEY sk-ant-...

# Read from stdin (more secure, avoids shell history)
echo "sk-ant-..." | agentkernel secret set ANTHROPIC_API_KEY
```

### Retrieve a secret

```bash
agentkernel secret get ANTHROPIC_API_KEY
```

### List secrets

```bash
$ agentkernel secret list
Keys:
  ANTHROPIC_API_KEY
  OPENAI_API_KEY
  GITHUB_TOKEN
```

### Delete a secret

```bash
agentkernel secret delete GITHUB_TOKEN
```

## Storage Backends

The vault supports three backends, configured in `agentkernel.toml`:

```toml
[secrets]
backend = "file"   # "file" (default), "env", or "keyring"
```

| Backend | Storage | set/delete | Best For |
|---------|---------|------------|----------|
| `file` | `~/.agentkernel/secrets.json` (base64-encoded, `0600` perms) | Yes | Local development |
| `env` | Host environment variables | No (read-only) | CI/CD pipelines |
| `keyring` | OS keychain (macOS Keychain, Linux secret-service) | Yes | Production workstations |

The `keyring` backend requires building with the `keyring` Cargo feature.

## See Also

- [Secrets Overview](../features/secrets.md) — proxy injection, placeholder tokens, file injection, SDK usage, and security model
- [Agents](../agents/index.md) — agent-specific API key configuration
