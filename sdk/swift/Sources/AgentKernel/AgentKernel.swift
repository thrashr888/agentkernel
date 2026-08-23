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
    static let sdkVersion = "0.3.0"

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

    /// Discover server-supported and ready sandbox backends.
    public func getBackends() async throws -> BackendDiscovery {
        try await request(method: "GET", path: "/backends")
    }

    /// Create a new sandbox.
    public func createSandbox(
        _ name: String,
        options: CreateSandboxOptions? = nil
    ) async throws -> SandboxInfo {
        let body = CreateRequest(
            name: name,
            backend: options?.backend,
            image: options?.image,
            vcpus: options?.vcpus,
            memory_mb: options?.memoryMB,
            profile: options?.profile,
            source_url: options?.sourceURL,
            source_ref: options?.sourceRef,
            volumes: options?.volumes,
            secrets: options?.secrets,
            secret_files: options?.secretFiles
        )
        return try await request(method: "POST", path: "/sandboxes", body: body)
    }

    /// Get info about a sandbox.
    public func getSandbox(_ name: String) async throws -> SandboxInfo {
        try await request(method: "GET", path: "/sandboxes/\(name)")
    }

    /// Get info about a sandbox by UUID.
    public func getSandboxByUUID(_ uuid: String) async throws -> SandboxInfo {
        try await request(method: "GET", path: "/sandboxes/by-uuid/\(uuid)")
    }

    /// Remove a sandbox.
    public func removeSandbox(_ name: String) async throws {
        let _: String = try await request(method: "DELETE", path: "/sandboxes/\(name)")
    }

    /// Run a command in an existing sandbox.
    public func execInSandbox(
        _ name: String,
        command: [String],
        options: ExecOptions? = nil
    ) async throws -> RunOutput {
        let body = ExecRequest(
            command: command,
            env: options?.env.isEmpty == false ? options?.env : nil,
            workdir: options?.workdir,
            sudo: options?.sudo
        )
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

    /// Write multiple files to a sandbox in one request.
    public func writeFiles(_ name: String, files: [String: String]) async throws -> BatchFileWriteResponse {
        let body = BatchFileWriteRequest(files: files)
        return try await request(method: "POST", path: "/sandboxes/\(name)/files", body: body)
    }

    /// Run multiple commands in parallel.
    public func batchRun(_ commands: [BatchCommand]) async throws -> BatchRunResponse {
        let body = BatchRunRequest(commands: commands)
        return try await request(method: "POST", path: "/batch/run", body: body)
    }

    /// Start a detached (background) command in a sandbox.
    public func execDetached(
        _ name: String,
        command: [String],
        options: ExecOptions? = nil
    ) async throws -> DetachedCommand {
        let body = ExecRequest(
            command: command,
            env: options?.env.isEmpty == false ? options?.env : nil,
            workdir: options?.workdir,
            sudo: options?.sudo
        )
        return try await request(method: "POST", path: "/sandboxes/\(name)/exec/detach", body: body)
    }

    /// Get the status of a detached command.
    public func detachedStatus(_ name: String, cmdId: String) async throws -> DetachedCommand {
        try await request(method: "GET", path: "/sandboxes/\(name)/exec/detached/\(cmdId)")
    }

    /// Get logs from a detached command.
    public func detachedLogs(_ name: String, cmdId: String, stream: String? = nil) async throws -> DetachedLogsResponse {
        let query = stream == "stderr" ? "?stream=stderr" : ""
        return try await request(method: "GET", path: "/sandboxes/\(name)/exec/detached/\(cmdId)/logs\(query)")
    }

    /// Kill a detached command.
    public func detachedKill(_ name: String, cmdId: String) async throws -> String {
        try await request(method: "DELETE", path: "/sandboxes/\(name)/exec/detached/\(cmdId)")
    }

    /// List detached commands in a sandbox.
    public func detachedList(_ name: String) async throws -> [DetachedCommand] {
        try await request(method: "GET", path: "/sandboxes/\(name)/exec/detached")
    }

    /// List all orchestrations.
    public func listOrchestrations() async throws -> [Orchestration] {
        try await requestObjectArray(method: "GET", path: "/orchestrations")
    }

    /// Create a new orchestration.
    public func createOrchestration(_ payload: Orchestration) async throws -> Orchestration {
        try await requestObject(method: "POST", path: "/orchestrations", body: payload)
    }

    /// Get an orchestration by identifier.
    public func getOrchestration(_ id: String) async throws -> Orchestration {
        try await requestObject(method: "GET", path: "/orchestrations/\(id)")
    }

    /// Raise an external event for an orchestration.
    public func signalOrchestration(_ id: String, payload: Orchestration) async throws -> Orchestration {
        try await requestObject(method: "POST", path: "/orchestrations/\(id)/events", body: payload)
    }

    /// Terminate an orchestration.
    public func terminateOrchestration(_ id: String, payload: Orchestration = [:]) async throws -> Orchestration {
        try await requestObject(method: "POST", path: "/orchestrations/\(id)/terminate", body: payload)
    }

    /// List orchestration definitions.
    public func listOrchestrationDefinitions() async throws -> [OrchestrationDefinition] {
        try await requestObjectArray(method: "GET", path: "/orchestrations/definitions")
    }

    /// Register or update an orchestration definition.
    public func upsertOrchestrationDefinition(_ payload: OrchestrationDefinition) async throws -> OrchestrationDefinition {
        try await requestObject(method: "POST", path: "/orchestrations/definitions", body: payload)
    }

    /// Get an orchestration definition by name.
    public func getOrchestrationDefinition(_ name: String) async throws -> OrchestrationDefinition {
        try await requestObject(method: "GET", path: "/orchestrations/definitions/\(name)")
    }

    /// Delete an orchestration definition by name.
    public func deleteOrchestrationDefinition(_ name: String) async throws -> String {
        try await request(method: "DELETE", path: "/orchestrations/definitions/\(name)")
    }

    /// List all objects.
    public func listObjects() async throws -> [DurableObject] {
        try await requestObjectArray(method: "GET", path: "/objects")
    }

    /// Create a new object.
    public func createObject(_ payload: DurableObject) async throws -> DurableObject {
        try await requestObject(method: "POST", path: "/objects", body: payload)
    }

    /// Get an object by identifier.
    public func getObject(_ id: String) async throws -> DurableObject {
        try await requestObject(method: "GET", path: "/objects/\(id)")
    }

    /// Call a method on a durable object (auto-creates/wakes if needed).
    public func callObject(class className: String, id objectId: String, method: String, args: [String: Any] = [:]) async throws -> [String: Any] {
        let path = "/objects/\(className)/\(objectId)/call/\(method)"
        return try await requestObject(method: "POST", path: path, body: args)
    }

    /// Delete a durable object by identifier.
    public func deleteObject(_ id: String) async throws -> String {
        try await request(method: "DELETE", path: "/objects/\(id)")
    }

    /// Partially update a durable object (storage and/or status).
    public func patchObject(_ id: String, storage: [String: Any]? = nil, status: String? = nil) async throws -> DurableObject {
        var body: [String: Any] = [:]
        if let s = storage { body["storage"] = s }
        if let st = status { body["status"] = st }
        return try await requestObject(method: "PATCH", path: "/objects/\(id)", body: body)
    }

    /// List all schedules.
    public func listSchedules() async throws -> [Schedule] {
        try await requestObjectArray(method: "GET", path: "/schedules")
    }

    /// Create a new schedule.
    public func createSchedule(_ payload: Schedule) async throws -> Schedule {
        try await requestObject(method: "POST", path: "/schedules", body: payload)
    }

    /// Get a schedule by identifier.
    public func getSchedule(_ id: String) async throws -> Schedule {
        try await requestObject(method: "GET", path: "/schedules/\(id)")
    }

    /// Delete a schedule by identifier.
    public func deleteSchedule(_ id: String) async throws -> String {
        try await request(method: "DELETE", path: "/schedules/\(id)")
    }

    /// List all durable stores.
    public func listStores() async throws -> [DurableStore] {
        try await requestObjectArray(method: "GET", path: "/stores")
    }

    /// Create a durable store.
    public func createStore(_ payload: DurableStore) async throws -> DurableStore {
        try await requestObject(method: "POST", path: "/stores", body: payload)
    }

    /// Get a durable store by identifier.
    public func getStore(_ id: String) async throws -> DurableStore {
        try await requestObject(method: "GET", path: "/stores/\(id)")
    }

    /// Delete a durable store by identifier.
    public func deleteStore(_ id: String) async throws -> String {
        try await request(method: "DELETE", path: "/stores/\(id)")
    }

    /// Run a read query against a durable store.
    public func queryStore(_ id: String, payload: DurableStore) async throws -> DurableStoreQueryResult {
        try await requestObject(method: "POST", path: "/stores/\(id)/query", body: payload)
    }

    /// Run a write statement against a durable store.
    public func executeStore(_ id: String, payload: DurableStore) async throws -> DurableStoreExecuteResult {
        try await requestObject(method: "POST", path: "/stores/\(id)/execute", body: payload)
    }

    /// Run a command against a durable store (Redis-style engines).
    public func commandStore(_ id: String, payload: DurableStore) async throws -> DurableStoreCommandResult {
        try await requestObject(method: "POST", path: "/stores/\(id)/command", body: payload)
    }

    // MARK: - TTL Extension

    /// Extend a sandbox's time-to-live. Returns the new expiry time.
    public func extendTtl(_ name: String, by: String) async throws -> ExtendTtlResponse {
        let body = ExtendTtlRequest(by: by)
        return try await request(method: "POST", path: "/sandboxes/\(name)/extend", body: body)
    }

    // MARK: - Snapshots

    /// List all snapshots.
    public func listSnapshots() async throws -> [SnapshotMeta] {
        try await request(method: "GET", path: "/snapshots")
    }

    /// Take a snapshot of a sandbox.
    public func takeSnapshot(_ opts: TakeSnapshotOptions) async throws -> SnapshotMeta {
        try await request(method: "POST", path: "/snapshots", body: opts)
    }

    /// Get info about a snapshot.
    public func getSnapshot(_ name: String) async throws -> SnapshotMeta {
        try await request(method: "GET", path: "/snapshots/\(name)")
    }

    /// Delete a snapshot.
    public func deleteSnapshot(_ name: String) async throws {
        let _: String = try await request(method: "DELETE", path: "/snapshots/\(name)")
    }

    /// Restore a sandbox from a snapshot.
    public func restoreSnapshot(_ name: String) async throws -> SandboxInfo {
        try await request(method: "POST", path: "/snapshots/\(name)/restore")
    }

    // MARK: - Browser

    /// Create a sandboxed headless browser session.
    ///
    /// Provisions a sandbox with Chromium (via Playwright) installed and returns
    /// a ``BrowserSession`` for navigating pages, taking screenshots, and evaluating JS.
    ///
    /// ```swift
    /// let browser = try await client.browser("my-browser")
    /// let page = try await browser.goto("https://example.com")
    /// print(page.title)
    /// try await browser.remove()
    /// ```
    ///
    /// - Parameters:
    ///   - name: The sandbox name.
    ///   - memoryMB: Memory allocation in MB (default 2048).
    /// - Returns: A ``BrowserSession`` ready for use.
    public func browser(_ name: String, memoryMB: Int = 2048) async throws -> BrowserSession {
        let options = CreateSandboxOptions(
            image: "python:3.12-slim",
            memoryMB: memoryMB,
            profile: .moderate
        )
        _ = try await createSandbox(name, options: options)
        _ = try await execInSandbox(
            name,
            command: ["sh", "-c", "pip install -q playwright && playwright install --with-deps chromium"]
        )
        return BrowserSession(name: name, client: self)
    }

    // MARK: - Internal

    private func request<T: Decodable>(
        method: String,
        path: String
    ) async throws -> T {
        try await request(method: method, path: path, body: nil as AnyEncodable?)
    }

    private func requestObject(
        method: String,
        path: String,
        body: [String: Any]? = nil
    ) async throws -> [String: Any] {
        let url = URL(string: "\(config.baseURL)\(path)")!
        var req = URLRequest(url: url)
        req.httpMethod = method
        applyHeaders(&req)
        if let body {
            req.setValue("application/json", forHTTPHeaderField: "Content-Type")
            req.httpBody = try JSONSerialization.data(withJSONObject: body)
        }

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
        guard let result = json["data"] as? [String: Any] else {
            throw AgentKernelError.server("Missing data field")
        }
        return result
    }

    private func requestObjectArray(
        method: String,
        path: String,
        body: [String: Any]? = nil
    ) async throws -> [[String: Any]] {
        let url = URL(string: "\(config.baseURL)\(path)")!
        var req = URLRequest(url: url)
        req.httpMethod = method
        applyHeaders(&req)
        if let body {
            req.setValue("application/json", forHTTPHeaderField: "Content-Type")
            req.httpBody = try JSONSerialization.data(withJSONObject: body)
        }

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
        guard let result = json["data"] as? [[String: Any]] else {
            throw AgentKernelError.server("Missing data field")
        }
        return result
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
