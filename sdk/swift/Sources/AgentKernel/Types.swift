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
    /// Git repository URL to clone into the sandbox.
    public var sourceURL: String?
    /// Git ref to checkout after cloning.
    public var sourceRef: String?
    /// Volume mounts (slug:/path or slug:/path:ro). Create volumes via CLI first.
    public var volumes: [String]?

    public init(
        image: String? = nil,
        vcpus: Int? = nil,
        memoryMB: Int? = nil,
        profile: SecurityProfile? = nil,
        sourceURL: String? = nil,
        sourceRef: String? = nil,
        volumes: [String]? = nil
    ) {
        self.image = image
        self.vcpus = vcpus
        self.memoryMB = memoryMB
        self.profile = profile
        self.sourceURL = sourceURL
        self.sourceRef = sourceRef
        self.volumes = volumes
    }
}

/// Options for executing a command in a sandbox.
public struct ExecOptions: Sendable {
    /// Environment variables (KEY=VALUE).
    public var env: [String]
    /// Working directory inside the container.
    public var workdir: String?
    /// Run as root.
    public var sudo: Bool?

    public init(env: [String] = [], workdir: String? = nil, sudo: Bool? = nil) {
        self.env = env
        self.workdir = workdir
        self.sudo = sudo
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
    let source_url: String?
    let source_ref: String?
    let volumes: [String]?
}

/// Exec request body.
struct ExecRequest: Encodable {
    let command: [String]
    let env: [String]?
    let workdir: String?
    let sudo: Bool?
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

/// Batch file write request body.
struct BatchFileWriteRequest: Encodable {
    let files: [String: String]
}

/// Result of a batch file write.
public struct BatchFileWriteResponse: Codable, Sendable {
    public let written: Int
}

/// Status of a detached command.
public enum DetachedStatus: String, Codable, Sendable {
    case running
    case completed
    case failed
}

/// A detached (background) command running in a sandbox.
public struct DetachedCommand: Codable, Sendable {
    public let id: String
    public let sandbox: String
    public let command: [String]
    public let pid: UInt32
    public let status: DetachedStatus
    public let exit_code: Int32?
    public let started_at: String
}

/// Response from detached command logs.
public struct DetachedLogsResponse: Codable, Sendable {
    public let stdout: String?
    public let stderr: String?
}

// MARK: - TTL Extension Types

/// Response from extending a sandbox's TTL.
public struct ExtendTtlResponse: Codable, Sendable {
    public let expires_at: String?
}

/// Extend TTL request body.
struct ExtendTtlRequest: Encodable {
    let by: String
}

// MARK: - Snapshot Types

/// Metadata for a sandbox snapshot.
public struct SnapshotMeta: Codable, Sendable {
    public let name: String
    public let sandbox: String
    public let image_tag: String
    public let backend: String
    public let base_image: String?
    public let vcpus: Int?
    public let memory_mb: Int?
    public let created_at: String
}

/// Options for taking a snapshot.
public struct TakeSnapshotOptions: Encodable, Sendable {
    public let sandbox: String
    public let name: String?

    public init(sandbox: String, name: String? = nil) {
        self.sandbox = sandbox
        self.name = name
    }
}

// MARK: - Browser Types

/// A link extracted from a web page.
public struct PageLink: Codable, Sendable {
    public let text: String
    public let href: String
}

/// Result of navigating to a web page.
public struct PageResult: Codable, Sendable {
    public let title: String
    public let url: String
    /// Page body text (truncated to ~8KB).
    public let text: String
    /// First 50 links on the page.
    public let links: [PageLink]
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
