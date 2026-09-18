# SPDX-License-Identifier: AGPL-3.0-or-later
# Multi-stage build for the modelwrite service (mw-server).
#
# A single image serves both the PostgreSQL deployment and the SQLite fallback:
# the postgres feature is a default feature of the server crate. The runtime image
# carries no C toolchain and runs as a non-root user (uid 65532). The root
# filesystem is meant to be mounted read-only (see the compose file and chart);
# the only writable path is the /data volume (SQLite database and gate evidence).

# ---- builder ---------------------------------------------------------------
# The server crate declares rust-version = "1.85". Override RUST_VERSION to build
# with a newer toolchain if a dependency ever raises the floor.
ARG RUST_VERSION=1.85
FROM rust:${RUST_VERSION}-slim AS builder

# rusqlite is built with the "bundled" feature (compiles SQLite from C) and
# rustls/ring need a C compiler too, so build-essential is required AT BUILD TIME
# only. ca-certificates is for the crate fetches over HTTPS; it does not leak into
# the runtime image.
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# Copy the workspace manifest and every workspace member. Cargo must be able to
# load the WHOLE workspace even though only mw-server is built, so the engine,
# server and cli trees are all present.
COPY Cargo.toml Cargo.lock ./
COPY engine ./engine
COPY server ./server
COPY cli ./cli

# --locked keeps the build reproducible against the committed Cargo.lock. The
# postgres feature is on by default in server/Cargo.toml, so MW_DATABASE_URL works.
RUN cargo build --release --locked -p mw-server

# ---- runtime ----------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

# ca-certificates is the system trust store that rustls-native-certs reads to
# verify the TLS certificate of the PostgreSQL server; it is the only package the
# runtime image needs. Create a dedicated non-root user for the service.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 65532 mw \
    && useradd --system --uid 65532 --gid 65532 --no-create-home \
        --home-dir /nonexistent --shell /usr/sbin/nologin mw

COPY --from=builder /src/target/release/mw-server /usr/local/bin/mw-server
COPY deploy/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

# /data is the writable volume (SQLite database and gate evidence). These
# defaults make the SQLite path work with no extra configuration once /data is
# mounted; the chart and compose file mount a read-write volume here.
#
# The directory must be CREATED with the service user as its owner, before the
# user switch below. A named volume is copy-populated from the image path on
# first use, so a root-owned /data would give the volume root ownership too, and
# the non-root service could not create its database or its evidence directory:
# the trial would crash-loop on first up. Kubernetes has fsGroup to fix this;
# Docker does not, so the image has to get it right.
RUN install -d -m 0750 -o 65532 -g 65532 /data
WORKDIR /data
ENV MW_DB=/data/modelwrite.db \
    MW_EVIDENCE_DIR=/data/evidence

USER 65532:65532

EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
