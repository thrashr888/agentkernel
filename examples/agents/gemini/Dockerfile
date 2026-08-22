# Google Gemini CLI Agent Image
#
# This image provides Google Gemini CLI in an isolated sandbox environment.
#
# Build:
#   docker build -t agentkernel/gemini .
#
# Run with agentkernel:
#   agentkernel create my-project --config agentkernel.toml
#
# Environment variables:
#   GEMINI_API_KEY - Required for Gemini to work

FROM node:24-alpine3.24

ARG GEMINI_CLI_VERSION=0.56.0

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

# Install Google Gemini CLI globally
RUN npm install -g "@google/gemini-cli@${GEMINI_CLI_VERSION}"

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
