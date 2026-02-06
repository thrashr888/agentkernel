//! OpenCode-compatible API endpoints.
//!
//! Implements the OpenCode HTTP API, allowing OpenCode to connect directly to
//! agentkernel as a sandbox backend. Sessions map to agentkernel sandboxes.
//!
//! ## Usage
//!
//! ```bash
//! opencode --api-url http://localhost:18888/opencode
//! ```
//!
//! ## Endpoints
//!
//! | Endpoint | Method | Description |
//! |----------|--------|-------------|
//! | `/session` | GET | List all sessions |
//! | `/session` | POST | Create a new session |
//! | `/session/{id}` | GET | Get session details |
//! | `/session/{id}/message` | POST | Send a message (execute command) |
//! | `/session/{id}/message` | GET | Get message history |
//! | `/event` | GET | SSE stream for session events |
//! | `/global/event` | GET | SSE stream for global events |
//! | `/permission` | GET | List pending permissions (stub) |
//! | `/permission/{id}/reply` | POST | Reply to permission (stub) |
//! | `/question` | GET | List pending questions (stub) |
//! | `/question/{id}/reply` | POST | Reply to question (stub) |

use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Method, Request, Response, StatusCode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::vmm::VmManager;

type BoxBody = http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>;

fn full<T: Into<bytes::Bytes>>(chunk: T) -> BoxBody {
    Full::new(chunk.into())
        .map_err(|never| match never {})
        .boxed()
}

// ============================================================================
// OpenCode Types (matching opencode-sdk types.gen.ts)
// ============================================================================

/// Session represents an OpenCode session (maps to agentkernel sandbox).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub title: String,
    pub version: u32,
    pub time: SessionTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTime {
    pub created: String,
    pub updated: String,
}

