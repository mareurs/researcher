# ── Stage 1: Build ────────────────────────────────────────────────────────────
FROM rust:1.82-slim AS builder

WORKDIR /build

# Cache dependencies separately from source
COPY Cargo.toml Cargo.lock* ./
RUN mkdir src && echo "fn main(){}" > src/main.rs
RUN cargo build --release 2>/dev/null || true
RUN rm src/main.rs

# Build actual source
COPY src ./src
RUN touch src/main.rs && cargo build --release

# ── Stage 2: Runtime ──────────────────────────────────────────────────────────
# Distroless gives us a ~2MB base with no shell, no package manager.
# The Rust binary is statically linked so no libc issues.
FROM gcr.io/distroless/cc-debian12 AS runtime

COPY --from=builder /build/target/release/researcher /usr/local/bin/researcher

ENV RUST_LOG=info
EXPOSE 3000

ENTRYPOINT ["/usr/local/bin/researcher", "--server"]
