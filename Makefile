# Agentkernel Makefile
# Build standalone executables for multiple platforms

VERSION := $(shell grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)
NAME := agentkernel
BUILD_DIR := target
RELEASE_DIR := dist

.PHONY: all build build-release clean install test lint check help
.PHONY: build-macos build-linux build-all release

# Default target
all: build

# Development build
build:
	cargo build

# Release build (optimized)
build-release:
	cargo build --release

# Run tests
test:
	cargo test

# Run linting and formatting checks
lint:
	cargo fmt -- --check
	cargo clippy -- -D warnings

# Run all checks (before commit)
check: lint test

# Clean build artifacts
clean:
	cargo clean
	rm -rf $(RELEASE_DIR)

# Install locally
install: build-release
	cargo install --path .

# === Cross-platform builds ===

# Build for macOS (Apple Silicon)
build-macos-arm64:
	cargo build --release --target aarch64-apple-darwin

# Build for macOS (Intel)
build-macos-x64:
	cargo build --release --target x86_64-apple-darwin

# Build for Linux (x86_64)
build-linux-x64:
	cargo build --release --target x86_64-unknown-linux-gnu

# Build for Linux (ARM64)
build-linux-arm64:
	cargo build --release --target aarch64-unknown-linux-gnu

# Build for current platform
build-native: build-release

# === Release packaging ===

$(RELEASE_DIR):
	mkdir -p $(RELEASE_DIR)

# Package release binaries
package-macos-arm64: build-macos-arm64 $(RELEASE_DIR)
	cp $(BUILD_DIR)/aarch64-apple-darwin/release/$(NAME) $(RELEASE_DIR)/$(NAME)-$(VERSION)-darwin-arm64
	cd $(RELEASE_DIR) && tar -czf $(NAME)-$(VERSION)-darwin-arm64.tar.gz $(NAME)-$(VERSION)-darwin-arm64

package-macos-x64: build-macos-x64 $(RELEASE_DIR)
	cp $(BUILD_DIR)/x86_64-apple-darwin/release/$(NAME) $(RELEASE_DIR)/$(NAME)-$(VERSION)-darwin-x64
	cd $(RELEASE_DIR) && tar -czf $(NAME)-$(VERSION)-darwin-x64.tar.gz $(NAME)-$(VERSION)-darwin-x64

package-linux-x64: build-linux-x64 $(RELEASE_DIR)
	cp $(BUILD_DIR)/x86_64-unknown-linux-gnu/release/$(NAME) $(RELEASE_DIR)/$(NAME)-$(VERSION)-linux-x64
	cd $(RELEASE_DIR) && tar -czf $(NAME)-$(VERSION)-linux-x64.tar.gz $(NAME)-$(VERSION)-linux-x64

package-linux-arm64: build-linux-arm64 $(RELEASE_DIR)
	cp $(BUILD_DIR)/aarch64-unknown-linux-gnu/release/$(NAME) $(RELEASE_DIR)/$(NAME)-$(VERSION)-linux-arm64
	cd $(RELEASE_DIR) && tar -czf $(NAME)-$(VERSION)-linux-arm64.tar.gz $(NAME)-$(VERSION)-linux-arm64

# === Convenience targets ===

# Build all macOS variants
build-macos: build-macos-arm64 build-macos-x64

# Build all Linux variants
build-linux: build-linux-x64 build-linux-arm64

# Build all platforms (requires cross-compilation toolchains)
build-all: build-macos build-linux

# Package all platforms
release: package-macos-arm64 package-macos-x64 package-linux-x64 package-linux-arm64
	@echo "Release packages created in $(RELEASE_DIR)/"
	@ls -la $(RELEASE_DIR)/

# === Development ===

# Run the CLI
run:
	cargo run -- $(ARGS)

# Run with verbose output
run-verbose:
	RUST_BACKTRACE=1 cargo run -- $(ARGS)

# Format code
fmt:
	cargo fmt

# === Help ===

help:
	@echo "Agentkernel Build System"
	@echo ""
	@echo "Development:"
	@echo "  make build         - Development build"
	@echo "  make build-release - Optimized release build"
	@echo "  make test          - Run tests"
	@echo "  make lint          - Check formatting and lints"
	@echo "  make check         - Run all checks (lint + test)"
	@echo "  make install       - Install to ~/.cargo/bin"
	@echo "  make clean         - Clean build artifacts"
	@echo ""
	@echo "Cross-compilation:"
	@echo "  make build-macos-arm64 - Build for macOS ARM64"
	@echo "  make build-macos-x64   - Build for macOS x64"
	@echo "  make build-linux-x64   - Build for Linux x64"
	@echo "  make build-linux-arm64 - Build for Linux ARM64"
	@echo "  make build-all         - Build all platforms"
	@echo ""
	@echo "Release:"
	@echo "  make release       - Package all platforms"
	@echo ""
	@echo "Usage: make run ARGS='status'"
