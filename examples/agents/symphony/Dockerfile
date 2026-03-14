# Symphony Agent Image
#
# This image provides OpenAI Symphony orchestration daemon in an isolated sandbox environment.
#
# Build:
#   docker build -t agentkernel/symphony .
#
# Run with agentkernel:
#   agentkernel create my-project --config agentkernel.toml
#
# Environment variables:
#   LINEAR_API_KEY - Required for Linear issue tracker integration
#   OPENAI_API_KEY - Required for Codex agent

FROM elixir:1.19-otp-28-slim

# Install essential tools and Node.js
RUN apt-get update && apt-get install -y --no-install-recommends \
    git \
    curl \
    bash \
    ca-certificates \
    gnupg \
    openssh-client \
    && curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/*

# Install OpenAI Codex CLI
RUN npm install -g @openai/codex

# Install hex and rebar
RUN mix local.hex --force && mix local.rebar --force

# Clone and build Symphony
RUN git clone --depth 1 https://github.com/openai/symphony.git /opt/symphony \
    && cd /opt/symphony/elixir \
    && mix deps.get \
    && mix compile

# Create workspace directory
RUN mkdir -p /workspace
WORKDIR /workspace

# Set up a non-root user for better security
RUN useradd -m -d /home/developer -s /bin/bash developer \
    && chown -R developer:developer /workspace /opt/symphony

# Switch to non-root user
USER developer

# Set up shell
ENV SHELL=/bin/bash
ENV HOME=/home/developer

# Keep container running (for attach/exec)
CMD ["tail", "-f", "/dev/null"]
