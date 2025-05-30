# Use a Rust image for building the application
FROM rust:1.85.0-slim-bullseye AS builder

# Create a new empty shell project
WORKDIR /usr/src/docker_mirrors
COPY . .

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

# Build the application with release profile
RUN cargo build --release

# Use a smaller image for the runtime environment
FROM debian:bullseye-slim

# Install necessary runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy the built binary from the builder stage
COPY --from=builder /usr/src/docker_mirrors/target/release/docker_mirrors /usr/local/bin/docker_mirrors

# Expose the port the app runs on
EXPOSE 8080

# Run the binary
CMD ["docker_mirrors"]
