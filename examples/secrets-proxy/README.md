# Secrets Proxy Example

Demonstrates v0.12.0 secrets features:

- **Network-layer secret injection** via HTTP proxy (Gondolin pattern)
- **File-based secret injection** via `--secret-file`
- **Proxy hooks** for request/response logging
- **Interactive shell** via `agentkernel attach`

## Prerequisites

Store a test secret in the vault:

```bash
agentkernel secret set MY_TEST_KEY "sk-test-12345"
```

## Usage

### 1. Proxy-based secret injection

Secrets are injected as HTTP headers by the host proxy. The sandbox only
sees `OPENAI_API_KEY=ak-proxy-managed` — the real value never enters the VM.

```bash
# Create with inline secret binding (value:host)
agentkernel create secrets-proxy \
  --image python:3.12-slim \
  --secret OPENAI_API_KEY=sk-test-12345:api.openai.com

# Or use vault lookup (key:host)
agentkernel create secrets-proxy \
  --image python:3.12-slim \
  --secret MY_TEST_KEY:httpbin.org

# Start the sandbox
agentkernel start secrets-proxy

# Run the test script
agentkernel exec secrets-proxy python3 /workspace/test_proxy.py

# Interactive shell
agentkernel attach secrets-proxy
```

### 2. File-based secret injection

Secrets are written as files at `/run/agentkernel/secrets/KEY`:

```bash
agentkernel create secrets-files \
  --image alpine \
  --secret-file MY_TEST_KEY

agentkernel start secrets-files

# Verify the secret file exists
agentkernel exec secrets-files cat /run/agentkernel/secrets/MY_TEST_KEY

# Check permissions (should be 400)
agentkernel exec secrets-files ls -la /run/agentkernel/secrets/
```

### 3. Both together

```bash
agentkernel create secrets-both \
  --image python:3.12-slim \
  --secret OPENAI_API_KEY=sk-test:api.openai.com \
  --secret-file MY_TEST_KEY

agentkernel start secrets-both
agentkernel exec secrets-both python3 /workspace/test_proxy.py
```

### 4. Using the config file

```bash
agentkernel create secrets-proxy \
  --config examples/secrets-proxy/agentkernel.toml \
  --secret MY_TEST_KEY:httpbin.org

agentkernel start secrets-proxy
agentkernel exec secrets-proxy python3 /workspace/test_proxy.py
```

### 5. Gondolin-style SDK demo

The `gondolin_demo` scripts demonstrate the same pattern as
[Gondolin](https://github.com/earendil-works/gondolin): create a sandbox
with secret bindings via the SDK, then make authenticated API calls where
the proxy injects credentials transparently.

**TypeScript (Node SDK):**

```bash
export GITHUB_TOKEN="ghp_..."
npx tsx examples/secrets-proxy/gondolin_demo.ts
```

**Python SDK:**

```bash
export GITHUB_TOKEN="ghp_..."
python examples/secrets-proxy/gondolin_demo.py
```

Both scripts:
1. Create a sandbox with `secrets: ["GITHUB_TOKEN=<token>:api.github.com"]`
2. Verify the VM only has a placeholder env var (not the real token)
3. Call `https://api.github.com/user` — the proxy injects `Authorization: Bearer <token>`
4. Attempt a request to an unauthorized host (blocked)
5. Clean up the sandbox

## What to look for

| Check | Expected |
|-------|----------|
| `HTTP_PROXY` env var | Set to proxy address |
| `OPENAI_API_KEY` env var | `ak-proxy-managed` (placeholder) |
| CA cert at `/usr/local/share/ca-certificates/` | Present |
| Secret files at `/run/agentkernel/secrets/` | mode 400 |
| HTTP request to allowed host | 200 OK, headers injected |
| HTTP request to blocked host | 403 Forbidden |

## Proxy hooks

The config file registers two hooks:

1. **request-logger** — writes `on_request` events to `/tmp/agentkernel-proxy-events.jsonl`
2. **response-logger** — logs `on_response` events to stderr (audit)

After running requests through the proxy, check the log:

```bash
cat /tmp/agentkernel-proxy-events.jsonl | python3 -m json.tool
```

## Cleanup

```bash
agentkernel stop secrets-proxy
agentkernel remove secrets-proxy
```
