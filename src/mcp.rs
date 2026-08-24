//! MCP (Model Context Protocol) server implementation.
//!
//! Provides JSON-RPC 2.0 over stdio for integration with Claude Code and other MCP clients.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use tokio::runtime::Handle;

use crate::browser_scripts;
use crate::languages;
use crate::permissions::{CompatibilityMode, SecurityProfile};
use crate::vmm::VmManager;

/// Maximum output size in bytes before truncation.
/// Keeps head + tail and inserts a marker so agents see the start (install progress)
/// and end (actual results) without blowing up context windows.
const MAX_OUTPUT_BYTES: usize = 16_000;

/// Truncate output to keep head and tail, inserting a marker in the middle.
fn truncate_output(output: &str) -> String {
    if output.len() <= MAX_OUTPUT_BYTES {
        return output.to_string();
    }
    let keep = MAX_OUTPUT_BYTES / 2;
    let head = &output[..keep];
    let tail = &output[output.len() - keep..];
    let omitted = output.len() - MAX_OUTPUT_BYTES;
    format!(
        "{}\n\n... [{} bytes truncated] ...\n\n{}",
        head, omitted, tail
    )
}

/// Tool handlers return this to support both text and image responses.
#[derive(Debug)]
enum ToolOutput {
    Text(String),
    /// Base64-encoded image data with MIME type.
    Image {
        data: String,
        mime_type: &'static str,
    },
}

