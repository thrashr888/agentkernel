# Agentkernel multi-stage build for Kubernetes/Nomad deployment
#
# Builds agentkernel with orchestrator backends enabled, producing a
# minimal runtime image suitable for deployment as a K8s Deployment
# or Nomad service job.
#
# Build:
#   docker build -t agentkernel .
#
# Run:
#   docker run -p 18888:18888 agentkernel

# --- Builder stage ---
FROM rust:1.89-slim-bookworm AS builder

WORKDIR /build

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests first for layer caching
COPY Cargo.toml Cargo.lock ./

# Copy source and directories referenced by include_str!()
COPY src ./src
COPY tests ./tests
COPY guest-agent ./guest-agent
COPY claude-plugin ./claude-plugin
COPY plugins ./plugins
COPY templates ./templates
COPY examples/agents/tested-versions.json ./examples/agents/tested-versions.json
COPY docker ./docker
COPY images/build ./images/build
COPY images/kernel/microvm.config ./images/kernel/microvm.config

# Build release binary with orchestrator features
RUN cargo build --release --features kubernetes,nomad

# --- Runtime stage ---
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Copy the compiled binary
COPY --from=builder /build/target/release/agentkernel /usr/local/bin/agentkernel

EXPOSE 18888

ENTRYPOINT ["agentkernel"]
CMD ["serve", "--host", "0.0.0.0", "--port", "18888"]
