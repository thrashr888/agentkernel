// Sandbox session example for the AgentKernel Swift SDK.
//
// Demonstrates withSandbox for guaranteed cleanup.
// Requires: agentkernel server running on localhost:8880

import AgentKernel

@main
struct SandboxExample {
    static func main() async throws {
        let client = AgentKernel()

        // withSandbox guarantees cleanup even if the closure throws.
        let result: String = try await client.withSandbox("swift-demo", image: "python:3.12-alpine") { session in
            print("Sandbox '\(session.name)' created")

            // Run commands inside the sandbox
            let hello = try await session.run(["echo", "Hello from sandbox!"])
            print("  \(hello.output)")

            let pyVersion = try await session.run(["python", "--version"])
            print("  Python: \(pyVersion.output)")

            // Get sandbox info
            let info = try await session.info()
            print("  Status: \(info.status), Backend: \(info.backend)")

            return pyVersion.output
        }
        // Sandbox is now removed automatically
        print("Done. Python version was: \(result)")
    }
}
