---
title: API Overview
permalink: /api.html
sidebar: agentkernel_sidebar
topnav: topnav
---

# API

agentkernel provides two API interfaces for programmatic access:

- **HTTP API** - REST API for managing sandboxes
- **MCP Server** - Model Context Protocol for AI assistant integration

## Quick Comparison

| Feature | HTTP API | MCP Server |
|---------|----------|------------|
| Protocol | REST over HTTP | JSON-RPC over stdio |
| Use case | Scripts, automation | AI assistant integration |
| Authentication | API key | None (stdio) |
| Sandbox management | Full | Full |
| Best for | CI/CD, tooling | Claude Desktop, IDE extensions |
