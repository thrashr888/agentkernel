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

FROM nikolaik/python-nodejs:python3.11-nodejs22

# Install essential tools
RUN apt-get update && apt-get install -y --no-install-recommends \
    git \
    curl \
    bash \
    openssh-client \
    ripgrep \
    jq \
    && rm -rf /var/lib/apt/lists/*

# Clone and install Hermes Agent
RUN git clone --depth 1 https://github.com/NousResearch/hermes-agent.git /opt/hermes-agent \
    && cd /opt/hermes-agent \
    && git submodule update --init --depth 1 \
    && pip install --no-cache-dir -e ".[all]" \
    && pip install --no-cache-dir -e "./mini-swe-agent" \
    && npm install

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
