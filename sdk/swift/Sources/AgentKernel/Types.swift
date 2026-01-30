import Foundation

// MARK: - Enums

/// Security profile for sandbox execution.
public enum SecurityProfile: String, Codable, Sendable {
    case permissive
    case moderate
    case restrictive
}

// MARK: - Request Types

/// Options for the run command.
public struct RunOptions: Sendable {
    public var image: String?
    public var profile: SecurityProfile?
    public var fast: Bool

    public init(image: String? = nil, profile: SecurityProfile? = nil, fast: Bool = true) {
        self.image = image
        self.profile = profile
        self.fast = fast
    }
}

/// Options for creating a sandbox.
public struct CreateSandboxOptions: Sendable {
    public var image: String?
    public var vcpus: Int?
    public var memoryMB: Int?
    public var profile: SecurityProfile?

    public init(image: String? = nil, vcpus: Int? = nil, memoryMB: Int? = nil, profile: SecurityProfile? = nil) {
        self.image = image
        self.vcpus = vcpus
        self.memoryMB = memoryMB
        self.profile = profile
    }
}

// MARK: - Response Types

/// Output from a command execution.
public struct RunOutput: Codable, Sendable {
    public let output: String
}

/// Information about a sandbox.
public struct SandboxInfo: Codable, Sendable {
    public let name: String
    public let status: String
    public let backend: String
    public let image: String?
    public let vcpus: Int?
    public let memory_mb: Int?
    public let created_at: String?
}

/// SSE stream event.
///
/// Uses `@unchecked Sendable` because `[String: Any]` isn't `Sendable`,
/// but all properties are immutable `let` bindings set at init.
public struct StreamEvent: @unchecked Sendable {
    public let eventType: String
    public let data: [String: Any]

    public init(eventType: String, data: [String: Any]) {
        self.eventType = eventType
        self.data = data
    }
}

// MARK: - Internal Types

/// API response wrapper.
struct ApiResponse<T: Decodable>: Decodable {
    let success: Bool
    let data: T?
    let error: String?
}

/// Run request body.
struct RunRequest: Encodable {
    let command: [String]
    let image: String?
    let profile: SecurityProfile?
    let fast: Bool
}

/// Create sandbox request body.
struct CreateRequest: Encodable {
    let name: String
    let image: String?
    let vcpus: Int?
    let memory_mb: Int?
    let profile: SecurityProfile?
}

/// Exec request body.
struct ExecRequest: Encodable {
    let command: [String]
}

/// File write request body.
struct FileWriteRequest: Encodable {
    let content: String
    let encoding: String?
}

/// Response from reading a file.
public struct FileReadResponse: Codable, Sendable {
    public let content: String
    public let encoding: String
    public let size: Int
}

/// A command for batch execution.
public struct BatchCommand: Encodable, Sendable {
    public let command: [String]
    public init(command: [String]) { self.command = command }
}

/// Result of a single batch command.
public struct BatchResult: Codable, Sendable {
    public let output: String?
    public let error: String?
}

/// Response from batch execution.
public struct BatchRunResponse: Codable, Sendable {
    public let results: [BatchResult]
}

/// Batch run request body.
struct BatchRunRequest: Encodable {
    let commands: [BatchCommand]
}

// MARK: - Type Erasure

/// Type-erased `Encodable` wrapper for generic request bodies.
struct AnyEncodable: Encodable {
    private let encode: (Encoder) throws -> Void

    init<T: Encodable>(_ wrapped: T) {
        self.encode = wrapped.encode
    }

    func encode(to encoder: Encoder) throws {
        try encode(encoder)
    }
}
