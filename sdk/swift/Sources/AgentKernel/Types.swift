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

    public init(image: String? = nil) {
        self.image = image
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
}

/// Exec request body.
struct ExecRequest: Encodable {
    let command: [String]
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
