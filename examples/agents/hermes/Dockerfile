# Hermes Agent Image
#
# This image provides Hermes Agent (NousResearch) in an isolated sandbox environment.
#
# Build:
#   docker build -t agentkernel/hermes .
#
# Run with agentkernel:
#   agentkernel create my-project --config agentkernel.toml
#
# Environment variables:
#   OPENROUTER_API_KEY - Recommended (access to all models via OpenRouter)
#   ANTHROPIC_API_KEY  - Alternative (direct Anthropic access)

FROM node:24-bookworm-slim

ARG HERMES_VERSION=0.20.5
ARG HERMES_COMMIT=fc7523ca31eeb6eff9114afe384c2cf6380359df

# Install essential tools
RUN apt-get update && apt-get install -y --no-install-recommends \
    git \
    curl \
    bash \
    openssh-client \
    python3 \
    python3-pip \
    python3-venv \
    ripgrep \
    jq \
    && rm -rf /var/lib/apt/lists/*

# Install the tested Hermes release from an immutable upstream commit. Keep its
# Python environment outside the checkout, matching upstream's guidance.
RUN git init /opt/hermes-agent \
    && git -C /opt/hermes-agent remote add origin https://github.com/NousResearch/hermes-agent.git \
    && git -C /opt/hermes-agent fetch --depth 1 origin "${HERMES_COMMIT}" \
    && git -C /opt/hermes-agent checkout --detach FETCH_HEAD \
    && test "$(python3 -c 'import tomllib; print(tomllib.load(open("/opt/hermes-agent/pyproject.toml", "rb"))["project"]["version"])')" = "${HERMES_VERSION}" \
    && python3 -m venv /opt/hermes-venv \
    && /opt/hermes-venv/bin/pip install --no-cache-dir --upgrade pip \
    && /opt/hermes-venv/bin/pip install --no-cache-dir -e "/opt/hermes-agent[all]" \
    && ln -s /opt/hermes-venv/bin/hermes /usr/local/bin/hermes

# Create workspace directory
RUN mkdir -p /workspace
WORKDIR /workspace

# Set up a non-root user for better security
RUN useradd -m -d /home/developer -s /bin/bash developer \
    && chown -R developer:developer /workspace

# Switch to non-root user
USER developer

# Set up shell and PATH
ENV SHELL=/bin/bash
ENV HOME=/home/developer

# Keep container running (for attach/exec)
CMD ["tail", "-f", "/dev/null"]
