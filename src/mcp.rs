//! MCP (Model Context Protocol) server implementation.
//!
//! Provides JSON-RPC 2.0 over stdio for integration with Claude Code and other MCP clients.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use tokio::runtime::Handle;

use crate::languages;
use crate::permissions::{CompatibilityMode, SecurityProfile};
use crate::vmm::VmManager;

/// MCP server for agentkernel
pub struct McpServer {
    initialized: bool,
}

// JSON-RPC 2.0 types
#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl McpServer {
    pub fn new() -> Self {
        Self { initialized: false }
    }

    /// Run the MCP server (reads from stdin, writes to stdout)
    pub fn run(&mut self) -> Result<()> {
        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout();
        let reader = BufReader::new(stdin.lock());

        eprintln!("agentkernel MCP server started");

        for line in reader.lines() {
            let line = line.context("Failed to read from stdin")?;
            if line.is_empty() {
                continue;
            }

            // Parse JSON-RPC request
            let request: JsonRpcRequest = match serde_json::from_str(&line) {
                Ok(req) => req,
                Err(e) => {
                    let error_response = JsonRpcResponse {
                        jsonrpc: "2.0",
                        id: Value::Null,
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32700,
                            message: format!("Parse error: {}", e),
                            data: None,
                        }),
                    };
                    writeln!(stdout, "{}", serde_json::to_string(&error_response)?)?;
                    stdout.flush()?;
                    continue;
                }
            };

            // Handle the request
            let response = self.handle_request(&request);

            // Only send response if there was an id (not a notification)
            if request.id.is_some() {
                writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
                stdout.flush()?;
            }
        }

        Ok(())
    }

    fn handle_request(&mut self, request: &JsonRpcRequest) -> JsonRpcResponse {
        let id = request.id.clone().unwrap_or(Value::Null);

        match request.method.as_str() {
            "initialize" => self.handle_initialize(id, &request.params),
            "initialized" => {
                // Notification, no response needed
                JsonRpcResponse {
                    jsonrpc: "2.0",
                    id,
                    result: Some(Value::Null),
                    error: None,
                }
            }
            "tools/list" => self.handle_tools_list(id),
            "tools/call" => self.handle_tools_call(id, &request.params),
            "shutdown" => {
                self.initialized = false;
                JsonRpcResponse {
                    jsonrpc: "2.0",
                    id,
                    result: Some(Value::Null),
                    error: None,
                }
            }
            _ => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: None,
                error: Some(JsonRpcError {
                    code: -32601,
                    message: format!("Method not found: {}", request.method),
                    data: None,
                }),
            },
        }
    }

    fn handle_initialize(&mut self, id: Value, _params: &Value) -> JsonRpcResponse {
        self.initialized = true;

        JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "agentkernel",
                    "version": env!("CARGO_PKG_VERSION")
                }
            })),
            error: None,
        }
    }

    fn handle_tools_list(&self, id: Value) -> JsonRpcResponse {
        let tools = json!({
            "tools": [
                {
                    "name": "sandbox_run",
                    "description": "Run a command in an isolated sandbox (SAFE: executes in isolation, cannot affect host). By default uses a pre-warmed container pool for fast execution (~50ms). Set fast=false for custom images or advanced options.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "command": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "The command and arguments to run (e.g., [\"python\", \"script.py\"] or [\"npm\", \"test\"])"
                            },
                            "image": {
                                "type": "string",
                                "description": "Docker image to use (only when fast=false). If not specified, auto-detected from command."
                            },
                            "fast": {
                                "type": "boolean",
                                "description": "Use container pool for fast execution (default: true). Set to false for custom images.",
                                "default": true
                            },
                            "cwd": {
                                "type": "string",
                                "description": "Working directory inside the sandbox (only when fast=false)"
                            },
                            "env": {
                                "type": "object",
                                "description": "Environment variables to set (only when fast=false)",
                                "additionalProperties": { "type": "string" }
                            },
                            "timeout_ms": {
                                "type": "integer",
                                "description": "Timeout in milliseconds (default: 30000)",
                                "default": 30000
                            },
                            "profile": {
                                "type": "string",
                                "enum": ["permissive", "moderate", "restrictive"],
                                "description": "Security profile (default: moderate). Only when fast=false.",
                                "default": "moderate"
                            },
                            "network": {
                                "type": "boolean",
                                "description": "Enable network access (default: depends on profile). Only when fast=false."
                            },
                            "compatibility_mode": {
                                "type": "string",
                                "enum": ["native", "claude", "codex", "gemini"],
                                "description": "Agent compatibility mode with preset permissions and network policies. Only when fast=false.",
                                "default": "native"
                            }
                        },
                        "required": ["command"]
                    }
                },
                {
                    "name": "sandbox_create",
                    "description": "Create a new persistent sandbox for running multiple commands (creates isolated container resource).",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name for the sandbox"
                            },
                            "image": {
                                "type": "string",
                                "description": "Docker image to use (default: alpine:3.20)"
                            }
                        },
                        "required": ["name"]
                    }
                },
                {
                    "name": "sandbox_exec",
                    "description": "Execute a command in an existing running sandbox (SAFE: executes in isolation).",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the sandbox"
                            },
                            "command": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "The command and arguments to run"
                            }
                        },
                        "required": ["name", "command"]
                    }
                },
                {
                    "name": "sandbox_list",
                    "description": "List all sandboxes and their status (SAFE: read-only operation).",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                },
                {
                    "name": "sandbox_remove",
                    "description": "Remove a sandbox (deletes container resource).",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the sandbox to remove"
                            }
                        },
                        "required": ["name"]
                    }
                },
                {
                    "name": "sandbox_file_write",
                    "description": "Write a file into a running sandbox (writes to sandbox only, cannot affect host filesystem).",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the sandbox"
                            },
                            "path": {
                                "type": "string",
                                "description": "Path inside the sandbox where to write the file"
                            },
                            "content": {
                                "type": "string",
                                "description": "Content to write to the file"
                            }
                        },
                        "required": ["name", "path", "content"]
                    }
                },
                {
                    "name": "sandbox_file_read",
                    "description": "Read a file from a running sandbox (SAFE: reads from sandbox only).",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the sandbox"
                            },
                            "path": {
                                "type": "string",
                                "description": "Path inside the sandbox to read"
                            }
                        },
                        "required": ["name", "path"]
                    }
                },
                {
                    "name": "sandbox_start",
                    "description": "Start a stopped sandbox (SAFE: starts existing isolated container).",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the sandbox to start"
                            }
                        },
                        "required": ["name"]
                    }
                },
                {
                    "name": "sandbox_stop",
                    "description": "Stop a running sandbox (SAFE: stops isolated container, keeps for later use).",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the sandbox to stop"
                            }
                        },
                        "required": ["name"]
                    }
                }
            ]
        });

        JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(tools),
            error: None,
        }
    }

    fn handle_tools_call(&self, id: Value, params: &Value) -> JsonRpcResponse {
        let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

        let result = match tool_name {
            "sandbox_run" => self.tool_sandbox_run(&arguments),
            "sandbox_create" => self.tool_sandbox_create(&arguments),
            "sandbox_exec" => self.tool_sandbox_exec(&arguments),
            "sandbox_list" => self.tool_sandbox_list(),
            "sandbox_remove" => self.tool_sandbox_remove(&arguments),
            "sandbox_file_write" => self.tool_sandbox_file_write(&arguments),
            "sandbox_file_read" => self.tool_sandbox_file_read(&arguments),
            "sandbox_start" => self.tool_sandbox_start(&arguments),
            "sandbox_stop" => self.tool_sandbox_stop(&arguments),
            _ => Err(anyhow::anyhow!("Unknown tool: {}", tool_name)),
        };

        match result {
            Ok(content) => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(json!({
                    "content": [{
                        "type": "text",
                        "text": content
                    }]
                })),
                error: None,
            },
            Err(e) => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Error: {}", e)
                    }],
                    "isError": true
                })),
                error: None,
            },
        }
    }

    fn tool_sandbox_run(&self, args: &Value) -> Result<String> {
        let command: Vec<String> = args
            .get("command")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        if command.is_empty() {
            anyhow::bail!("command is required");
        }

        // Default to fast mode (use container pool)
        let fast = args.get("fast").and_then(|v| v.as_bool()).unwrap_or(true);

        // Fast path: use container pool (default)
        if fast {
            if args.get("image").is_some() {
                eprintln!("Warning: custom image ignored in fast mode (pool uses alpine:3.20)");
            }

            return tokio::task::block_in_place(|| {
                Handle::current().block_on(async { VmManager::run_pooled(&command).await })
            });
        }

        // Slow path: full sandbox lifecycle (when fast=false or custom image needed)
        let image = args
            .get("image")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| languages::detect_image(&command));

        // Check for compatibility mode first (takes precedence over profile)
        let mut perms =
            if let Some(mode_str) = args.get("compatibility_mode").and_then(|v| v.as_str()) {
                let mode = CompatibilityMode::from_str(mode_str).unwrap_or_default();
                let profile = mode.profile();
                eprintln!(
                    "Using {} compatibility mode (API: {:?})",
                    mode_str, profile.api_key_env
                );
                profile.permissions
            } else {
                // Fall back to security profile
                let profile_str = args
                    .get("profile")
                    .and_then(|v| v.as_str())
                    .unwrap_or("moderate");

                SecurityProfile::from_str(profile_str)
                    .unwrap_or_default()
                    .permissions()
            };

        // Apply network override if specified (overrides both mode and profile)
        if let Some(network) = args.get("network").and_then(|v| v.as_bool()) {
            perms.network = network;
        }

        // Use the current runtime via block_in_place
        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;

                // Use optimized ephemeral run with permissions
                manager
                    .run_ephemeral_with_files(&image, &command, &perms, &[])
                    .await
            })
        })
    }

    fn tool_sandbox_create(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;

        let image = args
            .get("image")
            .and_then(|v| v.as_str())
            .unwrap_or("alpine:3.20");

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;
                manager.create(name, image, 1, 512).await?;
                manager.start(name).await?;
                Ok(format!(
                    "Sandbox '{}' created and started with image '{}'",
                    name, image
                ))
            })
        })
    }

    fn tool_sandbox_exec(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;

        let command: Vec<String> = args
            .get("command")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        if command.is_empty() {
            anyhow::bail!("command is required");
        }

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;
                manager.exec_cmd(name, &command).await
            })
        })
    }

    fn tool_sandbox_list(&self) -> Result<String> {
        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let manager = VmManager::new()?;
                let sandboxes = manager.list();

                if sandboxes.is_empty() {
                    return Ok("No sandboxes found.".to_string());
                }

                let mut output = String::from("NAME\tSTATUS\n");
                for (name, running) in sandboxes {
                    let status = if running { "running" } else { "stopped" };
                    output.push_str(&format!("{}\t{}\n", name, status));
                }
                Ok(output)
            })
        })
    }

    fn tool_sandbox_remove(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;
                manager.remove(name).await?;
                Ok(format!("Sandbox '{}' removed.", name))
            })
        })
    }

    fn tool_sandbox_file_write(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;

        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("path is required"))?;

        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("content is required"))?;

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;

                if !manager.is_running(name) {
                    anyhow::bail!(
                        "Sandbox '{}' is not running. Start it first with sandbox_start.",
                        name
                    );
                }

                manager.write_file(name, path, content.as_bytes()).await?;
                Ok(format!(
                    "Wrote {} bytes to '{}' in sandbox '{}'",
                    content.len(),
                    path,
                    name
                ))
            })
        })
    }

    fn tool_sandbox_file_read(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;

        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("path is required"))?;

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;

                if !manager.is_running(name) {
                    anyhow::bail!(
                        "Sandbox '{}' is not running. Start it first with sandbox_start.",
                        name
                    );
                }

                let content = manager.read_file(name, path).await?;

                // Try to convert to UTF-8 string, fall back to base64 for binary
                match String::from_utf8(content.clone()) {
                    Ok(text) => Ok(text),
                    Err(_) => {
                        use base64::{Engine, engine::general_purpose::STANDARD};
                        Ok(format!(
                            "[binary file, {} bytes, base64 encoded]\n{}",
                            content.len(),
                            STANDARD.encode(&content)
                        ))
                    }
                }
            })
        })
    }

    fn tool_sandbox_start(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;

                if !manager.exists(name) {
                    anyhow::bail!(
                        "Sandbox '{}' not found. Create it first with sandbox_create.",
                        name
                    );
                }

                if manager.is_running(name) {
                    return Ok(format!("Sandbox '{}' is already running.", name));
                }

                manager.start(name).await?;
                Ok(format!("Sandbox '{}' started.", name))
            })
        })
    }

    fn tool_sandbox_stop(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;

                if !manager.exists(name) {
                    anyhow::bail!("Sandbox '{}' not found.", name);
                }

                if !manager.is_running(name) {
                    return Ok(format!("Sandbox '{}' is already stopped.", name));
                }

                manager.stop(name).await?;
                Ok(format!("Sandbox '{}' stopped.", name))
            })
        })
    }
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new()
    }
}

