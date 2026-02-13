/**
 * Gondolin-style secrets demo for agentkernel.
 *
 * Demonstrates the same pattern as github.com/earendil-works/gondolin:
 * secrets are injected as HTTP headers by the host proxy and never
 * enter the sandbox VM. The sandbox only sees placeholder env vars.
 *
 * Usage:
 *   export GITHUB_TOKEN="ghp_..."
 *   npx tsx examples/secrets-proxy/gondolin_demo.ts
 */

import { AgentKernel } from "../../sdk/nodejs/src/index.js";

const SANDBOX_NAME = "gondolin-demo";

async function main() {
  const token = process.env.GITHUB_TOKEN;
  if (!token) {
    console.error("Set GITHUB_TOKEN env var first:");
    console.error('  export GITHUB_TOKEN="ghp_..."');
    process.exit(1);
  }

  const client = new AgentKernel();

  try {
    // Create sandbox with secret binding (Gondolin pattern).
    // The proxy intercepts HTTPS requests to api.github.com and injects
    // an Authorization header with the real token. The VM never sees it.
    console.log("Creating sandbox with secret binding...");
    await client.createSandbox(SANDBOX_NAME, {
      image: "python:3.12-slim",
      profile: "moderate",
      secrets: [`GITHUB_TOKEN=${token}:api.github.com`],
    });

    // 1. Verify the sandbox doesn't have the real token
    console.log("\n--- Env check (token should be a placeholder) ---");
    const envCheck = await client.execInSandbox(SANDBOX_NAME, [
      "sh", "-c", "echo GITHUB_TOKEN=$GITHUB_TOKEN",
    ]);
    console.log(envCheck.output.trim());

    // 2. Make an authenticated API call through the proxy (HTTPS with MITM).
    //    The proxy transparently injects Authorization: Bearer <real-token>.
    console.log("\n--- Calling GitHub API (secret injected by proxy) ---");
    const result = await client.execInSandbox(SANDBOX_NAME, [
      "python3", "-c",
      `import urllib.request, json
req = urllib.request.Request("https://api.github.com/user",
    headers={"Accept": "application/vnd.github+json"})
resp = urllib.request.urlopen(req, timeout=15)
print(resp.read().decode())`,
    ]);
    const user = JSON.parse(result.output);
    console.log(`Authenticated as: ${user.login} (${user.name})`);
    console.log(`Public repos: ${user.public_repos}`);

    // 3. Try an unauthorized host — should be blocked by the proxy
    console.log("\n--- Attempting unauthorized host (should fail) ---");
    try {
      await client.execInSandbox(SANDBOX_NAME, [
        "python3", "-c",
        "import urllib.request; urllib.request.urlopen('https://evil.com', timeout=5)",
      ]);
      console.log("ERROR: request to unauthorized host should have failed");
    } catch {
      console.log("Blocked as expected");
    }

    console.log("\nDone. Secret never entered the VM.");
  } finally {
    try {
      await client.removeSandbox(SANDBOX_NAME);
      console.log("Sandbox removed.");
    } catch {
      // ignore if already removed
    }
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
