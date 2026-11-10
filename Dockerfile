FROM rust:1-bookworm AS rust-build
RUN apt-get update && apt-get install -y --no-install-recommends libpq-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY migrations ./migrations
COPY web ./web
RUN cargo build --release --locked

FROM pgvector/pgvector:0.8.7-pg17-bookworm
LABEL org.opencontainers.image.title="Code Atlas" \
      org.opencontainers.image.description="Native code graph, MCP, review and analytics with PostgreSQL"
RUN apt-get update && apt-get install -y --no-install-recommends libpq5 && rm -rf /var/lib/apt/lists/*
COPY --from=rust-build /build/target/release/code-atlas /usr/local/bin/code-atlas
COPY docker/entrypoint.sh /usr/local/bin/atlas-entrypoint
COPY docker/001-vector.sql /docker-entrypoint-initdb.d/001-vector.sql
RUN chmod +x /usr/local/bin/atlas-entrypoint
WORKDIR /opt/code-atlas
ENV POSTGRES_DB=atlas POSTGRES_USER=atlas PGDATABASE=atlas PGUSER=atlas \
    ATLAS_PROJECT_ROOT=/project ATLAS_BIND=0.0.0.0:8080 \
    ATLAS_DATABASE_URL="host=127.0.0.1 connect_timeout=5"
EXPOSE 8080
STOPSIGNAL SIGTERM
HEALTHCHECK --interval=15s --timeout=10s --start-period=120s --retries=3 \
    CMD pg_isready -h 127.0.0.1 -U "$POSTGRES_USER" -d "$POSTGRES_DB" && code-atlas healthcheck
ENTRYPOINT ["/usr/local/bin/atlas-entrypoint"]
CMD ["serve"]
