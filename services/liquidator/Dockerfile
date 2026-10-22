# ==============================================================================
# Multi-stage production build for vigil-liquidator microservice
#
# Context: Monorepo root directory
# Usage:
#   docker build -t vigil-liquidator -f services/liquidator/Dockerfile .
# ==============================================================================

# ------------------------------------------------------------------------------
# Stage 1: Build stage (official Rust Debian Bookworm image)
# ------------------------------------------------------------------------------
    FROM rust:1.94-bookworm AS builder

WORKDIR /src

# Copy database migrations (required at compile time by sqlx::migrate!("../../migrations"))
COPY migrations/ ./migrations/

# Copy services workspace (Cargo.toml, Cargo.lock, common, liquidator, and other workspace crates)
COPY services/ ./services/

# Build release binary for vigil-liquidator
WORKDIR /src/services
RUN cargo build --release -p vigil-liquidator

# ------------------------------------------------------------------------------
# Stage 2: Lean runtime stage (Debian Bookworm slim)
# ------------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

# Install CA certificates for TLS/SSL RPC and NATS communication
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the compiled release binary from builder stage
COPY --from=builder /src/services/target/release/vigil-liquidator /usr/local/bin/vigil-liquidator

# Create convenient aliases
RUN ln -s /usr/local/bin/vigil-liquidator /usr/local/bin/liquidator

# Production entrypoint
ENTRYPOINT ["/usr/local/bin/vigil-liquidator"]
