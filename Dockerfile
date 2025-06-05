# ---------- Stage 1: Build ----------
FROM rust:1.77 as builder

# Set working directory inside the container
WORKDIR /usr/src/url-uploader

# Pre-cache dependencies
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs
RUN cargo build --release
RUN rm -rf src

# Copy actual source code
COPY . .

# Build release binary
RUN cargo build --release

# ---------- Stage 2: Runtime ----------
FROM debian:bullseye-slim

# Install SSL certs (required for HTTPS)
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

# Set working directory
WORKDIR /app

# Copy binary from builder
COPY --from=builder /usr/src/url-uploader/target/release/url-uploader .

# Expose app port (you can change this if the app uses a different one)
EXPOSE 8080

# Run the app
CMD ["./url-uploader"]
