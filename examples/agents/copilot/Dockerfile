# Copilot CLI Agent Image
#
# This image provides GitHub Copilot CLI in an isolated sandbox environment.
#
# Build:
#   docker build -t agentkernel/copilot .
#
# Run with agentkernel:
#   agentkernel create my-project --image agentkernel/copilot
#   agentkernel start my-project
#   agentkernel attach my-project
#
# Environment variables:
#   GITHUB_TOKEN - Required for Copilot CLI to work

FROM node:22-alpine

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

# Install Copilot CLI globally
RUN npm install -g @githubnext/github-copilot-cli

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
