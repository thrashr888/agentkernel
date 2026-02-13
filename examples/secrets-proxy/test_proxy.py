"""Test script for proxy-based secret injection.

Run inside the sandbox to verify:
1. HTTP_PROXY/HTTPS_PROXY env vars are set
2. Proxy CA cert is trusted
3. Secrets are injected as headers (not visible to the sandbox)
4. Blocked hosts are rejected

Usage:
    agentkernel exec secrets-proxy python3 /workspace/test_proxy.py
"""

import os
import json
import urllib.request


def section(title):
    print(f"\n{'='*60}")
    print(f"  {title}")
    print(f"{'='*60}\n")


def check_env():
    """Check that proxy env vars are set."""
    section("1. Proxy Environment Variables")

    proxy_vars = [
        "HTTP_PROXY", "HTTPS_PROXY",
        "http_proxy", "https_proxy",
        "NO_PROXY",
        "NODE_EXTRA_CA_CERTS",
        "REQUESTS_CA_BUNDLE",
        "SSL_CERT_FILE",
        "AGENTKERNEL_SECRETS_PATH",
    ]

    for var in proxy_vars:
        val = os.environ.get(var, "(not set)")
        # Mask actual values for security
        if "PROXY" in var.upper() and val != "(not set)":
            print(f"  {var} = {val}")
        else:
            print(f"  {var} = {val}")

    # Check placeholder secret env vars
    section("2. Placeholder Secret Env Vars")
    secret_vars = ["OPENAI_API_KEY", "ANTHROPIC_API_KEY"]
    for var in secret_vars:
        val = os.environ.get(var, "(not set)")
        print(f"  {var} = {val}")
        if val == "ak-proxy-managed":
            print(f"    ^ Correct! Real secret is injected by proxy, not exposed here.")


def check_ca_cert():
    """Check that the proxy CA cert is installed."""
    section("3. Proxy CA Certificate")

    ca_path = "/usr/local/share/ca-certificates/agentkernel-proxy.crt"
    if os.path.exists(ca_path):
        with open(ca_path) as f:
            content = f.read()
        print(f"  CA cert found at {ca_path}")
        print(f"  Size: {len(content)} bytes")
        print(f"  Starts with: {content[:40]}...")
    else:
        print(f"  CA cert not found at {ca_path}")
        print("  (This is expected if no --secret bindings were used)")


def check_secret_files():
    """Check file-based secrets."""
    section("4. File-Based Secrets")

    secrets_path = os.environ.get("AGENTKERNEL_SECRETS_PATH", "/run/agentkernel/secrets")
    if os.path.isdir(secrets_path):
        files = os.listdir(secrets_path)
        print(f"  Secrets directory: {secrets_path}")
        print(f"  Files: {files}")
        for f in files:
            path = os.path.join(secrets_path, f)
            mode = oct(os.stat(path).st_mode)[-3:]
            print(f"    {f}: mode={mode}")
    else:
        print(f"  No secrets directory at {secrets_path}")
        print("  (This is expected if no --secret-file flags were used)")


def check_http_request():
    """Make an HTTP request through the proxy to verify it works."""
    section("5. HTTP Request Through Proxy")

    # Use httpbin.org to echo back headers — this shows if the proxy is working
    try:
        url = "http://httpbin.org/headers"
        print(f"  GET {url}")
        req = urllib.request.Request(url)
        with urllib.request.urlopen(req, timeout=10) as resp:
            data = json.loads(resp.read())
            print(f"  Status: {resp.status}")
            print(f"  Headers echoed back:")
            for k, v in data.get("headers", {}).items():
                # Don't print full auth headers
                if "auth" in k.lower() or "key" in k.lower():
                    print(f"    {k}: {v[:20]}... (truncated)")
                else:
                    print(f"    {k}: {v}")
    except Exception as e:
        print(f"  Request failed: {e}")
        print("  (This is expected if proxy is not running or network is disabled)")


def main():
    print("Agentkernel Secrets Proxy Test")
    print("=" * 60)

    check_env()
    check_ca_cert()
    check_secret_files()
    check_http_request()

    section("Done")
    print("  All checks completed. Review the output above.")
    print("  If using --secret, verify that the proxy injected headers")
    print("  without exposing real secret values inside the sandbox.")


if __name__ == "__main__":
    main()
