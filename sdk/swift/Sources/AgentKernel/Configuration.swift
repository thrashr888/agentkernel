import Foundation

/// Configuration options for the AgentKernel client.
public struct AgentKernelOptions: Sendable {
    /// Base URL of the agentkernel API server.
    public var baseURL: String?
    /// API key for Bearer authentication.
    public var apiKey: String?
    /// Request timeout interval in seconds.
    public var timeout: TimeInterval?

    public init(
        baseURL: String? = nil,
        apiKey: String? = nil,
        timeout: TimeInterval? = nil
    ) {
        self.baseURL = baseURL
        self.apiKey = apiKey
        self.timeout = timeout
    }
}

/// Resolved configuration with defaults applied.
/// Resolution order: explicit options > environment variables > defaults.
struct ResolvedConfig: Sendable {
    let baseURL: String
    let apiKey: String?
    let timeout: TimeInterval

    static let defaultBaseURL = "http://localhost:18888"
    static let defaultTimeout: TimeInterval = 30

    init(options: AgentKernelOptions? = nil) {
        let opts = options ?? AgentKernelOptions()

        self.baseURL = (opts.baseURL
            ?? ProcessInfo.processInfo.environment["AGENTKERNEL_BASE_URL"]
            ?? Self.defaultBaseURL)
            .trimmingSuffix("/")

        self.apiKey = opts.apiKey
            ?? ProcessInfo.processInfo.environment["AGENTKERNEL_API_KEY"]

        self.timeout = opts.timeout ?? Self.defaultTimeout
    }
}

private extension String {
    func trimmingSuffix(_ suffix: String) -> String {
        if hasSuffix(suffix) {
            return String(dropLast(suffix.count))
        }
        return self
    }
}
