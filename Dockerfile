# Multi-stage Dockerfile for building and running sqew

FROM rust:1-bookworm AS builder
WORKDIR /app

# Cache dependencies
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo "fn main(){}" > src/main.rs && cargo build --release || true

# Build
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
         ca-certificates libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -u 10001 sqew

COPY --from=builder /app/target/release/sqew /usr/local/bin/sqew

WORKDIR /data
VOLUME ["/data"]
ENV SQEW_BIND=0.0.0.0
EXPOSE 8888

USER sqew
ENTRYPOINT ["/usr/local/bin/sqew"]
CMD ["serve", "--port", "8888"]

