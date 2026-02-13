/// Gondolin-style secrets demo for agentkernel.
///
/// Demonstrates the same pattern as github.com/earendil-works/gondolin:
/// secrets are injected as HTTP headers by the host proxy and never
/// enter the sandbox VM. The sandbox only sees placeholder env vars.
///
/// Usage:
///   export GITHUB_TOKEN="ghp_..."
///   swift run GondolinDemo
///
/// (Or copy this into a Swift package that depends on AgentKernel.)

import AgentKernel
import Foundation

let sandboxName = "gondolin-demo"

@main
struct GondolinDemo {
    static func main() async throws {
        guard let token = ProcessInfo.processInfo.environment["GITHUB_TOKEN"], !token.isEmpty else {
            fputs("Set GITHUB_TOKEN env var first:\n", stderr)
            fputs("  export GITHUB_TOKEN=\"ghp_...\"\n", stderr)
            exit(1)
        }

        let client = AgentKernel()

        do {
            // Create sandbox with secret binding (Gondolin pattern).
            // The proxy intercepts HTTPS requests to api.github.com and injects
            // an Authorization header with the real token. The VM never sees it.
            print("Creating sandbox with secret binding...")
            _ = try await client.createSandbox(sandboxName, options: CreateSandboxOptions(
                image: "python:3.12-slim",
                profile: .moderate,
                secrets: ["GITHUB_TOKEN=\(token):api.github.com"]
            ))

            // 1. Verify the sandbox doesn't have the real token
            print("\n--- Env check (token should be a placeholder) ---")
            let envCheck = try await client.execInSandbox(
                sandboxName,
                command: ["sh", "-c", "echo GITHUB_TOKEN=$GITHUB_TOKEN"]
            )
            print(envCheck.output.trimmingCharacters(in: .whitespacesAndNewlines))

            // 2. Make an authenticated API call through the proxy (HTTPS with MITM).
            //    The proxy transparently injects Authorization: Bearer <real-token>.
            print("\n--- Calling GitHub API (secret injected by proxy) ---")
            let result = try await client.execInSandbox(sandboxName, command: [
                "python3", "-c",
                """
                import urllib.request, json
                req = urllib.request.Request("https://api.github.com/user",
                    headers={"Accept": "application/vnd.github+json"})
                resp = urllib.request.urlopen(req, timeout=15)
                print(resp.read().decode())
                """,
            ])
            let data = result.output.data(using: .utf8)!
            let user = try JSONSerialization.jsonObject(with: data) as! [String: Any]
            print("Authenticated as: \(user["login"] ?? "?") (\(user["name"] ?? ""))")
            print("Public repos: \(user["public_repos"] ?? 0)")

            // 3. Try an unauthorized host — should be blocked by the proxy
            print("\n--- Attempting unauthorized host (should fail) ---")
            do {
                _ = try await client.execInSandbox(sandboxName, command: [
                    "python3", "-c",
                    "import urllib.request; urllib.request.urlopen('https://evil.com', timeout=5)",
                ])
                print("ERROR: request to unauthorized host should have failed")
            } catch {
                print("Blocked as expected")
            }

            print("\nDone. Secret never entered the VM.")
        } catch {
            fputs("Error: \(error)\n", stderr)
        }

        // Cleanup
        try? await client.removeSandbox(sandboxName)
        print("Sandbox removed.")
    }
}
