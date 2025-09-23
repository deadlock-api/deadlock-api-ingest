# Multi-stage Dockerfile for deadlock-api-ingest
# Stage 1: Build environment
FROM rust:1.90-slim AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    libpcap-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Set working directory
WORKDIR /app

# Copy dependency files first for better caching
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./

# Create a dummy main.rs to build dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs

# Build dependencies (this layer will be cached)
RUN cargo build --release && rm -rf src target/release/deps/deadlock*

# Copy source code
COPY src ./src

# Build the actual application
RUN cargo build --release --locked

# Stage 2: Runtime environment
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    libpcap0.8 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Create a non-root user (though we'll need to run with elevated privileges for packet capture)
RUN groupadd -r deadlock && useradd -r -g deadlock deadlock

# Set working directory
WORKDIR /app

# Copy the binary from builder stage
COPY --from=builder /app/target/release/deadlock-api-ingest /app/deadlock-api-ingest

# Make binary executable
RUN chmod +x /app/deadlock-api-ingest

# Create directory for logs and data
RUN mkdir -p /app/data && chown -R deadlock:deadlock /app

# Set environment variables
ENV RUST_LOG=info
ENV RUST_BACKTRACE=1

# Expose any ports if needed (though this app doesn't serve HTTP)
# The app monitors network traffic, so no ports need to be exposed

# Health check to ensure the application is running
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD pgrep -f deadlock-api-ingest || exit 1

# Note: This application requires network packet capture capabilities
# It must be run with --cap-add=NET_RAW --cap-add=NET_ADMIN or --privileged
# and typically --network=host for proper network monitoring

# Switch to non-root user (though capabilities will be needed)
USER deadlock

# Set the entrypoint
ENTRYPOINT ["/app/deadlock-api-ingest"]
