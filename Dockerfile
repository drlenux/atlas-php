# App / MCP only. PostgreSQL is a separate Compose service (official postgres image).
FROM rust:1-bookworm AS rust-build
RUN apt-get update && apt-get install -y --no-install-recommends libpq-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY migrations ./migrations
COPY web ./web
RUN cargo build --release --locked

FROM debian:bookworm-slim
LABEL org.opencontainers.image.title="Code Atlas" \
      org.opencontainers.image.description="Native code graph, MCP, review and analytics"
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libpq5 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=rust-build /build/target/release/code-atlas /usr/local/bin/code-atlas
WORKDIR /opt/code-atlas
ENV ATLAS_PROJECT_ROOT=/project ATLAS_BIND=0.0.0.0:8080
EXPOSE 8080
STOPSIGNAL SIGTERM
HEALTHCHECK --interval=15s --timeout=5s --start-period=30s --retries=3 \
    CMD code-atlas healthcheck
ENTRYPOINT ["code-atlas"]
CMD ["serve"]
