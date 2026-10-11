
# API

agentkernel provides two API interfaces for programmatic access, plus official SDK clients in five languages:

- **[HTTP API](http.md)** - REST API for managing sandboxes
- **[MCP Server](mcp.md)** - Model Context Protocol for AI assistant integration
- **[SDKs](../sdks/index.md)** - Client libraries for Node.js, Python, Go, Rust, and Swift

## Quick Comparison

| Feature | HTTP API | MCP Server | SDKs |
|---------|----------|------------|------|
| Protocol | REST over HTTP | JSON-RPC over stdio | Language-native |
| Use case | Scripts, automation | AI assistant integration | Application development |
| Authentication | API key | None (stdio) | API key |
| Sandbox management | Endpoint-dependent | Tool-dependent | SDK-dependent |
| Best for | CI/CD, tooling | Claude Desktop, IDE extensions | Building on agentkernel |
