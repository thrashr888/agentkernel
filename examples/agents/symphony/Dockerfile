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

ARG CODEX_VERSION=0.149.0
ARG SYMPHONY_VERSION=0.0.2
ARG SYMPHONY_COMMIT=8001b52e3062495a16e520e4ceaf8f9de868c4d0

# Install essential tools and Node.js
RUN apt-get update && apt-get install -y --no-install-recommends \
    git \
    curl \
    bash \
    ca-certificates \
    gnupg \
    openssh-client \
    && curl -fsSL https://deb.nodesource.com/setup_24.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/*

# Install OpenAI Codex CLI
RUN npm install -g "@openai/codex@${CODEX_VERSION}"

# Install hex and rebar
RUN mix local.hex --force && mix local.rebar --force

# Clone and build the tested Symphony reference implementation from an
# immutable upstream commit, then expose the escript on the system PATH.
RUN git init /opt/symphony \
    && git -C /opt/symphony remote add origin https://github.com/openai/symphony.git \
    && git -C /opt/symphony fetch --depth 1 origin "${SYMPHONY_COMMIT}" \
    && git -C /opt/symphony checkout --detach FETCH_HEAD \
    && test "$(sed -n 's/.*version: \"\([^\"]*\)\".*/\1/p' /opt/symphony/elixir/mix.exs | head -n 1)" = "${SYMPHONY_VERSION}" \
    && cd /opt/symphony/elixir \
    && MIX_ENV=prod mix setup \
    && MIX_ENV=prod mix build \
    && ln -s /opt/symphony/elixir/bin/symphony /usr/local/bin/symphony

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
