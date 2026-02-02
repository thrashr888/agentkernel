
# agentkernel secret

Manage secrets (API keys and credentials) in a local vault. Secrets are stored in `~/.agentkernel/secrets.json` with file permissions set to `0600`.

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

## Storage

- Secrets file: `~/.agentkernel/secrets.json`
- File permissions: `0600` (owner read/write only)
- Values are base64-encoded (not encrypted — use OS keychain for production secrets)

## See Also

- [Agents](agents) - Agent-specific API key configuration
