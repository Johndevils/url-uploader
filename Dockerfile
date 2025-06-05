# Stage 1: Builder
FROM rust:1.77 as builder

WORKDIR /usr/src/app

# Cache dependencies
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release && rm -rf src

# Copy full source and build
COPY . .
RUN cargo build --release

# Stage 2: Runtime (use newer Debian base with GLIBC ≥ 2.34)
FROM debian:bookworm-slim

# Install CA certs
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the Rust binary from builder stage
COPY --from=builder /usr/src/app/target/release/url-uploader .

# Expose app port
EXPOSE 8080

# Run the app
CMD ["./url-uploader"]
