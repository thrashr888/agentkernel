# Stock Claude Code example (non-root). It is not an OpenSSH/Rex image.
#
# From the repository root:
#   agentkernel build --tag agentkernel/claude-code examples/agents/claude-code
#   agentkernel sandbox create my-project --backend docker --image agentkernel/claude-code
#   agentkernel exec my-project -- claude --version
#
# Authenticate inside your own guest with Claude's native login, or configure
# AgentKernel's secret integration for API-key usage. Never build credentials
# into this image or publish a snapshot of an authenticated guest.

FROM node:24-alpine3.24

ARG CLAUDE_CODE_VERSION=2.1.327

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

# Fail the image build if the pinned CLI does not run as the runtime user.
RUN claude --version | grep -Fx "${CLAUDE_CODE_VERSION} (Claude Code)"

# Keep container running (for attach/exec)
CMD ["tail", "-f", "/dev/null"]
