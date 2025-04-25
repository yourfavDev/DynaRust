# syntax=docker/dockerfile:1

###############################
# Builder Stage (Debian-based)
###############################
FROM rust:1.86.0 AS builder
WORKDIR /app

# Install build deps: pkg-config + OpenSSL headers + a linker
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    clang \
    lld \
  && rm -rf /var/lib/apt/lists/*

# Copy only manifests to leverage Docker cache
COPY Cargo.toml Cargo.lock ./

# Copy source + any keys
COPY src ./src
COPY encryption.key .

# Tell openssl-sys to stop vendoring OpenSSL and to use pkg-config instead
ENV OPENSSL_NO_VENDOR=1

# Build
RUN cargo build --release --locked

# Copy binary for next stage
RUN cp target/release/DynaRust /server

###############################
# Runtime Stage (Debian Bookworm Slim)
###############################
FROM debian:bookworm-slim
WORKDIR /app

# Install just the runtime libssl
RUN apt-get update && apt-get install -y \
    libssl3 \
  && rm -rf /var/lib/apt/lists/*

# Pull in the server
COPY --from=builder /server /usr/local/bin/server

EXPOSE 6660
ENTRYPOINT ["/usr/local/bin/server", "0.0.0.0:6660"]