/// MCP server for agentkernel
pub struct McpServer {
    initialized: bool,
    permission_store: crate::interactive_permissions::PermissionStore,
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

fn reject_fresh_manager_firecracker(
    manager: &VmManager,
    sandbox: Option<&str>,
    operation: &str,
) -> Result<()> {
    let backend = sandbox
        .and_then(|name| manager.get_state(name))
        .and_then(|state| state.backend)
        .unwrap_or(manager.backend());
    if backend == crate::backend::BackendType::Firecracker {
        anyhow::bail!(
            "MCP operation '{operation}' is not yet routed through the authoritative Firecracker daemon; use the delegated sandbox lifecycle/exec tools or select another backend"
        );
    }
    Ok(())
}

impl McpServer {
    pub fn new() -> Self {
        Self {
            initialized: false,
            permission_store: crate::interactive_permissions::PermissionStore::new(),
        }
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
                            },
                            "ports": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Port mappings (e.g., [\"8080:80\", \"3000\", \"5353:53/udp\"]). Only when fast=false."
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
                                "description": "Docker image to use (default: alpine:3.24)"
                            },
                            "ports": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Port mappings (e.g., [\"8080:80\", \"3000\", \"5353:53/udp\"])"
                            },
                            "source_url": {
                                "type": "string",
                                "description": "Git repo URL to clone into /workspace"
                            },
                            "source_ref": {
                                "type": "string",
                                "description": "Git ref to checkout after cloning (branch, tag, or commit)"
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
                            },
                            "env": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Environment variables as KEY=VALUE pairs"
                            },
                            "workdir": {
                                "type": "string",
                                "description": "Working directory inside the sandbox"
                            },
                            "sudo": {
                                "type": "boolean",
                                "description": "Run the command as root"
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
                },
                {
                    "name": "sandbox_pause",
                    "description": "Pause a running Firecracker sandbox into a durable full-VM checkpoint that preserves guest memory and processes.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the running Firecracker sandbox to pause"
                            }
                        },
                        "required": ["name"]
                    }
                },
                {
                    "name": "sandbox_resume",
                    "description": "Resume a paused Firecracker sandbox from its full-VM checkpoint.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the paused Firecracker sandbox to resume"
                            }
                        },
                        "required": ["name"]
                    }
                },
                {
                    "name": "sandbox_fork",
                    "description": "Fork a paused Firecracker sandbox into a new running sandbox. The paused source remains reusable. The fork copies guest memory and filesystem state, including any credentials captured in the checkpoint.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the paused source sandbox"
                            },
                            "as_name": {
                                "type": "string",
                                "description": "Name for the new running sandbox"
                            }
                        },
                        "required": ["name", "as_name"]
                    }
                },
                {
                    "name": "sandbox_write_files",
                    "description": "Write multiple files to a running sandbox in one call (writes to sandbox only).",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the sandbox"
                            },
                            "files": {
                                "type": "object",
                                "description": "Map of absolute path to file content (e.g., {\"/app/main.py\": \"print('hello')\"})",
                                "additionalProperties": { "type": "string" }
                            }
                        },
                        "required": ["name", "files"]
                    }
                },
                {
                    "name": "sandbox_exec_detach",
                    "description": "Start a detached (background) command in a sandbox. Returns a command ID for tracking.",
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
                            },
                            "env": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Environment variables as KEY=VALUE pairs"
                            },
                            "workdir": {
                                "type": "string",
                                "description": "Working directory inside the sandbox"
                            },
                            "sudo": {
                                "type": "boolean",
                                "description": "Run the command as root"
                            }
                        },
                        "required": ["name", "command"]
                    }
                },
                {
                    "name": "sandbox_exec_status",
                    "description": "Get the status of a detached command (running, completed, failed).",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "string",
                                "description": "The detached command ID"
                            }
                        },
                        "required": ["id"]
                    }
                },
                {
                    "name": "sandbox_exec_logs",
                    "description": "Get stdout/stderr from a detached command.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "string",
                                "description": "The detached command ID"
                            },
                            "stream": {
                                "type": "string",
                                "description": "Which stream to read: stdout (default) or stderr",
                                "enum": ["stdout", "stderr"]
                            }
                        },
                        "required": ["id"]
                    }
                },
                {
                    "name": "sandbox_exec_kill",
                    "description": "Kill a detached command.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "string",
                                "description": "The detached command ID"
                            }
                        },
                        "required": ["id"]
                    }
                },
                {
                    "name": "sandbox_exec_list",
                    "description": "List all detached commands in a sandbox.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the sandbox (optional, lists all if omitted)"
                            }
                        }
                    }
                },
                {
                    "name": "sandbox_extend_ttl",
                    "description": "Extend a sandbox's time-to-live. Adds additional time to the current expiry.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the sandbox"
                            },
                            "by": {
                                "type": "string",
                                "description": "Additional time (e.g., '1h', '30m', '2d'). Default: 1h"
                            }
                        },
                        "required": ["name"]
                    }
                },
                {
                    "name": "snapshot_list",
                    "description": "List all snapshots.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                },
                {
                    "name": "snapshot_take",
                    "description": "Take a snapshot of a sandbox.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "sandbox": {
                                "type": "string",
                                "description": "Name of the sandbox to snapshot"
                            },
                            "name": {
                                "type": "string",
                                "description": "Name for the snapshot"
                            }
                        },
                        "required": ["sandbox", "name"]
                    }
                },
                {
                    "name": "snapshot_get",
                    "description": "Get information about a snapshot.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the snapshot"
                            }
                        },
                        "required": ["name"]
                    }
                },
                {
                    "name": "snapshot_delete",
                    "description": "Delete a snapshot.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the snapshot to delete"
                            }
                        },
                        "required": ["name"]
                    }
                },
                {
                    "name": "snapshot_restore",
                    "description": "Restore a sandbox from a snapshot.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the snapshot to restore"
                            },
                            "as_name": {
                                "type": "string",
                                "description": "Name for the restored sandbox (defaults to original + '-restored')"
                            }
                        },
                        "required": ["name"]
                    }
                },
                // -- Browser tools --
                {
                    "name": "browser_create",
                    "description": "Create a browser sandbox with Playwright and Chromium pre-installed. One call replaces sandbox_create + installing Playwright. Uses python:3.12-slim with 2048MB RAM by default.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name for the browser sandbox"
                            },
                            "memory_mb": {
                                "type": "integer",
                                "description": "Memory in MB (default: 2048). Chromium needs ~1.5GB minimum.",
                                "default": 2048
                            }
                        },
                        "required": ["name"]
                    }
                },
                {
                    "name": "browser_goto",
                    "description": "Navigate to a URL in a browser sandbox and get page content. Returns JSON with title, final URL, body text (first 8KB), and links (up to 50). The sandbox must have been created with browser_create first.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the browser sandbox"
                            },
                            "url": {
                                "type": "string",
                                "description": "URL to navigate to"
                            }
                        },
                        "required": ["name", "url"]
                    }
                },
                {
                    "name": "browser_screenshot",
                    "description": "Take a screenshot of a web page in a browser sandbox. Returns the image as PNG. The sandbox must have been created with browser_create first.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the browser sandbox"
                            },
                            "url": {
                                "type": "string",
                                "description": "URL to screenshot"
                            }
                        },
                        "required": ["name", "url"]
                    }
                },
                {
                    "name": "browser_evaluate",
                    "description": "Run a JavaScript expression on a web page in a browser sandbox. Returns the result as JSON. The sandbox must have been created with browser_create first.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the browser sandbox"
                            },
                            "url": {
                                "type": "string",
                                "description": "URL to navigate to before evaluating"
                            },
                            "expression": {
                                "type": "string",
                                "description": "JavaScript expression to evaluate (e.g. \"document.title\" or \"document.querySelectorAll('h1').length\")"
                            }
                        },
                        "required": ["name", "url", "expression"]
                    }
                },
                {
                    "name": "browser_remove",
                    "description": "Remove a browser sandbox created with browser_create.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the browser sandbox to remove"
                            }
                        },
                        "required": ["name"]
                    }
                },
                // -- Browser v2 tools (ARIA snapshots, persistent pages, ref-based interaction) --
                {
                    "name": "browser_open",
                    "description": "Navigate to a URL and get an ARIA snapshot of the page. Returns a structured YAML accessibility tree with [ref=eN] identifiers on interactive elements. Use refs with browser_click/browser_fill. The browser server is auto-started on first call. Pages persist across calls — no cold start per action.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the browser sandbox"
                            },
                            "url": {
                                "type": "string",
                                "description": "URL to navigate to"
                            },
                            "page": {
                                "type": "string",
                                "description": "Page name (default: 'default'). Use different names for multiple tabs."
                            }
                        },
                        "required": ["name", "url"]
                    }
                },
                {
                    "name": "browser_snapshot",
                    "description": "Get the current ARIA snapshot of a page without navigating. Returns the accessibility tree as structured YAML with [ref=eN] identifiers. Use this to see the current state after interactions.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the browser sandbox"
                            },
                            "page": {
                                "type": "string",
                                "description": "Page name (default: 'default')"
                            }
                        },
                        "required": ["name"]
                    }
                },
                {
                    "name": "browser_click",
                    "description": "Click an element by ref ID (from ARIA snapshot) or CSS selector. Returns the new ARIA snapshot after the click. Prefer ref= over selector for reliability.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the browser sandbox"
                            },
                            "ref": {
                                "type": "string",
                                "description": "Element ref from ARIA snapshot (e.g. 'e5')"
                            },
                            "selector": {
                                "type": "string",
                                "description": "CSS selector (fallback if ref not available)"
                            },
                            "page": {
                                "type": "string",
                                "description": "Page name (default: 'default')"
                            }
                        },
                        "required": ["name"]
                    }
                },
                {
                    "name": "browser_fill",
                    "description": "Fill an input field by ref ID (from ARIA snapshot) or CSS selector with a value. Returns the new ARIA snapshot after filling.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the browser sandbox"
                            },
                            "ref": {
                                "type": "string",
                                "description": "Element ref from ARIA snapshot (e.g. 'e3')"
                            },
                            "selector": {
                                "type": "string",
                                "description": "CSS selector (fallback if ref not available)"
                            },
                            "value": {
                                "type": "string",
                                "description": "Value to fill into the input"
                            },
                            "page": {
                                "type": "string",
                                "description": "Page name (default: 'default')"
                            }
                        },
                        "required": ["name", "value"]
                    }
                },
                {
                    "name": "browser_close",
                    "description": "Close a named browser page. Use browser_remove to remove the entire sandbox.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the browser sandbox"
                            },
                            "page": {
                                "type": "string",
                                "description": "Page name to close (default: 'default')"
                            }
                        },
                        "required": ["name"]
                    }
                },
                {
                    "name": "browser_events",
                    "description": "Get browser event history for debugging and context recovery. Events include navigation, clicks, fills, and screenshots with sequence numbers.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Name of the browser sandbox"
                            },
                            "offset": {
                                "type": "integer",
                                "description": "Return events after this sequence number (default: 0)"
                            },
                            "limit": {
                                "type": "integer",
                                "description": "Maximum events to return (default: 100)"
                            }
                        },
                        "required": ["name"]
                    }
                },
                // Permission tools
                {
                    "name": "permission_grant",
                    "description": "Grant or deny a permission request. Use after receiving a permission_required error.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "request_id": {
                                "type": "string",
                                "description": "The permission request ID from the error response"
                            },
                            "kind": {
                                "type": "string",
                                "enum": ["sandbox_remove", "sandbox_create", "network_access", "mount_directory", "sudo_exec", "file_delete"],
                                "description": "The permission kind to grant"
                            },
                            "granted": {
                                "type": "boolean",
                                "description": "Whether to grant (true) or deny (false) the permission"
                            },
                            "scope": {
                                "type": "string",
                                "enum": ["once", "session", "always"],
                                "description": "Scope of the grant (default: once)"
                            },
                            "sandbox": {
                                "type": "string",
                                "description": "Sandbox name to scope the grant to (omit for all sandboxes)"
                            }
                        },
                        "required": ["kind", "granted"]
                    }
                },
                {
                    "name": "permission_list",
                    "description": "List all active permission grants.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                },
                {
                    "name": "permission_revoke",
                    "description": "Revoke a specific permission grant by ID.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "string",
                                "description": "The grant ID to revoke"
                            }
                        },
                        "required": ["id"]
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

        let result: Result<ToolOutput> = match tool_name {
            "sandbox_run" => self.tool_sandbox_run(&arguments).map(ToolOutput::Text),
            "sandbox_create" => self.tool_sandbox_create(&arguments).map(ToolOutput::Text),
            "sandbox_exec" => self.tool_sandbox_exec(&arguments).map(ToolOutput::Text),
            "sandbox_list" => self.tool_sandbox_list().map(ToolOutput::Text),
            "sandbox_remove" => self.tool_sandbox_remove(&arguments).map(ToolOutput::Text),
            "sandbox_file_write" => self
                .tool_sandbox_file_write(&arguments)
                .map(ToolOutput::Text),
            "sandbox_file_read" => self
                .tool_sandbox_file_read(&arguments)
                .map(ToolOutput::Text),
            "sandbox_write_files" => self
                .tool_sandbox_write_files(&arguments)
                .map(ToolOutput::Text),
            "sandbox_start" => self.tool_sandbox_start(&arguments).map(ToolOutput::Text),
            "sandbox_stop" => self.tool_sandbox_stop(&arguments).map(ToolOutput::Text),
            "sandbox_pause" => self.tool_sandbox_pause(&arguments).map(ToolOutput::Text),
            "sandbox_resume" => self.tool_sandbox_resume(&arguments).map(ToolOutput::Text),
            "sandbox_fork" => self.tool_sandbox_fork(&arguments).map(ToolOutput::Text),
            "sandbox_exec_detach" => self
                .tool_sandbox_exec_detach(&arguments)
                .map(ToolOutput::Text),
            "sandbox_exec_status" => self
                .tool_sandbox_exec_status(&arguments)
                .map(ToolOutput::Text),
            "sandbox_exec_logs" => self
                .tool_sandbox_exec_logs(&arguments)
                .map(ToolOutput::Text),
            "sandbox_exec_kill" => self
                .tool_sandbox_exec_kill(&arguments)
                .map(ToolOutput::Text),
            "sandbox_exec_list" => self
                .tool_sandbox_exec_list(&arguments)
                .map(ToolOutput::Text),
            "sandbox_extend_ttl" => self
                .tool_sandbox_extend_ttl(&arguments)
                .map(ToolOutput::Text),
            "snapshot_list" => self.tool_snapshot_list().map(ToolOutput::Text),
            "snapshot_take" => self.tool_snapshot_take(&arguments).map(ToolOutput::Text),
            "snapshot_get" => self.tool_snapshot_get(&arguments).map(ToolOutput::Text),
            "snapshot_delete" => self.tool_snapshot_delete(&arguments).map(ToolOutput::Text),
            "snapshot_restore" => self.tool_snapshot_restore(&arguments).map(ToolOutput::Text),
            // Browser tools (v1)
            "browser_create" => self.tool_browser_create(&arguments).map(ToolOutput::Text),
            "browser_goto" => self.tool_browser_goto(&arguments).map(ToolOutput::Text),
            "browser_screenshot" => self.tool_browser_screenshot(&arguments),
            "browser_evaluate" => self.tool_browser_evaluate(&arguments).map(ToolOutput::Text),
            "browser_remove" => self.tool_browser_remove(&arguments).map(ToolOutput::Text),
            // Browser tools (v2 — ARIA snapshots, persistent pages, ref-based)
            "browser_open" => self.tool_browser_open(&arguments).map(ToolOutput::Text),
            "browser_snapshot" => self
                .tool_browser_snapshot_v2(&arguments)
                .map(ToolOutput::Text),
            "browser_click" => self.tool_browser_click(&arguments).map(ToolOutput::Text),
            "browser_fill" => self.tool_browser_fill(&arguments).map(ToolOutput::Text),
            "browser_close" => self.tool_browser_close(&arguments).map(ToolOutput::Text),
            "browser_events" => self.tool_browser_events(&arguments).map(ToolOutput::Text),
            // Permission tools
            "permission_grant" => self.tool_permission_grant(&arguments).map(ToolOutput::Text),
            "permission_list" => self.tool_permission_list().map(ToolOutput::Text),
            "permission_revoke" => self
                .tool_permission_revoke(&arguments)
                .map(ToolOutput::Text),
            _ => Err(anyhow::anyhow!("Unknown tool: {}", tool_name)),
        };

        match result {
            Ok(ToolOutput::Text(content)) => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(json!({
                    "content": [{
                        "type": "text",
                        "text": truncate_output(&content)
                    }]
                })),
                error: None,
            },
            Ok(ToolOutput::Image { data, mime_type }) => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(json!({
                    "content": [{
                        "type": "image",
                        "data": data,
                        "mimeType": mime_type
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

        // Parse port mappings
        let ports: Vec<crate::backend::PortMapping> = args
            .get("ports")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(crate::backend::PortMapping::parse)
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default();

        // Default to fast mode (use container pool)
        let fast = args.get("fast").and_then(|v| v.as_bool()).unwrap_or(true);

        // Fast path: use container pool (default)
        if fast {
            if !ports.is_empty() {
                anyhow::bail!(
                    "Cannot use fast mode with ports (pooled containers don't support port mapping). Set fast=false."
                );
            }
            if args.get("image").is_some() {
                eprintln!("Warning: custom image ignored in fast mode (pool uses alpine:3.24)");
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
                reject_fresh_manager_firecracker(&manager, None, "sandbox_run fast=false")?;

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

        // Permission check
        self.require_permission(
            crate::interactive_permissions::PermissionKind::SandboxCreate,
            Some(name),
        )?;

        let image = args
            .get("image")
            .and_then(|v| v.as_str())
            .unwrap_or("alpine:3.24");

        let ports: Vec<crate::backend::PortMapping> = args
            .get("ports")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(crate::backend::PortMapping::parse)
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default();

        let source_url = args
            .get("source_url")
            .and_then(|v| v.as_str())
            .map(String::from);

        let source_ref = args
            .get("source_ref")
            .and_then(|v| v.as_str())
            .map(String::from);

        let port_desc = if ports.is_empty() {
            String::new()
        } else {
            format!(
                " with ports {}",
                ports
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;
                if manager.backend() == crate::backend::BackendType::Firecracker {
                    let port = crate::local_api_port();
                    if !crate::try_server_health("127.0.0.1", port).await {
                        return Err(crate::firecracker_server_required("create", port));
                    }
                }
                manager
                    .create_with_options(name, image, 1, 512, None, ports)
                    .await?;
                let backend = manager
                    .get_state(name)
                    .and_then(|state| state.backend)
                    .unwrap_or(manager.backend());
                let firecracker = backend == crate::backend::BackendType::Firecracker;
                let port = crate::local_api_port();
                if firecracker {
                    if !crate::try_server_health("127.0.0.1", port).await {
                        return Err(crate::firecracker_server_required("start", port));
                    }
                    let permissions = crate::permissions::Permissions::default();
                    crate::delegate_configured_start_to_server(
                        "127.0.0.1",
                        port,
                        name,
                        &manager,
                        &permissions,
                        &[],
                    )
                    .await?;
                } else {
                    manager.start(name).await?;
                }

                let mut result = format!(
                    "Sandbox '{}' created and started with image '{}'{}",
                    name, image, port_desc
                );

                // Clone git repo if source_url provided
                if let Some(ref url) = source_url {
                    let install = vec![
                        "sh".to_string(),
                        "-c".to_string(),
                        "which git >/dev/null 2>&1 || apk add --no-cache git >/dev/null 2>&1 || apt-get update -qq && apt-get install -y -qq git >/dev/null 2>&1 || true".to_string(),
                    ];
                    if firecracker {
                        let _ = crate::delegate_sandbox_action_to_server(
                            "127.0.0.1",
                            port,
                            name,
                            "exec",
                            Some(json!({"command": install})),
                        )
                        .await;
                    } else {
                        let _ = manager.exec_cmd(name, &install).await;
                    }

                    let clone = vec![
                        "git".to_string(),
                        "clone".to_string(),
                        url.clone(),
                        "/workspace".to_string(),
                    ];
                    if firecracker {
                        crate::delegate_sandbox_action_to_server(
                            "127.0.0.1",
                            port,
                            name,
                            "exec",
                            Some(json!({"command": clone})),
                        )
                        .await?;
                    } else {
                        manager.exec_cmd(name, &clone).await?;
                    }

                    if let Some(ref git_ref) = source_ref {
                        let checkout = vec![
                            "git".to_string(),
                            "-C".to_string(),
                            "/workspace".to_string(),
                            "checkout".to_string(),
                            git_ref.clone(),
                        ];
                        if firecracker {
                            crate::delegate_sandbox_action_to_server(
                                "127.0.0.1",
                                port,
                                name,
                                "exec",
                                Some(json!({"command": checkout})),
                            )
                            .await?;
                        } else {
                            manager.exec_cmd(name, &checkout).await?;
                        }
                        result.push_str(&format!(". Cloned {} (ref: {}) into /workspace", url, git_ref));
                    } else {
                        result.push_str(&format!(". Cloned {} into /workspace", url));
                    }
                }

                Ok(result)
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

        let env: Vec<String> = args
            .get("env")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let workdir = args
            .get("workdir")
            .and_then(|v| v.as_str())
            .map(String::from);

        let sudo = args.get("sudo").and_then(|v| v.as_bool()).unwrap_or(false);

        let opts = crate::backend::ExecOptions {
            env,
            workdir,
            user: if sudo { Some("root".to_string()) } else { None },
        };

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;
                if !manager.exists(name) {
                    anyhow::bail!("Sandbox '{}' not found.", name);
                }
                let backend = manager
                    .get_state(name)
                    .and_then(|state| state.backend)
                    .unwrap_or(manager.backend());
                if backend == crate::backend::BackendType::Firecracker {
                    let port = crate::local_api_port();
                    if !crate::try_server_health("127.0.0.1", port).await {
                        return Err(crate::firecracker_server_required("exec", port));
                    }
                    crate::delegate_exec_to_server(
                        "127.0.0.1",
                        port,
                        name,
                        &command,
                        &opts.env,
                        opts.workdir.as_deref(),
                        sudo,
                    )
                    .await
                } else {
                    manager.exec_cmd_full(name, &command, &opts).await
                }
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

                let mut output = String::from("NAME\tSTATUS\tBACKEND\tIP\tPORTS\n");
                for (name, running, backend) in &sandboxes {
                    let local_status = manager
                        .get_state(name)
                        .map(|state| state.status(*running))
                        .unwrap_or(if *running { "running" } else { "stopped" })
                        .to_string();
                    let authoritative =
                        if *backend == Some(crate::backend::BackendType::Firecracker) {
                            crate::delegated_firecracker_status(name).await?
                        } else {
                            None
                        };
                    let status = authoritative
                        .as_ref()
                        .map(|status| status.status.as_str())
                        .unwrap_or(&local_status);
                    let backend_str = backend
                        .map(|b| format!("{}", b))
                        .unwrap_or_else(|| "unknown".to_string());
                    let ip_str = if let Some(authoritative) = authoritative.as_ref() {
                        authoritative.ip.clone().unwrap_or_else(|| "-".to_string())
                    } else if *running {
                        manager
                            .get_container_ip(name)
                            .unwrap_or_else(|| "-".to_string())
                    } else {
                        "-".to_string()
                    };
                    let ports_str = manager
                        .get_state(name)
                        .map(|s| {
                            s.ports
                                .iter()
                                .map(|p| p.to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    output.push_str(&format!(
                        "{}\t{}\t{}\t{}\t{}\n",
                        name, status, backend_str, ip_str, ports_str
                    ));
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

        // Permission check
        self.require_permission(
            crate::interactive_permissions::PermissionKind::SandboxRemove,
            Some(name),
        )?;

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;
                crate::remove_sandbox_via_authoritative_manager(&mut manager, name).await?;
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
                reject_fresh_manager_firecracker(&manager, Some(name), "sandbox_file_write")?;

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
                reject_fresh_manager_firecracker(&manager, Some(name), "sandbox_file_read")?;

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

    fn tool_sandbox_write_files(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;

        let files = args
            .get("files")
            .and_then(|v| v.as_object())
            .ok_or_else(|| anyhow::anyhow!("files is required (object mapping path to content)"))?;

        if files.is_empty() {
            anyhow::bail!("files map is empty");
        }

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;
                reject_fresh_manager_firecracker(&manager, Some(name), "sandbox_write_files")?;

                if !manager.is_running(name) {
                    anyhow::bail!(
                        "Sandbox '{}' is not running. Start it first with sandbox_start.",
                        name
                    );
                }

                let mut count = 0;
                for (path, content) in files {
                    let text = content.as_str().unwrap_or("");
                    manager.write_file(name, path, text.as_bytes()).await?;
                    count += 1;
                }

                Ok(format!("Wrote {} file(s) to sandbox '{}'", count, name))
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

                let backend = manager
                    .get_state(name)
                    .and_then(|state| state.backend)
                    .unwrap_or(manager.backend());
                if backend == crate::backend::BackendType::Firecracker {
                    let port = crate::local_api_port();
                    if !crate::try_server_health("127.0.0.1", port).await {
                        return Err(crate::firecracker_server_required("start", port));
                    }
                    let permissions = crate::permissions::Permissions::default();
                    crate::delegate_configured_start_to_server(
                        "127.0.0.1",
                        port,
                        name,
                        &manager,
                        &permissions,
                        &[],
                    )
                    .await?;
                } else {
                    manager.start(name).await?;
                }
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

                let backend = manager
                    .get_state(name)
                    .and_then(|state| state.backend)
                    .unwrap_or(manager.backend());
                if backend == crate::backend::BackendType::Firecracker {
                    let port = crate::local_api_port();
                    if !crate::try_server_health("127.0.0.1", port).await {
                        return Err(crate::firecracker_server_required("stop", port));
                    }
                    crate::delegate_sandbox_action_to_server("127.0.0.1", port, name, "stop", None)
                        .await?;
                } else if !manager.is_running(name) {
                    return Ok(format!("Sandbox '{}' is already stopped.", name));
                } else {
                    manager.stop(name).await?;
                }
                Ok(format!("Sandbox '{}' stopped.", name))
            })
        })
    }

    fn tool_sandbox_pause(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;
        crate::validation::validate_sandbox_name(name)?;

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;
                if !manager.exists(name) {
                    anyhow::bail!("Sandbox '{}' not found.", name);
                }

                let backend = manager
                    .get_state(name)
                    .and_then(|state| state.backend)
                    .unwrap_or(manager.backend());
                if backend == crate::backend::BackendType::Firecracker {
                    let port = crate::local_api_port();
                    if !crate::try_server_health("127.0.0.1", port).await {
                        return Err(crate::firecracker_server_required("pause", port));
                    }
                    crate::delegate_sandbox_action_to_server(
                        "127.0.0.1",
                        port,
                        name,
                        "pause",
                        None,
                    )
                    .await?;
                } else {
                    manager.pause(name).await?;
                }
                Ok(format!(
                    "Sandbox '{}' paused. Resume it with sandbox_resume.",
                    name
                ))
            })
        })
    }

    fn tool_sandbox_resume(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;
        crate::validation::validate_sandbox_name(name)?;

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;
                if !manager.exists(name) {
                    anyhow::bail!("Sandbox '{}' not found.", name);
                }

                let backend = manager
                    .get_state(name)
                    .and_then(|state| state.backend)
                    .unwrap_or(manager.backend());
                if backend == crate::backend::BackendType::Firecracker {
                    let port = crate::local_api_port();
                    if !crate::try_server_health("127.0.0.1", port).await {
                        return Err(crate::firecracker_server_required("resume", port));
                    }
                    crate::delegate_sandbox_action_to_server(
                        "127.0.0.1",
                        port,
                        name,
                        "resume",
                        None,
                    )
                    .await?;
                } else {
                    manager.resume(name).await?;
                }
                Ok(format!("Sandbox '{}' resumed.", name))
            })
        })
    }

    fn tool_sandbox_fork(&self, args: &Value) -> Result<String> {
        let source = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;
        let child = args
            .get("as_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("as_name is required"))?;
        crate::validation::validate_sandbox_name(source)?;
        crate::validation::validate_sandbox_name(child)?;

        self.require_permission(
            crate::interactive_permissions::PermissionKind::SandboxCreate,
            Some(child),
        )?;

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;
                if !manager.exists(source) {
                    anyhow::bail!("Sandbox '{}' not found.", source);
                }
                if manager.exists(child) {
                    anyhow::bail!("Sandbox '{}' already exists.", child);
                }

                let backend = manager
                    .get_state(source)
                    .and_then(|state| state.backend)
                    .unwrap_or(manager.backend());
                if backend == crate::backend::BackendType::Firecracker {
                    let port = crate::local_api_port();
                    if !crate::try_server_health("127.0.0.1", port).await {
                        return Err(crate::firecracker_server_required("fork", port));
                    }
                    crate::delegate_sandbox_action_to_server(
                        "127.0.0.1",
                        port,
                        source,
                        "fork",
                        Some(json!({"as_name": child})),
                    )
                    .await?;
                } else {
                    manager.fork_sandbox(source, child).await?;
                }
                Ok(format!(
                    "Sandbox '{}' forked from '{}' and is running. The source remains paused and reusable.\n\nSecurity warning: {}",
                    child, source, crate::full_state::FORK_SECURITY_WARNING
                ))
            })
        })
    }

    fn tool_sandbox_exec_detach(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;
        let command: Vec<String> = args
            .get("command")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("command is required"))?
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        let env: Vec<String> = args
            .get("env")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let workdir = args
            .get("workdir")
            .and_then(|v| v.as_str())
            .map(String::from);
        let sudo = args.get("sudo").and_then(|v| v.as_bool()).unwrap_or(false);

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;
                reject_fresh_manager_firecracker(&manager, Some(name), "sandbox_exec_detach")?;
                let opts = crate::backend::ExecOptions {
                    env,
                    workdir,
                    user: if sudo { Some("root".to_string()) } else { None },
                };
                let cmd = manager.exec_detached(name, &command, &opts).await?;
                Ok(serde_json::to_string_pretty(&cmd)?)
            })
        })
    }

    fn tool_sandbox_exec_status(&self, args: &Value) -> Result<String> {
        let id = args
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("id is required"))?;

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;
                let cmd = manager.detached_status(id).await?;
                Ok(serde_json::to_string_pretty(&cmd)?)
            })
        })
    }

    fn tool_sandbox_exec_logs(&self, args: &Value) -> Result<String> {
        let id = args
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("id is required"))?;
        let stream = args.get("stream").and_then(|v| v.as_str());

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;
                manager.detached_logs(id, stream).await
            })
        })
    }

    fn tool_sandbox_exec_kill(&self, args: &Value) -> Result<String> {
        let id = args
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("id is required"))?;

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;
                manager.detached_kill(id).await?;
                Ok(format!("Command {} killed.", id))
            })
        })
    }

    fn tool_sandbox_exec_list(&self, args: &Value) -> Result<String> {
        let name = args.get("name").and_then(|v| v.as_str());

        let manager = VmManager::new()?;
        let commands = manager.detached_list(name);
        Ok(serde_json::to_string_pretty(&commands)?)
    }

    fn tool_sandbox_extend_ttl(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;

        let by = args.get("by").and_then(|v| v.as_str()).unwrap_or("1h");

        let mut manager = VmManager::new()?;
        if !manager.exists(name) {
            anyhow::bail!("Sandbox '{}' not found", name);
        }

        let new_expiry = tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                crate::extend_ttl_via_authoritative_manager(&mut manager, name, by).await
            })
        })?;

        match new_expiry {
            Some(exp) => Ok(format!(
                "Extended TTL for sandbox '{}'. New expiry: {}",
                name, exp
            )),
            None => Ok(format!(
                "Sandbox '{}' now has no expiry (TTL disabled).",
                name
            )),
        }
    }

    fn tool_snapshot_list(&self) -> Result<String> {
        let snapshots = crate::snapshot::list()?;
        Ok(serde_json::to_string_pretty(&snapshots)?)
    }

    fn tool_snapshot_take(&self, args: &Value) -> Result<String> {
        let sandbox = args
            .get("sandbox")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("sandbox is required"))?;

        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;

        // Get sandbox state
        let manager = VmManager::new()?;
        let sandbox_state = manager
            .get_state(sandbox)
            .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", sandbox))?;

        let input = crate::snapshot::SnapshotInput {
            image: sandbox_state.image.clone(),
            backend: sandbox_state
                .backend
                .map(|b| format!("{:?}", b).to_lowercase())
                .unwrap_or_else(|| "docker".to_string()),
            vcpus: sandbox_state.vcpus,
            memory_mb: sandbox_state.memory_mb,
            remote_id: sandbox_state.remote_id.clone(),
            remote_namespace: sandbox_state.remote_namespace.clone(),
            remote_metadata: sandbox_state.remote_metadata.clone(),
            workspace_revision: sandbox_state.workspace_revision.clone(),
            work_dir: sandbox_state.work_dir.clone(),
            config_path: sandbox_state.config_path.clone(),
        };

        let meta = crate::snapshot::take(sandbox, name, &input)?;
        Ok(serde_json::to_string_pretty(&meta)?)
    }

    fn tool_snapshot_get(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;

        match crate::snapshot::get(name)? {
            Some(meta) => Ok(serde_json::to_string_pretty(&meta)?),
            None => anyhow::bail!("Snapshot '{}' not found", name),
        }
    }

    fn tool_snapshot_delete(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;

        crate::snapshot::delete(name)?;
        Ok(format!("Snapshot '{}' deleted.", name))
    }

    fn tool_snapshot_restore(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;

        let as_name = args.get("as_name").and_then(|v| v.as_str());

        // Get snapshot metadata
        let meta = crate::snapshot::get(name)?
            .ok_or_else(|| anyhow::anyhow!("Snapshot '{}' not found", name))?;

        let restore_name = as_name
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{}-restored", meta.sandbox));

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;
                if let Ok(snapshot_backend) = meta.backend.parse::<crate::backend::BackendType>() {
                    manager
                        .create_with_backend(
                            snapshot_backend,
                            &restore_name,
                            meta.restore_image(),
                            meta.vcpus,
                            meta.memory_mb,
                        )
                        .await?;
                } else {
                    manager
                        .create(
                            &restore_name,
                            meta.restore_image(),
                            meta.vcpus,
                            meta.memory_mb,
                        )
                        .await?;
                }
                manager.set_work_dir(&restore_name, meta.work_dir.clone())?;
                manager.set_config_path(&restore_name, meta.config_path.clone())?;
                if let Some(snapshot_handle) = meta.remote_snapshot.as_deref() {
                    manager.set_remote_restore_snapshot(&restore_name, snapshot_handle)?;
                }
                Ok(format!(
                    "Restored snapshot '{}' as sandbox '{}'.",
                    name, restore_name
                ))
            })
        })
    }

    // ---------- Browser tools ----------

    fn tool_browser_create(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;

        let memory_mb = args
            .get("memory_mb")
            .and_then(|v| v.as_u64())
            .unwrap_or(browser_scripts::BROWSER_MEMORY_MB);

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;
                reject_fresh_manager_firecracker(&manager, None, "browser_create")?;
                manager
                    .create_with_options(
                        name,
                        browser_scripts::BROWSER_IMAGE,
                        1,
                        memory_mb,
                        None,
                        Vec::new(),
                    )
                    .await?;
                manager.start(name).await?;

                // Install Playwright + Chromium
                let setup_cmd: Vec<String> = browser_scripts::BROWSER_SETUP_CMD
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                manager.exec_cmd(name, &setup_cmd).await?;

                Ok(format!(
                    "Browser sandbox '{}' ready (Playwright + Chromium installed, {}MB RAM).",
                    name, memory_mb
                ))
            })
        })
    }

    fn tool_browser_goto(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;

        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("url is required"))?;

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;
                reject_fresh_manager_firecracker(&manager, Some(name), "browser_goto")?;
                let cmd = vec![
                    "python3".to_string(),
                    "-c".to_string(),
                    browser_scripts::GOTO_SCRIPT.to_string(),
                    url.to_string(),
                ];
                manager.exec_cmd(name, &cmd).await
            })
        })
    }

    fn tool_browser_screenshot(&self, args: &Value) -> Result<ToolOutput> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;

        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("url is required"))?;

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;
                reject_fresh_manager_firecracker(&manager, Some(name), "browser_screenshot")?;
                let cmd = vec![
                    "python3".to_string(),
                    "-c".to_string(),
                    browser_scripts::SCREENSHOT_SCRIPT.to_string(),
                    url.to_string(),
                ];
                let output = manager.exec_cmd(name, &cmd).await?;
                Ok(ToolOutput::Image {
                    data: output.trim().to_string(),
                    mime_type: "image/png",
                })
            })
        })
    }

    fn tool_browser_evaluate(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;

        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("url is required"))?;

        let expression = args
            .get("expression")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("expression is required"))?;

        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;
                reject_fresh_manager_firecracker(&manager, Some(name), "browser_evaluate")?;
                let cmd = vec![
                    "python3".to_string(),
                    "-c".to_string(),
                    browser_scripts::EVALUATE_SCRIPT.to_string(),
                    url.to_string(),
                    expression.to_string(),
                ];
                manager.exec_cmd(name, &cmd).await
            })
        })
    }

    fn tool_browser_remove(&self, args: &Value) -> Result<String> {
        self.tool_sandbox_remove(args)
    }

    // ---------- Browser v2 tools (ARIA snapshots, persistent pages) ----------

    /// Ensure browser server is running in the sandbox.
    /// Returns Ok(()) if server is healthy, starts it if needed.
    fn ensure_browser_server(&self, name: &str) -> Result<()> {
        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;
                reject_fresh_manager_firecracker(&manager, Some(name), "browser server")?;

                // Check if server is already running
                let health_cmd = vec![
                    "python3".to_string(),
                    "-c".to_string(),
                    browser_scripts::BROWSER_SERVER_HEALTH_CMD.to_string(),
                    browser_scripts::BROWSER_SERVER_PORT.to_string(),
                ];
                if let Ok(output) = manager.exec_cmd(name, &health_cmd).await
                    && (output.contains("\"status\": \"ok\"")
                        || output.contains("\"status\":\"ok\""))
                {
                    return Ok(());
                }

                // Start the server
                let start_cmd = vec![
                    "python3".to_string(),
                    "-c".to_string(),
                    browser_scripts::BROWSER_SERVER_START_CMD.to_string(),
                    browser_scripts::ARIA_SNAPSHOT_JS.to_string(),
                    browser_scripts::BROWSER_SERVER_PORT.to_string(),
                    browser_scripts::BROWSER_SERVER_SCRIPT.to_string(),
                ];
                let output = manager.exec_cmd(name, &start_cmd).await?;

                // Parse output for errors
                if let Ok(data) = serde_json::from_str::<Value>(&output)
                    && let Some(err) = data.get("error").and_then(|v| v.as_str())
                {
                    anyhow::bail!("Browser server failed to start: {}", err);
                }

                Ok(())
            })
        })
    }

    /// Send a request to the in-sandbox browser server.
    fn browser_request(
        &self,
        name: &str,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<String> {
        tokio::task::block_in_place(|| {
            Handle::current().block_on(async {
                let mut manager = VmManager::new()?;
                reject_fresh_manager_firecracker(&manager, Some(name), "browser request")?;
                let mut cmd = vec![
                    "python3".to_string(),
                    "-c".to_string(),
                    browser_scripts::BROWSER_SERVER_REQUEST_CMD.to_string(),
                    browser_scripts::BROWSER_SERVER_PORT.to_string(),
                    method.to_string(),
                    path.to_string(),
                ];
                if let Some(b) = body {
                    cmd.push(serde_json::to_string(b)?);
                }
                manager.exec_cmd(name, &cmd).await
            })
        })
    }

    fn tool_browser_open(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("url is required"))?;
        let page = args
            .get("page")
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        self.ensure_browser_server(name)?;
        let body = json!({"url": url});
        self.browser_request(name, "POST", &format!("/pages/{}/goto", page), Some(&body))
    }

    fn tool_browser_snapshot_v2(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;
        let page = args
            .get("page")
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        self.ensure_browser_server(name)?;
        self.browser_request(name, "GET", &format!("/pages/{}/snapshot", page), None)
    }

    fn tool_browser_click(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;
        let page = args
            .get("page")
            .and_then(|v| v.as_str())
            .unwrap_or("default");
        let ref_id = args.get("ref").and_then(|v| v.as_str());
        let selector = args.get("selector").and_then(|v| v.as_str());

        if ref_id.is_none() && selector.is_none() {
            anyhow::bail!("ref or selector is required");
        }

        self.ensure_browser_server(name)?;
        let mut body = json!({});
        if let Some(r) = ref_id {
            body["ref"] = json!(r);
        }
        if let Some(s) = selector {
            body["selector"] = json!(s);
        }
        self.browser_request(name, "POST", &format!("/pages/{}/click", page), Some(&body))
    }

    fn tool_browser_fill(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;
        let value = args
            .get("value")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("value is required"))?;
        let page = args
            .get("page")
            .and_then(|v| v.as_str())
            .unwrap_or("default");
        let ref_id = args.get("ref").and_then(|v| v.as_str());
        let selector = args.get("selector").and_then(|v| v.as_str());

        if ref_id.is_none() && selector.is_none() {
            anyhow::bail!("ref or selector is required");
        }

        self.ensure_browser_server(name)?;
        let mut body = json!({"value": value});
        if let Some(r) = ref_id {
            body["ref"] = json!(r);
        }
        if let Some(s) = selector {
            body["selector"] = json!(s);
        }
        self.browser_request(name, "POST", &format!("/pages/{}/fill", page), Some(&body))
    }

    fn tool_browser_close(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;
        let page = args
            .get("page")
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        self.ensure_browser_server(name)?;
        self.browser_request(name, "DELETE", &format!("/pages/{}", page), None)
    }

    fn tool_browser_events(&self, args: &Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;
        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0);
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(100);

        self.ensure_browser_server(name)?;
        self.browser_request(
            name,
            "GET",
            &format!("/events?offset={}&limit={}", offset, limit),
            None,
        )
    }

    // -----------------------------------------------------------------
    // Permission tools
    // -----------------------------------------------------------------

    fn tool_permission_grant(&self, args: &Value) -> Result<String> {
        use crate::interactive_permissions::{GrantScope, PermissionKind};

        let kind_str = args
            .get("kind")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("kind is required"))?;

        let kind = PermissionKind::from_str(kind_str)
            .ok_or_else(|| anyhow::anyhow!("Unknown permission kind: {kind_str}"))?;

        let granted = args
            .get("granted")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| anyhow::anyhow!("granted is required"))?;

        if !granted {
            return Ok("Permission denied by user.".to_string());
        }

        let scope = match args.get("scope").and_then(|v| v.as_str()) {
            Some("session") => GrantScope::Session,
            Some("always") => GrantScope::Always,
            _ => GrantScope::Once,
        };

        let sandbox = args
            .get("sandbox")
            .and_then(|v| v.as_str())
            .map(String::from);

        let grant_id = self
            .permission_store
            .grant(kind, scope, sandbox, "mcp_user");

        Ok(serde_json::to_string_pretty(&json!({
            "granted": true,
            "grant_id": grant_id,
            "kind": kind_str,
            "scope": format!("{:?}", scope).to_lowercase(),
        }))?)
    }

    fn tool_permission_list(&self) -> Result<String> {
        let grants = self.permission_store.list();
        Ok(serde_json::to_string_pretty(&grants)?)
    }

    fn tool_permission_revoke(&self, args: &Value) -> Result<String> {
        let id = args
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("id is required"))?;

        if self.permission_store.revoke(id) {
            Ok(format!("Permission grant '{id}' revoked."))
        } else {
            Err(anyhow::anyhow!("Grant '{id}' not found."))
        }
    }

    /// Check permission and return a structured error if not granted.
    fn require_permission(
        &self,
        kind: crate::interactive_permissions::PermissionKind,
        sandbox: Option<&str>,
    ) -> Result<()> {
        if self.permission_store.check(kind, sandbox) {
            // Consume once-grants on use
            self.permission_store.consume_once(kind, sandbox);
            Ok(())
        } else {
            let req =
                crate::interactive_permissions::PermissionStore::create_request(kind, sandbox);
            Err(anyhow::anyhow!(
                "Permission required: {} for {}. Grant with permission_grant tool. Request ID: {}",
                kind,
                sandbox.unwrap_or("all sandboxes"),
                req.id
            ))
        }
    }
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new()
    }
}

