// Quickstart example for the AgentKernel Swift SDK.
//
// Usage: swift run quickstart
// Requires: agentkernel server running on localhost:18888

import AgentKernel

@main
struct Quickstart {
    static func main() async throws {
        let client = AgentKernel()

        // Health check
        let status = try await client.health()
        print("Server status: \(status)")

        // Run a command
        let output = try await client.run(["echo", "Hello from Swift!"])
        print("Output: \(output.output)")

        // List sandboxes
        let sandboxes = try await client.listSandboxes()
        print("Active sandboxes: \(sandboxes.count)")
        for sb in sandboxes {
            print("  - \(sb.name) (\(sb.status), \(sb.backend))")
        }
    }
}
