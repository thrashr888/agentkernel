import Foundation

/// Handle to a sandbox within a `withSandbox` closure.
///
/// Provides a scoped interface for running commands in a sandbox
/// that is guaranteed to be cleaned up when the closure returns.
public struct SandboxSession: Sendable {
    /// The sandbox name.
    public let name: String

    private let client: AgentKernel

    init(name: String, client: AgentKernel) {
        self.name = name
        self.client = client
    }

    /// Run a command in this sandbox.
    public func run(_ command: [String]) async throws -> RunOutput {
        try await client.execInSandbox(name, command: command)
    }

    /// Get info about this sandbox.
    public func info() async throws -> SandboxInfo {
        try await client.getSandbox(name)
    }
}
