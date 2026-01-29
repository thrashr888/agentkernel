import Foundation

/// Errors returned by the AgentKernel SDK.
public enum AgentKernelError: Error, Sendable {
    /// 401 Unauthorized.
    case auth(String)
    /// 400 Bad Request.
    case validation(String)
    /// 404 Not Found.
    case notFound(String)
    /// 500+ Server Error.
    case server(String)
    /// Network / connection error.
    case network(Error)
    /// SSE streaming error.
    case stream(String)
    /// JSON encoding/decoding error.
    case json(Error)

    // Sendable conformance for wrapped errors
    enum CodingKeys: CodingKey {}
}

extension AgentKernelError: LocalizedError {
    public var errorDescription: String? {
        switch self {
        case .auth(let msg): return "authentication error: \(msg)"
        case .validation(let msg): return "validation error: \(msg)"
        case .notFound(let msg): return "not found: \(msg)"
        case .server(let msg): return "server error: \(msg)"
        case .network(let err): return "network error: \(err.localizedDescription)"
        case .stream(let msg): return "stream error: \(msg)"
        case .json(let err): return "json error: \(err.localizedDescription)"
        }
    }
}

/// Map an HTTP status code and response body to the appropriate error.
func errorFromStatus(_ status: Int, body: String) -> AgentKernelError {
    let message: String
    if let data = body.data(using: .utf8),
       let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
       let errMsg = json["error"] as? String
    {
        message = errMsg
    } else {
        message = body
    }

    switch status {
    case 400: return .validation(message)
    case 401: return .auth(message)
    case 404: return .notFound(message)
    default: return .server(message)
    }
}
