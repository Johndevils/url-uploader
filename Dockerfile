# Stage 1: Build the application
FROM rust:1.77 as builder

WORKDIR /usr/src/app

# Copy the manifest and lock files
COPY Cargo.toml Cargo.lock ./

# Create a dummy main.rs to build dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs

# Build dependencies to cache them
RUN cargo build --release && rm -rf src

# Copy the actual source code
COPY . .

# Build the application
RUN cargo build --release

# Stage 2: Create the runtime image
FROM debian:buster-slim

# Install necessary packages
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

# Set the working directory
WORKDIR /app

# Copy the compiled binary from the builder stage
COPY --from=builder /usr/src/app/target/release/url-uploader .

# Expose the application port
EXPOSE 8080

# Define the default command
CMD ["./url-uploader"]
