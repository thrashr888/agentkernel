# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**agentkernel** - [Add project description here]

## Build Commands

```bash
# Build the project
[Add build command]

# Run tests
[Add test command]

# Run the project
[Add run command]
```

## Architecture Overview

[Describe your project architecture here]

## Development Workflow

[Describe your development workflow here]

## Beads Issue Tracking

This repository uses `bd` (beads) for issue tracking.

### Essential Beads Commands

```bash
# Create issues
bd create --title="Implement feature X" --type=feature --priority=1

# List and filter
bd list --status=open
bd ready                    # Show unblocked work

# Update work
bd update <id> --status=in_progress
bd close <id>

# Dependencies
bd dep add <issue> <depends-on>
```

Use beads for tracking multi-session work and complex features with dependencies.
