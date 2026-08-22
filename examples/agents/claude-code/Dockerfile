# Claude Code Agent Image
#
# This image provides Claude Code CLI in an isolated sandbox environment.
#
# Build:
#   docker build -t agentkernel/claude-code .
#
# Run with agentkernel:
#   agentkernel create my-project --image agentkernel/claude-code
#   agentkernel start my-project
#   agentkernel attach my-project
#
# Environment variables:
#   ANTHROPIC_API_KEY - Required for Claude Code to work

FROM node:24-alpine3.24

ARG CLAUDE_CODE_VERSION=2.1.239

# Install essential tools
RUN apk add --no-cache \
    git \
    curl \
    bash \
    openssh-client \
    python3 \
    py3-pip \
    ripgrep \
    fd \
    jq \
    && rm -rf /var/cache/apk/*

# Install Claude Code CLI globally
RUN npm install -g "@anthropic-ai/claude-code@${CLAUDE_CODE_VERSION}"

# Create workspace directory
RUN mkdir -p /workspace
WORKDIR /workspace

# Set up a non-root user for better security
RUN adduser -D -h /home/developer developer \
    && chown -R developer:developer /workspace

# Switch to non-root user
USER developer

# Set up shell
ENV SHELL=/bin/bash
ENV HOME=/home/developer

# Keep container running (for attach/exec)
CMD ["tail", "-f", "/dev/null"]
