
# API

agentkernel provides two API interfaces for programmatic access, plus official SDK clients in four languages:

- **[HTTP API](api-http.md)** - REST API for managing sandboxes
- **[MCP Server](api-mcp.md)** - Model Context Protocol for AI assistant integration
- **[SDKs](sdks.md)** - Client libraries for Node.js, Python, Rust, and Swift

## Quick Comparison

| Feature | HTTP API | MCP Server | SDKs |
|---------|----------|------------|------|
| Protocol | REST over HTTP | JSON-RPC over stdio | Language-native |
| Use case | Scripts, automation | AI assistant integration | Application development |
| Authentication | API key | None (stdio) | API key |
| Sandbox management | Full | Full | Full |
| Best for | CI/CD, tooling | Claude Desktop, IDE extensions | Building on agentkernel |