/// Message types for OpenCode protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
    User(UserMessage),
    Assistant(AssistantMessage),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserMessage {
    pub id: String,
    pub session_id: String,
    pub time: MessageTimeCreated,
    pub parts: Vec<Part>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessage {
    pub id: String,
    pub session_id: String,
    pub time: MessageTime,
    pub parts: Vec<Part>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<TokenUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageTimeCreated {
    pub created: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageTime {
    pub created: String,
    pub completed: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<u64>,
}

/// Message parts (union type in OpenCode).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text(TextPart),
    Tool(ToolPart),
    File(FilePart),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextPart {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPart {
    pub id: String,
    pub tool: String,
    pub input: serde_json::Value,
    pub state: ToolState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolState {
    Pending,
    Running,
    Completed,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilePart {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

/// SSE Event types
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)]
pub enum Event {
    SessionCreated { session: Session },
    SessionUpdated { session: Session },
    MessageCreated { message: Message },
    MessageUpdated { message: Message },
}

// ============================================================================
// Request/Response types
// ============================================================================

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionRequest {
    #[serde(default)]
    pub directory: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SendMessageRequest {
    pub content: String,
    #[serde(default)]
    pub tool: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct PermissionReply {
    pub allow: bool,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct QuestionReply {
    pub answer: String,
}

// ============================================================================
// OpenCode State (session tracking)
// ============================================================================

/// Tracks OpenCode sessions and their message history.
pub struct OpenCodeState {
    sessions: RwLock<HashMap<String, SessionData>>,
}

struct SessionData {
    session: Session,
    messages: Vec<Message>,
    sandbox_name: String,
}

impl OpenCodeState {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for OpenCodeState {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Route handler
// ============================================================================

/// Handle OpenCode API requests.
///
/// Routes requests under `/opencode/...` to the appropriate handler.
pub async fn handle_opencode_request(
    req: Request<Incoming>,
    path_suffix: &str,
    state: Arc<OpenCodeState>,
) -> Response<BoxBody> {
    let method = req.method().clone();
    let segments: Vec<&str> = path_suffix.split('/').filter(|s| !s.is_empty()).collect();

    match (method, segments.as_slice()) {
        // Session endpoints
        (Method::GET, ["session"]) => handle_list_sessions(state).await,
        (Method::POST, ["session"]) => handle_create_session(req, state).await,
        (Method::GET, ["session", id]) => handle_get_session(id, state).await,
        (Method::POST, ["session", id, "message"]) => handle_send_message(req, id, state).await,
        (Method::GET, ["session", id, "message"]) => handle_get_messages(id, state).await,

        // SSE event streams
        (Method::GET, ["event"]) => handle_event_stream(req, state).await,
        (Method::GET, ["global", "event"]) => handle_global_event_stream().await,

        // Permission endpoints (stubs - agentkernel auto-approves)
        (Method::GET, ["permission"]) => handle_list_permissions().await,
        (Method::POST, ["permission", id, "reply"]) => handle_permission_reply(req, id).await,

        // Question endpoints (stubs - agentkernel doesn't ask questions)
        (Method::GET, ["question"]) => handle_list_questions().await,
        (Method::POST, ["question", id, "reply"]) => handle_question_reply(req, id).await,

        // Provider/Agent/Config stubs
        (Method::GET, ["provider"]) => json_response(StatusCode::OK, &serde_json::json!([])),
        (Method::GET, ["agent"]) => json_response(
            StatusCode::OK,
            &serde_json::json!([{"id": "sandbox", "name": "Sandbox Agent"}]),
        ),
        (Method::GET, ["config"]) => json_response(StatusCode::OK, &serde_json::json!({})),

        _ => json_response(
            StatusCode::NOT_FOUND,
            &serde_json::json!({"error": "Not found"}),
        ),
    }
}

// ============================================================================
// Handlers
// ============================================================================

async fn handle_list_sessions(state: Arc<OpenCodeState>) -> Response<BoxBody> {
    let sessions = state.sessions.read().await;
    let list: Vec<&Session> = sessions.values().map(|sd| &sd.session).collect();
    json_response(StatusCode::OK, &list)
}

async fn handle_create_session(
    req: Request<Incoming>,
    state: Arc<OpenCodeState>,
) -> Response<BoxBody> {
    let body: CreateSessionRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    // Generate session ID
    let id = format!(
        "session-{}",
        uuid::Uuid::new_v4().to_string().split('-').next().unwrap()
    );
    let sandbox_name = format!("opencode-{}", &id[8..]);

    // Create sandbox
    let mut manager = match VmManager::new() {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &serde_json::json!({"error": e.to_string()}),
            );
        }
    };

    if let Err(e) = manager
        .create(&sandbox_name, "node:22-alpine", 1, 512)
        .await
    {
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &serde_json::json!({"error": e.to_string()}),
        );
    }

    let perms = crate::permissions::SecurityProfile::Moderate.permissions();
    if let Err(e) = manager.start_with_permissions(&sandbox_name, &perms).await {
        let _ = manager.remove(&sandbox_name).await;
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &serde_json::json!({"error": e.to_string()}),
        );
    }

    let now = chrono::Utc::now().to_rfc3339();
    let session = Session {
        id: id.clone(),
        project_id: None,
        directory: body.directory,
        parent_id: body.parent_id,
        summary: None,
        title: body.title.unwrap_or_else(|| "New Session".to_string()),
        version: 1,
        time: SessionTime {
            created: now.clone(),
            updated: now,
        },
    };

    let mut sessions = state.sessions.write().await;
    sessions.insert(
        id.clone(),
        SessionData {
            session: session.clone(),
            messages: Vec::new(),
            sandbox_name,
        },
    );

    json_response(StatusCode::CREATED, &session)
}

async fn handle_get_session(id: &str, state: Arc<OpenCodeState>) -> Response<BoxBody> {
    let sessions = state.sessions.read().await;
    match sessions.get(id) {
        Some(sd) => json_response(StatusCode::OK, &sd.session),
        None => json_response(
            StatusCode::NOT_FOUND,
            &serde_json::json!({"error": "Session not found"}),
        ),
    }
}

async fn handle_send_message(
    req: Request<Incoming>,
    session_id: &str,
    state: Arc<OpenCodeState>,
) -> Response<BoxBody> {
    let body: SendMessageRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    // Get sandbox name
    let sandbox_name = {
        let sessions = state.sessions.read().await;
        match sessions.get(session_id) {
            Some(sd) => sd.sandbox_name.clone(),
            None => {
                return json_response(
                    StatusCode::NOT_FOUND,
                    &serde_json::json!({"error": "Session not found"}),
                );
            }
        }
    };

    // Create user message
    let user_msg_id = format!(
        "msg-{}",
        uuid::Uuid::new_v4().to_string().split('-').next().unwrap()
    );
    let now = chrono::Utc::now().to_rfc3339();

    let user_message = Message::User(UserMessage {
        id: user_msg_id.clone(),
        session_id: session_id.to_string(),
        time: MessageTimeCreated {
            created: now.clone(),
        },
        parts: vec![Part::Text(TextPart {
            text: body.content.clone(),
        })],
        agent: Some("sandbox".to_string()),
        model: None,
    });

    // Determine if this is a tool call or text message
    let (output, tool_used) = if body.tool.as_deref() == Some("bash")
        || body.tool.as_deref() == Some("shell")
        || body.content.starts_with("$ ")
        || body.content.starts_with("```bash")
    {
        // Execute as shell command
        let cmd = if body.content.starts_with("$ ") {
            body.content[2..].to_string()
        } else if body.content.starts_with("```bash") {
            // Extract command from code block
            body.content
                .lines()
                .skip(1)
                .take_while(|l| !l.starts_with("```"))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            body.content.clone()
        };

        let mut manager = match VmManager::new() {
            Ok(m) => m,
            Err(e) => {
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &serde_json::json!({"error": e.to_string()}),
                );
            }
        };

        let cmd_args: Vec<String> = vec!["sh".to_string(), "-c".to_string(), cmd.clone()];
        match manager.exec_cmd(&sandbox_name, &cmd_args).await {
            Ok(result) => (result, Some("bash".to_string())),
            Err(e) => (format!("Error: {}", e), Some("bash".to_string())),
        }
    } else {
        // Plain text - just echo back
        (format!("Received: {}", body.content), None)
    };

    let completed = chrono::Utc::now().to_rfc3339();
    let assistant_msg_id = format!(
        "msg-{}",
        uuid::Uuid::new_v4().to_string().split('-').next().unwrap()
    );

    let parts = if let Some(tool) = tool_used {
        vec![Part::Tool(ToolPart {
            id: format!("tool-{}", &assistant_msg_id[4..]),
            tool,
            input: serde_json::json!({"command": body.content}),
            state: ToolState::Completed,
            output: Some(output.clone()),
            metadata: None,
        })]
    } else {
        vec![Part::Text(TextPart { text: output })]
    };

    let assistant_message = Message::Assistant(AssistantMessage {
        id: assistant_msg_id,
        session_id: session_id.to_string(),
        time: MessageTime {
            created: now,
            completed,
        },
        parts,
        cost: None,
        tokens: None,
        finish: Some("stop".to_string()),
    });

    // Store messages
    {
        let mut sessions = state.sessions.write().await;
        if let Some(sd) = sessions.get_mut(session_id) {
            sd.messages.push(user_message);
            sd.messages.push(assistant_message.clone());
            sd.session.version += 1;
            sd.session.time.updated = chrono::Utc::now().to_rfc3339();
        }
    }

    json_response(StatusCode::OK, &assistant_message)
}

async fn handle_get_messages(id: &str, state: Arc<OpenCodeState>) -> Response<BoxBody> {
    let sessions = state.sessions.read().await;
    match sessions.get(id) {
        Some(sd) => json_response(StatusCode::OK, &sd.messages),
        None => json_response(
            StatusCode::NOT_FOUND,
            &serde_json::json!({"error": "Session not found"}),
        ),
    }
}

async fn handle_event_stream(
    req: Request<Incoming>,
    state: Arc<OpenCodeState>,
) -> Response<BoxBody> {
    // Parse session filter from query
    let session_filter = req.uri().query().and_then(|q| {
        q.split('&')
            .find_map(|p| p.strip_prefix("sessionId="))
            .map(|s| s.to_string())
    });

    // Return SSE stream with current sessions
    let sessions = state.sessions.read().await;
    let mut events = String::new();

    for sd in sessions.values() {
        if let Some(ref filter) = session_filter
            && &sd.session.id != filter
        {
            continue;
        }
        let event = Event::SessionCreated {
            session: sd.session.clone(),
        };
        events.push_str(&format!(
            "event: session_created\ndata: {}\n\n",
            serde_json::to_string(&event).unwrap_or_default()
        ));
    }

    // Keep connection open with heartbeat
    events.push_str("event: heartbeat\ndata: {}\n\n");

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .body(full(events))
        .unwrap()
}

async fn handle_global_event_stream() -> Response<BoxBody> {
    // Return SSE stream with heartbeat
    let events = "event: heartbeat\ndata: {}\n\n";

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .body(full(events))
        .unwrap()
}

async fn handle_list_permissions() -> Response<BoxBody> {
    // agentkernel auto-approves everything - no pending permissions
    json_response(StatusCode::OK, &serde_json::json!([]))
}

async fn handle_permission_reply(req: Request<Incoming>, _id: &str) -> Response<BoxBody> {
    let _body: PermissionReply = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    // No-op since we auto-approve
    json_response(StatusCode::OK, &serde_json::json!({"status": "ok"}))
}

async fn handle_list_questions() -> Response<BoxBody> {
    // agentkernel doesn't ask questions
    json_response(StatusCode::OK, &serde_json::json!([]))
}

async fn handle_question_reply(req: Request<Incoming>, _id: &str) -> Response<BoxBody> {
    let _body: QuestionReply = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    json_response(StatusCode::OK, &serde_json::json!({"status": "ok"}))
}

// ============================================================================
// Helpers
// ============================================================================

fn json_response<T: Serialize>(status: StatusCode, data: &T) -> Response<BoxBody> {
    let body = serde_json::to_string(data).unwrap_or_else(|_| "{}".to_string());
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(full(body))
        .unwrap()
}

async fn read_json_body<T: for<'de> Deserialize<'de>>(
    req: Request<Incoming>,
) -> Result<T, Response<BoxBody>> {
    let body_bytes = req
        .collect()
        .await
        .map_err(|_| {
            json_response(
                StatusCode::BAD_REQUEST,
                &serde_json::json!({"error": "Failed to read body"}),
            )
        })?
        .to_bytes();

    serde_json::from_slice(&body_bytes).map_err(|e| {
        json_response(
            StatusCode::BAD_REQUEST,
            &serde_json::json!({"error": format!("Invalid JSON: {}", e)}),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_serialize() {
        let session = Session {
            id: "session-abc123".to_string(),
            project_id: None,
            directory: Some("/workspace".to_string()),
            parent_id: None,
            summary: None,
            title: "Test Session".to_string(),
            version: 1,
            time: SessionTime {
                created: "2026-02-05T12:00:00Z".to_string(),
                updated: "2026-02-05T12:00:00Z".to_string(),
            },
        };
        let json = serde_json::to_string(&session).unwrap();
        assert!(json.contains("\"id\":\"session-abc123\""));
        assert!(json.contains("\"title\":\"Test Session\""));
        assert!(json.contains("\"directory\":\"/workspace\""));
    }

    #[test]
    fn test_message_user_serialize() {
        let msg = Message::User(UserMessage {
            id: "msg-123".to_string(),
            session_id: "session-abc".to_string(),
            time: MessageTimeCreated {
                created: "2026-02-05T12:00:00Z".to_string(),
            },
            parts: vec![Part::Text(TextPart {
                text: "Hello".to_string(),
            })],
            agent: Some("sandbox".to_string()),
            model: None,
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"role\":\"user\""));
        assert!(json.contains("\"id\":\"msg-123\""));
    }

    #[test]
    fn test_message_assistant_serialize() {
        let msg = Message::Assistant(AssistantMessage {
            id: "msg-456".to_string(),
            session_id: "session-abc".to_string(),
            time: MessageTime {
                created: "2026-02-05T12:00:00Z".to_string(),
                completed: "2026-02-05T12:00:01Z".to_string(),
            },
            parts: vec![Part::Text(TextPart {
                text: "Done".to_string(),
            })],
            cost: Some(0.001),
            tokens: None,
            finish: Some("stop".to_string()),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"role\":\"assistant\""));
        assert!(json.contains("\"finish\":\"stop\""));
    }

    #[test]
    fn test_tool_part_serialize() {
        let part = Part::Tool(ToolPart {
            id: "tool-1".to_string(),
            tool: "bash".to_string(),
            input: serde_json::json!({"command": "ls"}),
            state: ToolState::Completed,
            output: Some("file.txt".to_string()),
            metadata: None,
        });
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains("\"type\":\"tool\""));
        assert!(json.contains("\"tool\":\"bash\""));
        assert!(json.contains("\"state\":\"completed\""));
    }

    #[test]
    fn test_create_session_request_deserialize() {
        let json = r#"{"directory": "/home/user", "title": "My Session"}"#;
        let req: CreateSessionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.directory, Some("/home/user".to_string()));
        assert_eq!(req.title, Some("My Session".to_string()));
    }

    #[test]
    fn test_send_message_request_deserialize() {
        let json = r#"{"content": "echo hello", "tool": "bash"}"#;
        let req: SendMessageRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.content, "echo hello");
        assert_eq!(req.tool, Some("bash".to_string()));
    }
}
