# Amp Agent Image
#
# This image provides Amp (Sourcegraph) CLI in an isolated sandbox environment.
#
# Build:
#   docker build -t agentkernel/amp .
#
# Run with agentkernel:
#   agentkernel create my-project --config agentkernel.toml
#
# Environment variables:
#   ANTHROPIC_API_KEY - Required for Amp to work

# Amp publishes glibc Linux binaries, so use the maintained Node 24 Debian
# variant instead of Alpine/musl.
FROM node:24-bookworm-slim

ARG AMP_VERSION=0.0.1787342526-gc11bfb

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

# Amp's supported npm package moved from @sourcegraph/amp to @ampcode/cli.
RUN npm install -g --allow-scripts=@ampcode/cli "@ampcode/cli@${AMP_VERSION}"

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