/// Access the permission store from outside the MCP server (e.g., HTTP API).
pub fn default_permission_store() -> &'static crate::interactive_permissions::PermissionStore {
    use std::sync::OnceLock;
    static STORE: OnceLock<crate::interactive_permissions::PermissionStore> = OnceLock::new();
    STORE.get_or_init(crate::interactive_permissions::PermissionStore::new)
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
        assert!(tool_names.contains(&"sandbox_pause"));
        assert!(tool_names.contains(&"sandbox_resume"));
        assert!(tool_names.contains(&"sandbox_fork"));
    }

    #[test]
    fn test_full_state_tool_schemas() {
        let server = McpServer::new();
        let response = server.handle_tools_list(Value::Number(1.into()));
        let result = response.result.unwrap();
        let tools = result.get("tools").and_then(Value::as_array).unwrap();

        let fork = tools
            .iter()
            .find(|tool| tool["name"] == "sandbox_fork")
            .unwrap();
        assert_eq!(fork["inputSchema"]["required"], json!(["name", "as_name"]));
        assert!(
            fork["description"]
                .as_str()
                .unwrap()
                .contains("credentials")
        );
        assert!(crate::full_state::FORK_SECURITY_WARNING.contains("cryptographic tokens"));
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

    #[test]
    fn test_tool_sandbox_pause_missing_name() {
        let server = McpServer::new();
        let result = server.tool_sandbox_pause(&json!({}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("name is required"));
    }

    #[test]
    fn test_tool_sandbox_resume_missing_name() {
        let server = McpServer::new();
        let result = server.tool_sandbox_resume(&json!({}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("name is required"));
    }

    #[test]
    fn test_tool_sandbox_fork_missing_parameters() {
        let server = McpServer::new();
        let missing_source = server.tool_sandbox_fork(&json!({"as_name": "child"}));
        assert!(missing_source.is_err());
        assert!(
            missing_source
                .unwrap_err()
                .to_string()
                .contains("name is required")
        );

        let missing_child = server.tool_sandbox_fork(&json!({"name": "source"}));
        assert!(missing_child.is_err());
        assert!(
            missing_child
                .unwrap_err()
                .to_string()
                .contains("as_name is required")
        );
    }

    // === Browser tool tests ===

    #[test]
    fn test_handle_tools_list_includes_browser_tools() {
        let server = McpServer::new();
        let response = server.handle_tools_list(Value::Number(1.into()));
        let result = response.result.unwrap();
        let tools = result.get("tools").and_then(|t| t.as_array()).unwrap();
        let tool_names: Vec<&str> = tools
            .iter()
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
            .collect();

        assert!(tool_names.contains(&"browser_create"));
        assert!(tool_names.contains(&"browser_goto"));
        assert!(tool_names.contains(&"browser_screenshot"));
        assert!(tool_names.contains(&"browser_evaluate"));
        assert!(tool_names.contains(&"browser_remove"));
    }

    #[test]
    fn test_tool_browser_create_missing_name() {
        let server = McpServer::new();
        let result = server.tool_browser_create(&json!({}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("name is required"));
    }

    #[test]
    fn test_tool_browser_goto_missing_name() {
        let server = McpServer::new();
        let result = server.tool_browser_goto(&json!({"url": "https://example.com"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("name is required"));
    }

    #[test]
    fn test_tool_browser_goto_missing_url() {
        let server = McpServer::new();
        let result = server.tool_browser_goto(&json!({"name": "test"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("url is required"));
    }

    #[test]
    fn test_tool_browser_screenshot_missing_name() {
        let server = McpServer::new();
        let result = server.tool_browser_screenshot(&json!({"url": "https://example.com"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("name is required"));
    }

    #[test]
    fn test_tool_browser_screenshot_missing_url() {
        let server = McpServer::new();
        let result = server.tool_browser_screenshot(&json!({"name": "test"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("url is required"));
    }

    #[test]
    fn test_tool_browser_evaluate_missing_name() {
        let server = McpServer::new();
        let result = server
            .tool_browser_evaluate(&json!({"url": "https://example.com", "expression": "1+1"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("name is required"));
    }

    #[test]
    fn test_tool_browser_evaluate_missing_url() {
        let server = McpServer::new();
        let result = server.tool_browser_evaluate(&json!({"name": "test", "expression": "1+1"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("url is required"));
    }

    #[test]
    fn test_tool_browser_evaluate_missing_expression() {
        let server = McpServer::new();
        let result =
            server.tool_browser_evaluate(&json!({"name": "test", "url": "https://example.com"}));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("expression is required")
        );
    }

    #[test]
    fn test_tool_browser_remove_missing_name() {
        let server = McpServer::new();
        let result = server.tool_browser_remove(&json!({}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("name is required"));
    }

    // === Output truncation tests ===

    #[test]
    fn test_truncate_output_short() {
        let short = "hello world";
        assert_eq!(truncate_output(short), short);
    }

    #[test]
    fn test_truncate_output_at_limit() {
        let exact = "x".repeat(MAX_OUTPUT_BYTES);
        assert_eq!(truncate_output(&exact), exact);
    }

    #[test]
    fn test_truncate_output_long() {
        let long = "x".repeat(MAX_OUTPUT_BYTES + 10_000);
        let result = truncate_output(&long);
        assert!(result.len() < long.len());
        assert!(result.contains("bytes truncated"));
        assert!(result.contains("10000 bytes truncated"));
    }
}
