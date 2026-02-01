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
FROM rust:1.83-slim AS builder

WORKDIR /build

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests first for layer caching
COPY Cargo.toml Cargo.lock ./

# Copy source
COPY src ./src
COPY tests ./tests

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
