import Foundation

/// Client for the agentkernel HTTP API.
///
/// Thread-safe via Swift actor isolation. All methods are `async throws`.
///
/// ```swift
/// let client = AgentKernel()
/// let output = try await client.run(["echo", "hello"])
/// print(output.output) // "hello\n"
/// ```
public actor AgentKernel {
    private let config: ResolvedConfig
    private let session: URLSession

    /// SDK version string.
    static let sdkVersion = "0.4.0"

    /// Create a client with optional configuration.
    /// Resolution order: explicit options > environment variables > defaults.
    public init(_ options: AgentKernelOptions? = nil) {
        let config = ResolvedConfig(options: options)
        self.config = config

        let sessionConfig = URLSessionConfiguration.default
        sessionConfig.timeoutIntervalForRequest = config.timeout
        self.session = URLSession(configuration: sessionConfig)
    }

    /// Internal initializer for testing — accepts a custom URLSessionConfiguration.
    init(options: AgentKernelOptions? = nil, sessionConfiguration: URLSessionConfiguration) {
        let config = ResolvedConfig(options: options)
        self.config = config
        self.session = URLSession(configuration: sessionConfiguration)
    }

    // MARK: - Public API

    /// Health check. Returns `"ok"` on success.
    public func health() async throws -> String {
        try await request(method: "GET", path: "/health")
    }

    /// Run a command in a temporary sandbox.
    public func run(_ command: [String], options: RunOptions? = nil) async throws -> RunOutput {
        let opts = options ?? RunOptions()
        let body = RunRequest(
            command: command,
            image: opts.image,
            profile: opts.profile,
            fast: opts.fast
        )
        return try await request(method: "POST", path: "/run", body: body)
    }

    /// Run a command and stream output via SSE.
    public func runStream(
        _ command: [String],
        options: RunOptions? = nil
    ) async throws -> AsyncThrowingStream<StreamEvent, Error> {
        let opts = options ?? RunOptions()
        let body = RunRequest(
            command: command,
            image: opts.image,
            profile: opts.profile,
            fast: opts.fast
        )

        let url = URL(string: "\(config.baseURL)/run/stream")!
        var req = URLRequest(url: url)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.setValue("text/event-stream", forHTTPHeaderField: "Accept")
        applyHeaders(&req)
        req.httpBody = try JSONEncoder().encode(body)

        let (bytes, response) = try await session.bytes(for: req)
        guard let httpResponse = response as? HTTPURLResponse else {
            throw AgentKernelError.network(
                URLError(.badServerResponse)
            )
        }
        if httpResponse.statusCode >= 400 {
            // Collect error body
            var errorBody = ""
            for try await line in bytes.lines {
                errorBody += line
            }
            throw errorFromStatus(httpResponse.statusCode, body: errorBody)
        }

        return SSEParser.parse(bytes: bytes)
    }

    /// List all sandboxes.
    public func listSandboxes() async throws -> [SandboxInfo] {
        try await request(method: "GET", path: "/sandboxes")
    }

    /// Create a new sandbox.
    public func createSandbox(
        _ name: String,
        options: CreateSandboxOptions? = nil
    ) async throws -> SandboxInfo {
        let body = CreateRequest(
            name: name,
            image: options?.image,
            vcpus: options?.vcpus,
            memory_mb: options?.memoryMB,
            profile: options?.profile
        )
        return try await request(method: "POST", path: "/sandboxes", body: body)
    }

    /// Get info about a sandbox.
    public func getSandbox(_ name: String) async throws -> SandboxInfo {
        try await request(method: "GET", path: "/sandboxes/\(name)")
    }

    /// Remove a sandbox.
    public func removeSandbox(_ name: String) async throws {
        let _: String = try await request(method: "DELETE", path: "/sandboxes/\(name)")
    }

    /// Run a command in an existing sandbox.
    public func execInSandbox(_ name: String, command: [String]) async throws -> RunOutput {
        let body = ExecRequest(command: command)
        return try await request(method: "POST", path: "/sandboxes/\(name)/exec", body: body)
    }

    /// Create a sandbox, run a closure, then remove the sandbox.
    ///
    /// The sandbox is always cleaned up, even if the closure throws.
    ///
    /// ```swift
    /// let result = try await client.withSandbox("my-sandbox") { session in
    ///     let output = try await session.run(["echo", "hello"])
    ///     return output.output
    /// }
    /// ```
    public func withSandbox<T: Sendable>(
        _ name: String,
        options: CreateSandboxOptions? = nil,
        _ body: @Sendable (SandboxSession) async throws -> T
    ) async throws -> T {
        _ = try await createSandbox(name, options: options)
        let session = SandboxSession(name: name, client: self)
        do {
            let result = try await body(session)
            try? await removeSandbox(name)
            return result
        } catch {
            try? await removeSandbox(name)
            throw error
        }
    }

    /// Read a file from a sandbox.
    public func readFile(_ name: String, path: String) async throws -> FileReadResponse {
        try await request(method: "GET", path: "/sandboxes/\(name)/files/\(path)")
    }

    /// Write a file to a sandbox.
    public func writeFile(_ name: String, path: String, content: String, encoding: String = "utf8") async throws -> String {
        let body = FileWriteRequest(content: content, encoding: encoding)
        return try await request(method: "PUT", path: "/sandboxes/\(name)/files/\(path)", body: body)
    }

    /// Delete a file from a sandbox.
    public func deleteFile(_ name: String, path: String) async throws -> String {
        try await request(method: "DELETE", path: "/sandboxes/\(name)/files/\(path)")
    }

    /// Get audit log entries for a sandbox.
    public func getSandboxLogs(_ name: String) async throws -> [[String: Any]] {
        // Use raw JSON approach since [String: Any] isn't Decodable
        let url = URL(string: "\(config.baseURL)/sandboxes/\(name)/logs")!
        var req = URLRequest(url: url)
        req.httpMethod = "GET"
        applyHeaders(&req)
        let (data, response) = try await performRequest(req)
        guard let httpResponse = response as? HTTPURLResponse else {
            throw AgentKernelError.network(URLError(.badServerResponse))
        }
        if httpResponse.statusCode >= 400 {
            let bodyText = String(data: data, encoding: .utf8) ?? ""
            throw errorFromStatus(httpResponse.statusCode, body: bodyText)
        }
        let json = try JSONSerialization.jsonObject(with: data) as? [String: Any] ?? [:]
        guard json["success"] as? Bool == true else {
            throw AgentKernelError.server(json["error"] as? String ?? "Unknown error")
        }
        return json["data"] as? [[String: Any]] ?? []
    }

    /// Run multiple commands in parallel.
    public func batchRun(_ commands: [BatchCommand]) async throws -> BatchRunResponse {
        let body = BatchRunRequest(commands: commands)
        return try await request(method: "POST", path: "/batch/run", body: body)
    }

    // MARK: - Internal

    private func request<T: Decodable>(
        method: String,
        path: String
    ) async throws -> T {
        try await request(method: method, path: path, body: nil as AnyEncodable?)
    }

    private func request<T: Decodable, B: Encodable>(
        method: String,
        path: String,
        body: B?
    ) async throws -> T {
        let url = URL(string: "\(config.baseURL)\(path)")!
        var req = URLRequest(url: url)
        req.httpMethod = method
        applyHeaders(&req)

        if let body = body {
            req.setValue("application/json", forHTTPHeaderField: "Content-Type")
            req.httpBody = try JSONEncoder().encode(body)
        }

        let (data, response) = try await performRequest(req)
        guard let httpResponse = response as? HTTPURLResponse else {
            throw AgentKernelError.network(URLError(.badServerResponse))
        }

        if httpResponse.statusCode >= 400 {
            let bodyText = String(data: data, encoding: .utf8) ?? ""
            throw errorFromStatus(httpResponse.statusCode, body: bodyText)
        }

        let apiResponse: ApiResponse<T>
        do {
            apiResponse = try JSONDecoder().decode(ApiResponse<T>.self, from: data)
        } catch {
            throw AgentKernelError.json(error)
        }

        guard apiResponse.success else {
            throw AgentKernelError.server(apiResponse.error ?? "Unknown error")
        }

        guard let result = apiResponse.data else {
            throw AgentKernelError.server("Missing data field")
        }
        return result
    }

    private func applyHeaders(_ request: inout URLRequest) {
        request.setValue(
            "agentkernel-swift-sdk/\(Self.sdkVersion)",
            forHTTPHeaderField: "User-Agent"
        )
        if let apiKey = config.apiKey {
            request.setValue("Bearer \(apiKey)", forHTTPHeaderField: "Authorization")
        }
    }

    private func performRequest(_ request: URLRequest) async throws -> (Data, URLResponse) {
        do {
            return try await session.data(for: request)
        } catch {
            throw AgentKernelError.network(error)
        }
    }
}
