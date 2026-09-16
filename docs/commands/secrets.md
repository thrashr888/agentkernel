
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

# Read from stdin; paste the value and press Ctrl-D
agentkernel secret set ANTHROPIC_API_KEY
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

The vault backend is configured in `agentkernel.toml`:

```toml
[secrets]
backend = "file"   # "file" (default) or "env"; "keyring" is unavailable
```

| Backend | Storage | set/delete | Best For |
|---------|---------|------------|----------|
| `file` | Encrypted `~/.agentkernel/secrets.json` with a local `secrets.key` (`0600` permissions) | Yes | Local development |
| `env` | Host environment variables | No (read-only) | CI/CD pipelines |
| `keyring` | Reserved value; not implemented | No | Unavailable |

The current build implements `file` and `env`. Selecting `keyring` returns an error; there is no `keyring` Cargo feature to enable. Keep `secrets.json` and `secrets.key` protected together: someone who can read both can decrypt the stored values.

## See Also

- [Secrets Overview](../features/secrets.md) — proxy injection, placeholder tokens, file injection, SDK usage, and security model
- [Agents](../agents/index.md) — agent-specific API key configuration
