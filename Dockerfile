# syntax=docker/dockerfile:1

###############################
# Builder Stage (Debian-based)
###############################
FROM rust:1.86.0 AS builder
WORKDIR /app

# Install build dependencies, including OpenSSL development package.
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    clang \
    lld \
    && rm -rf /var/lib/apt/lists/*

# Copy Cargo manifest and lockfile for dependency caching.
COPY Cargo.toml Cargo.lock ./

# Copy the source files.
COPY src ./src

# Build the project in release mode.
RUN cargo build --release --locked

# Rename the produced binary.
# (Assuming your Cargo project name is "DynaRust" so that the binary is named "DynaRust" by default)
RUN cp target/release/DynaRust /server

###############################
# Runtime Stage (Debian Bookworm Slim)
###############################
FROM debian:bookworm-slim
WORKDIR /app

# Install the OpenSSL runtime library (which provides libssl.so.3).
RUN apt-get update && apt-get install -y \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Copy the built server binary from the builder stage.
COPY --from=builder /server /usr/local/bin/server

# Expose port 6660.
EXPOSE 6660

# Set the entrypoint.
# By default, the container will run:
#   /usr/local/bin/server 0.0.0.0:6660
# Any extra arguments passed to 'docker run' will be appended to the command.
ENTRYPOINT ["/usr/local/bin/server", "0.0.0.0:6660"]
