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
# Same version stamp the backend stage bakes in, for the same reason: no .git in
# the build context, so vite.config.ts reads this instead of calling git. The SPA
# compares its own stamp against /api/version to notice it is running against a
# redeployed server; left unset it would stamp "unknown" and never match.
# Declared after the npm layers so a new commit doesn't invalidate them.
ARG APP_VERSION=docker
ENV APP_VERSION=${APP_VERSION}
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
# Stage 3 — runtime. The binary + the built SPA + a store volume + the preview
# renderer. rustls (used by sqlx and reqwest) needs no OpenSSL; ca-certificates
# covers TLS trust roots when talking to a TLS-terminated Postgres.
#
# libarchive-tools provides bsdtar, the unpacker for every archive format that
# isn't zip: tar and its compressed forms, 7z, and rar (libarchive reads rar5
# with its own code, so no non-free unrar source is involved). Without it those
# imports fail the unpack job — see services/importer.rs.
#
# trixie, not bookworm, for f3d: bookworm only has f3d 1.3.1 (2021), trixie has
# 3.1.0. The upstream .deb releases are not an option — they are x86_64-only and
# this image is built for arm64. The backend binary is built on bookworm and
# runs here fine; glibc is backward compatible, so older-built runs on newer.
# ---------------------------------------------------------------------------
FROM debian:trixie-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        f3d \
        libarchive-tools \
        xvfb \
        xauth \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home --home-dir /app meshtrove

# f3d needs a GL context, and a container has no display. Both headless
# backends f3d offers are dead ends here: EGL fails ("Cannot use a EGL context
# on this platform") and OSMesa is refused outright ("the underlying VTK version
# is not recent enough"), so a virtual X server it is.
#
# This wrapper shadows /usr/bin/f3d on PATH so the renderer setting stays the
# plain `f3d` that works on a developer's machine — the display plumbing is a
# property of this image, not of the app's configuration. `-a` picks a free
# display number, so concurrent render jobs don't collide.
RUN printf '#!/bin/sh\nexec xvfb-run -a /usr/bin/f3d "$@"\n' > /usr/local/bin/f3d \
    && chmod +x /usr/local/bin/f3d

# f3d insists on a writable cache directory and has no flag to turn it off.
# It derives one from XDG_CACHE_HOME, else $HOME/.cache — and when the container
# runs as a uid with no passwd entry (which is exactly what MESHTROVE_UID does),
# Docker sets HOME=/, so f3d tries to create /.cache/f3d and dies with
# "Permission denied". Point it at a directory any uid can write: mode 1777 like
# /tmp, so the sticky bit still stops one user removing another's files.
RUN mkdir -p /var/cache/meshtrove && chmod 1777 /var/cache/meshtrove
ENV XDG_CACHE_HOME=/var/cache/meshtrove
WORKDIR /app

COPY --from=backend /app/backend/target/release/meshtrove /usr/local/bin/meshtrove
COPY --from=frontend /app/frontend/dist /app/static
# chmod 755 is load-bearing, not tidying. /app is the meshtrove user's home, and
# trixie's useradd defaults HOME_MODE to 0700 (bookworm left it to UMASK 022, so
# 0755). At 0700 nothing but uid 10001 can even *traverse* /app — so running the
# container as another uid, which docker-compose.prod.yml exists to support for
# bind-mounted stores (MESHTROVE_UID/GID), fails with "Permission denied"
# creating /app/store/imports. Set the mode explicitly so a future base bump
# cannot quietly change it again.
RUN mkdir -p /app/store && chown -R meshtrove:meshtrove /app && chmod 755 /app /app/store

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
