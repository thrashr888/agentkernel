#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10"
# dependencies = ["httpx>=0.27", "httpx-sse>=0.4", "pydantic>=2.0"]
# ///
"""
Gondolin-style secrets demo for agentkernel.

Demonstrates the same pattern as github.com/earendil-works/gondolin:
secrets are injected as HTTP headers by the host proxy and never
enter the sandbox VM. The sandbox only sees placeholder env vars.

Usage:
    export GITHUB_TOKEN="ghp_..."
    uv run examples/secrets-proxy/gondolin_demo.py
"""

import json
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "../../sdk/python/src"))

from agentkernel import AgentKernel

SANDBOX_NAME = "gondolin-demo"


def main():
    token = os.environ.get("GITHUB_TOKEN")
    if not token:
        print("Set GITHUB_TOKEN env var first:", file=sys.stderr)
        print('  export GITHUB_TOKEN="ghp_..."', file=sys.stderr)
        sys.exit(1)

    with AgentKernel() as client:
        try:
            # Create sandbox with secret binding (Gondolin pattern).
            # The proxy intercepts HTTPS requests to api.github.com and injects
            # an Authorization header with the real token. The VM never sees it.
            print("Creating sandbox with secret binding...")
            client.create_sandbox(
                SANDBOX_NAME,
                image="python:3.12-slim",
                profile="moderate",
                secrets=[f"GITHUB_TOKEN={token}:api.github.com"],
            )

            # Verify the sandbox doesn't have the real token
            print("\n--- Env check (token should be a placeholder) ---")
            env_check = client.exec_in_sandbox(
                SANDBOX_NAME,
                ["sh", "-c", "echo GITHUB_TOKEN=$GITHUB_TOKEN"],
            )
            print(env_check.output)

            # Make an authenticated API call through the proxy.
            # The proxy transparently injects Authorization: Bearer <real-token>.
            print("--- Calling GitHub API (secret injected by proxy) ---")
            result = client.exec_in_sandbox(SANDBOX_NAME, [
                "python3", "-c",
                'import urllib.request, json\n'
                'req = urllib.request.Request("https://api.github.com/user",\n'
                '    headers={"Accept": "application/vnd.github+json"})\n'
                'resp = urllib.request.urlopen(req)\n'
                'print(resp.read().decode())',
            ])
            user = json.loads(result.output)
            print(f"Authenticated as: {user['login']} ({user.get('name', '')})")
            print(f"Public repos: {user['public_repos']}")

            # Try an unauthorized host — should be blocked
            print("\n--- Attempting unauthorized host (should fail) ---")
            try:
                client.exec_in_sandbox(SANDBOX_NAME, [
                    "python3", "-c",
                    "import urllib.request; urllib.request.urlopen('https://evil.com')",
                ])
                print("ERROR: request to unauthorized host should have failed")
            except Exception:
                print("Blocked as expected (domain not in allowlist)")

            print("\nDone. Secret never entered the VM.")

        finally:
            try:
                client.remove_sandbox(SANDBOX_NAME)
                print("Sandbox removed.")
            except Exception:
                pass


if __name__ == "__main__":
    main()
