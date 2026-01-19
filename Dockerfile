# Agentkernel Docker runner
# Runs agentkernel inside a Docker container with KVM support
#
# On macOS with Docker Desktop, this provides a Linux environment with KVM
# for running Firecracker microVMs.
#
# Build:
#   docker build -t agentkernel .
#
# Run (requires --privileged for KVM access):
#   docker run --privileged -it agentkernel

FROM rust:1.82-slim-bookworm AS builder

WORKDIR /build

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Copy source
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY tests ./tests

# Build
RUN cargo build --release

# Runtime image
FROM debian:bookworm-slim

# Install runtime dependencies and Firecracker
RUN apt-get update && apt-get install -y \
    curl \
    ca-certificates \
    iproute2 \
    && rm -rf /var/lib/apt/lists/*

# Download Firecracker (latest release for amd64)
ARG FIRECRACKER_VERSION=v1.7.0
ARG ARCH=x86_64
RUN curl -fsSL "https://github.com/firecracker-microvm/firecracker/releases/download/${FIRECRACKER_VERSION}/firecracker-${FIRECRACKER_VERSION}-${ARCH}.tgz" \
    | tar -xz -C /usr/local/bin \
    && mv /usr/local/bin/release-${FIRECRACKER_VERSION}-${ARCH}/firecracker-${FIRECRACKER_VERSION}-${ARCH} /usr/local/bin/firecracker \
    && chmod +x /usr/local/bin/firecracker \
    && rm -rf /usr/local/bin/release-*

# Copy agentkernel binary
COPY --from=builder /build/target/release/agentkernel /usr/local/bin/

# Copy kernel and config
COPY images/kernel/microvm.config /images/kernel/
COPY images/build /images/build/

WORKDIR /workspace

# Default command shows help
CMD ["agentkernel", "--help"]
