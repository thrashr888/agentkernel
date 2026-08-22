# OpenCode Agent Image
#
# This image provides OpenCode CLI in an isolated sandbox environment.
#
# Build:
#   docker build -t agentkernel/opencode .
#
# Run with agentkernel:
#   agentkernel create my-project --config agentkernel.toml
#
# Environment variables:
#   Set your LLM provider's API key as needed (e.g. ANTHROPIC_API_KEY, OPENAI_API_KEY)

FROM node:24-alpine3.24

ARG OPENCODE_VERSION=1.18.21

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

# Install OpenCode into npm's system-wide prefix so the non-root runtime user
# can discover the executable without inheriting root's home directory.
RUN npm install -g "opencode-ai@${OPENCODE_VERSION}"

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
