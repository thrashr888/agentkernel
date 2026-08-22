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

# Copilot's current native musl artifact aborts during startup on Alpine 3.24,
# so use the maintained Node 24 Debian variant for this image.
FROM node:24-bookworm-slim

ARG COPILOT_CLI_VERSION=1.0.80

# Install essential tools
RUN apt-get update && apt-get install -y --no-install-recommends \
    git \
    curl \
    bash \
    openssh-client \
    python3 \
    python3-pip \
    ripgrep \
    fd-find \
    jq \
    && ln -s /usr/bin/fdfind /usr/local/bin/fd \
    && rm -rf /var/lib/apt/lists/*

# Install the supported GitHub Copilot CLI package. The old
# @githubnext/github-copilot-cli package is discontinued.
RUN npm install -g "@github/copilot@${COPILOT_CLI_VERSION}"

# Create workspace directory
RUN mkdir -p /workspace
WORKDIR /workspace

# Set up a non-root user for better security
RUN useradd -m -d /home/developer -s /bin/bash developer \
    && chown -R developer:developer /workspace

# Switch to non-root user
USER developer

# Set up shell
ENV SHELL=/bin/bash
ENV HOME=/home/developer

# Keep container running (for attach/exec)
CMD ["tail", "-f", "/dev/null"]