/// Run the MCP server
pub async fn run_server() -> Result<()> {
    let mut server = McpServer::new();
    server.run()
}

#[cfg(test)]
mod tests {
    use super::*;

    // === McpServer tests ===

    #[test]
    fn test_mcp_server_new() {
        let server = McpServer::new();
        assert!(!server.initialized);
    }

    #[test]
    fn test_mcp_server_default() {
        let server = McpServer::default();
        assert!(!server.initialized);
    }

    // === JsonRpcResponse tests ===

    #[test]
    fn test_json_rpc_response_serialize_result() {
        let response = JsonRpcResponse {
            jsonrpc: "2.0",
            id: Value::Number(1.into()),
            result: Some(json!({"key": "value"})),
            error: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"id\":1"));
        assert!(json.contains("\"result\":{\"key\":\"value\"}"));
        assert!(!json.contains("\"error\"")); // error skipped when None
    }

    #[test]
    fn test_json_rpc_response_serialize_error() {
        let response = JsonRpcResponse {
            jsonrpc: "2.0",
            id: Value::Null,
            result: None,
            error: Some(JsonRpcError {
                code: -32700,
                message: "Parse error".to_string(),
                data: None,
            }),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"error\""));
        assert!(json.contains("\"code\":-32700"));
        assert!(json.contains("\"message\":\"Parse error\""));
        assert!(!json.contains("\"result\"")); // result skipped when None
    }

    // === JsonRpcError tests ===

    #[test]
    fn test_json_rpc_error_serialize() {
        let error = JsonRpcError {
            code: -32601,
            message: "Method not found".to_string(),
            data: Some(json!({"method": "unknown"})),
        };
        let json = serde_json::to_string(&error).unwrap();
        assert!(json.contains("\"code\":-32601"));
        assert!(json.contains("\"message\":\"Method not found\""));
        assert!(json.contains("\"data\":{\"method\":\"unknown\"}"));
    }

    #[test]
    fn test_json_rpc_error_serialize_no_data() {
        let error = JsonRpcError {
            code: -32600,
            message: "Invalid request".to_string(),
            data: None,
        };
        let json = serde_json::to_string(&error).unwrap();
        assert!(json.contains("\"code\":-32600"));
        assert!(!json.contains("\"data\"")); // data skipped when None
    }

    // === JsonRpcRequest tests ===

    #[test]
    fn test_json_rpc_request_deserialize() {
        let json = r#"{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.method, "initialize");
        assert_eq!(req.id, Some(Value::Number(1.into())));
    }

    #[test]
    fn test_json_rpc_request_deserialize_without_params() {
        let json = r#"{"jsonrpc": "2.0", "id": 2, "method": "tools/list"}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.method, "tools/list");
        assert_eq!(req.params, Value::Null); // defaults to Null
    }

    #[test]
    fn test_json_rpc_request_deserialize_notification() {
        let json = r#"{"jsonrpc": "2.0", "method": "initialized"}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.method, "initialized");
        assert!(req.id.is_none());
    }

    // === handle_initialize tests ===

    #[test]
    fn test_handle_initialize() {
        let mut server = McpServer::new();
        assert!(!server.initialized);

        let response = server.handle_initialize(Value::Number(1.into()), &json!({}));

        assert!(server.initialized);
        assert!(response.error.is_none());
        assert!(response.result.is_some());

        let result = response.result.unwrap();
        assert!(result.get("protocolVersion").is_some());
        assert!(result.get("capabilities").is_some());
        assert!(result.get("serverInfo").is_some());
    }

    // === handle_tools_list tests ===

    #[test]
    fn test_handle_tools_list() {
        let server = McpServer::new();
        let response = server.handle_tools_list(Value::Number(1.into()));

        assert!(response.error.is_none());
        assert!(response.result.is_some());

        let result = response.result.unwrap();
        let tools = result.get("tools").and_then(|t| t.as_array()).unwrap();

        // Check that all expected tools are present
        let tool_names: Vec<&str> = tools
            .iter()
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
            .collect();

        assert!(tool_names.contains(&"sandbox_run"));
        assert!(tool_names.contains(&"sandbox_create"));
        assert!(tool_names.contains(&"sandbox_exec"));
        assert!(tool_names.contains(&"sandbox_list"));
        assert!(tool_names.contains(&"sandbox_remove"));
        assert!(tool_names.contains(&"sandbox_file_write"));
        assert!(tool_names.contains(&"sandbox_file_read"));
        assert!(tool_names.contains(&"sandbox_start"));
        assert!(tool_names.contains(&"sandbox_stop"));
    }

    // === handle_request tests ===

    #[test]
    fn test_handle_request_method_not_found() {
        let mut server = McpServer::new();
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(Value::Number(1.into())),
            method: "unknown_method".to_string(),
            params: Value::Null,
        };

        let response = server.handle_request(&request);
        assert!(response.error.is_some());

        let error = response.error.unwrap();
        assert_eq!(error.code, -32601);
        assert!(error.message.contains("Method not found"));
    }

    #[test]
    fn test_handle_request_shutdown() {
        let mut server = McpServer::new();
        server.initialized = true;

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(Value::Number(1.into())),
            method: "shutdown".to_string(),
            params: Value::Null,
        };

        let response = server.handle_request(&request);

        assert!(!server.initialized);
        assert!(response.error.is_none());
        assert_eq!(response.result, Some(Value::Null));
    }

    #[test]
    fn test_handle_request_initialized_notification() {
        let mut server = McpServer::new();
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(Value::Number(1.into())),
            method: "initialized".to_string(),
            params: Value::Null,
        };

        let response = server.handle_request(&request);
        assert!(response.error.is_none());
    }

    // === Tool parameter validation tests ===

    #[test]
    fn test_tool_sandbox_run_missing_command() {
        let server = McpServer::new();
        let result = server.tool_sandbox_run(&json!({}));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("command is required")
        );
    }

    #[test]
    fn test_tool_sandbox_run_empty_command() {
        let server = McpServer::new();
        let result = server.tool_sandbox_run(&json!({"command": []}));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("command is required")
        );
    }

    #[test]
    fn test_tool_sandbox_create_missing_name() {
        let server = McpServer::new();
        let result = server.tool_sandbox_create(&json!({}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("name is required"));
    }

    #[test]
    fn test_tool_sandbox_exec_missing_name() {
        let server = McpServer::new();
        let result = server.tool_sandbox_exec(&json!({"command": ["ls"]}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("name is required"));
    }

    #[test]
    fn test_tool_sandbox_exec_missing_command() {
        let server = McpServer::new();
        let result = server.tool_sandbox_exec(&json!({"name": "test"}));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("command is required")
        );
    }

    #[test]
    fn test_tool_sandbox_remove_missing_name() {
        let server = McpServer::new();
        let result = server.tool_sandbox_remove(&json!({}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("name is required"));
    }

    #[test]
    fn test_tool_sandbox_file_write_missing_name() {
        let server = McpServer::new();
        let result = server.tool_sandbox_file_write(&json!({"path": "/test", "content": "x"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("name is required"));
    }

    #[test]
    fn test_tool_sandbox_file_write_missing_path() {
        let server = McpServer::new();
        let result = server.tool_sandbox_file_write(&json!({"name": "test", "content": "x"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("path is required"));
    }

    #[test]
    fn test_tool_sandbox_file_write_missing_content() {
        let server = McpServer::new();
        let result = server.tool_sandbox_file_write(&json!({"name": "test", "path": "/test"}));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("content is required")
        );
    }

    #[test]
    fn test_tool_sandbox_file_read_missing_name() {
        let server = McpServer::new();
        let result = server.tool_sandbox_file_read(&json!({"path": "/test"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("name is required"));
    }

    #[test]
    fn test_tool_sandbox_file_read_missing_path() {
        let server = McpServer::new();
        let result = server.tool_sandbox_file_read(&json!({"name": "test"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("path is required"));
    }

    #[test]
    fn test_tool_sandbox_start_missing_name() {
        let server = McpServer::new();
        let result = server.tool_sandbox_start(&json!({}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("name is required"));
    }

    #[test]
    fn test_tool_sandbox_stop_missing_name() {
        let server = McpServer::new();
        let result = server.tool_sandbox_stop(&json!({}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("name is required"));
    }
}
