# TaskForge — multi-stage build producing the API and worker binaries.
#
# See docs/48-DEPLOYMENT-PROFILES.md for the three supported shapes and
# docs/52-DEPLOYMENT-GUIDE.md for the operator walkthrough.
#
#   docker build -t taskforge:dev .
#   docker run --rm taskforge:dev --version
#
# Design notes:
#   * Distroless, not scratch. TLS roots, timezone data, and the ability to get
#     a shell into a container during an incident are worth the megabytes
#     (docs/19 §Workspace-level policy).
#   * Dependencies are built in their own layer so a source-only change does not
#     recompile the dependency graph. On a workspace this size that is the
#     difference between a 20-second and a 4-minute rebuild.
#   * The image runs as a non-root user. This is separate from — and as
#     important as — the database role not being a superuser (migration 0012).

# ---------------------------------------------------------------------------
# Stage 1 — browser application
# ---------------------------------------------------------------------------
FROM node:24-bookworm-slim AS web-builder
WORKDIR /build/webapp

RUN corepack enable && corepack prepare pnpm@10.33.4 --activate
COPY webapp/package.json webapp/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY webapp ./
RUN pnpm build

# ---------------------------------------------------------------------------
# Stage 2 — Rust dependency cache
# ---------------------------------------------------------------------------
FROM rust:1.96-slim-bookworm AS planner
WORKDIR /build

RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev \
 && rm -rf /var/lib/apt/lists/*

# Manifests only. Copying the full source here would defeat the cache layer.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
COPY tools tools

# Replace every source file with a stub so cargo resolves and compiles the
# dependency graph without the real code. Both a lib and a bin stub are written
# for every crate: cargo builds whichever the manifest implies, and an extra
# unused stub is harmless. Doing it unconditionally keeps this POSIX sh and
# avoids per-crate special cases that rot as crates are added.
RUN set -eux; \
    find crates tools -name '*.rs' -delete; \
    for d in crates/* tools/*; do \
        mkdir -p "$d/src"; \
        printf 'fn main() {}\n' > "$d/src/main.rs"; \
        printf '\n'            > "$d/src/lib.rs"; \
    done; \
    cargo build --release --locked --bin casual-task-api --bin casual-task-worker

# ---------------------------------------------------------------------------
# Stage 3 — Rust build
# ---------------------------------------------------------------------------
FROM planner AS builder
WORKDIR /build

# Real sources now; the dependency layer above is reused.
COPY crates crates
COPY tools tools
COPY migrations migrations

# Touch so cargo does not trust the stub fingerprints.
RUN find crates tools -name '*.rs' -exec touch {} + \
 && cargo build --release --locked --bin casual-task-api --bin casual-task-worker \
 && strip target/release/casual-task-api target/release/casual-task-worker

# ---------------------------------------------------------------------------
# Stage 4 — runtime
# ---------------------------------------------------------------------------
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime

# Migrations ship IN the image so the version of the schema and the version of
# the code that expects it can never disagree (docs/48 §Migrations on deploy).
COPY --from=builder /build/migrations /app/migrations
COPY --from=web-builder /build/webapp/dist /app/webapp
COPY --from=builder /build/target/release/casual-task-api    /usr/local/bin/taskforge-api
COPY --from=builder /build/target/release/casual-task-worker /usr/local/bin/taskforge-worker

# distroless :nonroot is uid 65532. Declared explicitly so a base-image change
# cannot silently promote the process to root.
USER 65532:65532
WORKDIR /app
ENV TF_WEB_ROOT=/app/webapp

EXPOSE 8080

# The API serves /health/live, which must NOT touch the database — a liveness
# probe that fails during a database blip restarts every healthy instance at
# once and turns a partial outage into a total one (docs/46 §Health endpoints).
# Readiness is checked by the orchestrator against /health/ready, not here.

ENTRYPOINT ["/usr/local/bin/taskforge-api"]

LABEL org.opencontainers.image.title="TaskForge" \
      org.opencontainers.image.description="Work tracking with explainable permissions and a plugin-first core" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.source="https://github.com/CasualOffice/TaskForge"
