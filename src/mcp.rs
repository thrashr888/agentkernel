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

                let mut output = String::from("NAME\tSTATUS\tBACKEND\n");
                for (name, running, backend) in sandboxes {
                    let status = if running { "running" } else { "stopped" };
                    let backend_str = backend
                        .map(|b| format!("{}", b))
                        .unwrap_or_else(|| "unknown".to_string());
                    output.push_str(&format!("{}\t{}\t{}\n", name, status, backend_str));
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
