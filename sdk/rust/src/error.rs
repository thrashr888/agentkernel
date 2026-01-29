/// Errors returned by the agentkernel SDK.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 401 Unauthorized.
    #[error("authentication error: {0}")]
    Auth(String),

    /// 404 Not Found.
    #[error("not found: {0}")]
    NotFound(String),

    /// 400 Bad Request.
    #[error("validation error: {0}")]
    Validation(String),

    /// 500 Internal Server Error.
    #[error("server error: {0}")]
    Server(String),

    /// Network / connection error.
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    /// SSE streaming error.
    #[error("stream error: {0}")]
    Stream(String),

    /// JSON serialization/deserialization error.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Map an HTTP status + body to the appropriate error variant.
pub fn error_from_status(status: u16, body: &str) -> Error {
    let message = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str().map(String::from)))
        .unwrap_or_else(|| body.to_string());

    match status {
        400 => Error::Validation(message),
        401 => Error::Auth(message),
        404 => Error::NotFound(message),
        _ => Error::Server(message),
    }
}
