# syntax=docker/dockerfile:1

# ---------------------------------------------------------------------------
# Stage 1 — build the React/TS frontend into static assets (frontend/dist).
# The SPA talks to the backend over same-origin relative paths, so there is no
# build-time API URL to inject.
# ---------------------------------------------------------------------------
FROM node:22-slim AS frontend
WORKDIR /app/frontend
# Install deps first, off just the manifests, so the layer caches across source
# edits.
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

# ---------------------------------------------------------------------------
# Stage 2 — build the Rust binary. The sqlx query! macros are checked at compile
# time; there is no database here, so we build against the committed offline
# cache (backend/.sqlx) with SQLX_OFFLINE=true. Migrations are embedded into the
# binary by sqlx::migrate!(), so the runtime image needs no migrations dir.
# ---------------------------------------------------------------------------
FROM rust:1-slim-bookworm AS backend
# curl is needed by utoipa-swagger-ui's build script, which fetches the Swagger
# UI assets at compile time.
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential pkg-config curl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app/backend
ENV SQLX_OFFLINE=true

# Warm the dependency cache: compile just the deps against a stub main, so a pure
# source change doesn't re-download and rebuild the whole dependency graph.
COPY backend/Cargo.toml backend/Cargo.lock backend/build.rs ./
RUN mkdir src \
    && echo 'fn main() {}' > src/main.rs \
    && cargo build --release \
    && rm -rf src

# Version string baked into the binary (`meshtrove --version`); the .git dir is
# not in the build context, so build.rs reads this instead of calling git.
# Declared only now: the value changes every build, so putting it any earlier
# would invalidate the dependency-warm layer above on every commit.
ARG APP_VERSION=docker
ENV APP_VERSION=${APP_VERSION}

# Real sources (includes migrations/ and .sqlx/), then the actual build.
COPY backend/ ./
RUN cargo build --release

# ---------------------------------------------------------------------------
# Stage 3 — minimal runtime. Just the binary + the built SPA + a store volume.
# rustls (used by sqlx and reqwest) needs no OpenSSL; ca-certificates covers TLS
# trust roots when talking to a TLS-terminated Postgres.
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home --home-dir /app meshtrove
WORKDIR /app

COPY --from=backend /app/backend/target/release/meshtrove /usr/local/bin/meshtrove
COPY --from=frontend /app/frontend/dist /app/static
RUN mkdir -p /app/store && chown -R meshtrove:meshtrove /app

# Production defaults; every one is overridable at `docker run`/compose time.
# Bind on all interfaces so the port is reachable from outside the container.
ENV MESHTROVE_STATIC_DIR=/app/static \
    MESHTROVE_STORE_DIR=/app/store \
    MESHTROVE_BIND_ADDR=0.0.0.0:3000

USER meshtrove
EXPOSE 3000
# Content-addressed blob store — mount a volume here to persist uploaded files.
VOLUME ["/app/store"]
ENTRYPOINT ["meshtrove"]
