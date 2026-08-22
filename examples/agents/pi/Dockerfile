# Pi Agent Image
#
# This image provides Pi (Mario Zechner's coding agent) in an isolated sandbox environment.
#
# Build:
#   docker build -t agentkernel/pi .
#
# Run with agentkernel:
#   agentkernel create my-project --config agentkernel.toml
#
# Environment variables:
#   ANTHROPIC_API_KEY or OPENAI_API_KEY - Required for Pi to work

FROM node:24-alpine3.24

ARG PI_VERSION=0.84.2

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

# Pi's supported package moved from @mariozechner to @earendil-works.
RUN npm install -g "@earendil-works/pi-coding-agent@${PI_VERSION}"

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
