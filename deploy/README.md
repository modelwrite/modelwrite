# Deploying modelwrite

This directory turns the repository into software an organisation can install: a
container image, a Docker Compose trial stack, and a Helm chart for Kubernetes.

**WARNING — OPEN MODE. mw-server runs WITHOUT authentication when neither a token
nor a JWKS is configured, and in that mode every request is accepted as an
anonymous administrator. The container image refuses to start in that state
unless the operator sets `MW_ALLOW_OPEN=yes` explicitly. Never run anything
reachable by more than one person without `MW_AUTH_TOKEN` (a shared bearer
token) or `MW_AUTH_JWKS` (a JWKS file for signed JWTs).**

## What CI proves

The `ci` workflow (`.github/workflows/ci.yml`) runs the parts of this directory
that a machine without Docker or PostgreSQL cannot, on every push:

- **The container image builds.** The `deploy` job runs
  `docker build -f deploy/Dockerfile -t modelwrite/modelwrite:ci .`, so the
  Dockerfile's stages, paths, commands and non-root runtime are exercised rather
  than inspected.
- **The Helm chart lints.** The same job runs `helm lint deploy/helm/modelwrite`,
  which renders every template and fails on a malformed template or an invalid
  `Chart.yaml`/`values.yaml`.
- **Every plain YAML file parses.** A PyYAML pass over `docker-compose.yml`,
  `Chart.yaml`, `values.yaml` and `templates/secret.yaml.example` fails the job on
  a syntax error. (The Go-template `templates/*.yaml` files are not YAML until
  Helm renders them; `helm lint` is their check.)
- **The PostgreSQL backend executes.** The `postgres` job starts a real
  `postgres:16` service container, sets `MW_TEST_DATABASE_URL`, and runs
  `cargo test --workspace --include-ignored`, so the 13 PostgreSQL tests that
  skip locally actually run.

What CI still does **not** prove: the chart is not **installed** into a live
Kubernetes cluster, and `docker compose up` is not run end to end (the compose
file is YAML-validated and the image is built, but the trial stack is not booted).
Those remain an operator's first-run responsibility.

## What the chart does NOT do

Operate on the truth, not on a wish list. This chart does **not** provide:

- **TLS termination** — the service speaks plain HTTP on port 8080 and expects a
  proxy/ingress in front of it. No certificate management is included.
- **Backups** — the PostgreSQL data and the evidence volume are not backed up.
  You are responsible for database backups and for keeping the gate evidence.
- **High availability** — one replica, no leader election, no failover. The
  SQLite backend is a single file and must never be shared between replicas.
- **Secret management** — the chart references Secrets by name but does not
  create or rotate them (see `templates/secret.yaml.example`).
- **Database migration jobs or a managed database** — it expects an existing
  PostgreSQL URL (or falls back to SQLite on the persistent volume).

## The container image

Build from the repository root (the build context is the root, not `deploy/`):

    docker build -f deploy/Dockerfile -t modelwrite/modelwrite:0.1.0 .

- Multi-stage: the binary is compiled in a `rust` image and copied into a slim
  `debian:bookworm-slim` runtime with **no C toolchain**.
- Runs as a **non-root user** (uid/gid 65532), with a read-only root filesystem
  where the compose file and chart set `read_only`/`readOnlyRootFilesystem`.
- The only writable path is `/data` (a volume): the SQLite database and the
  gate evidence live there.
- Building the image needs network access to crates.io (or a vendored registry).
  The *air-gapped* path is about **deploying the already-built image**, described
  next.

Environment variables (the exact names the server reads): `MW_DATABASE_URL`
(PostgreSQL URL) or `MW_DB` (SQLite path), `MW_EVIDENCE_DIR`, `MW_BIND`
(default `127.0.0.1`; set `0.0.0.0` in a container), `MW_AUTH_TOKEN`,
`MW_AUTH_JWKS` (a file path), `MW_AUTH_ISSUER`, `MW_AUTH_AUDIENCE`,
`MW_AUTH_ROLES_CLAIM`, `MW_AUTH_PROJECTS_CLAIM`, and `MW_PORT`. The entrypoint
additionally honours `MW_ALLOW_OPEN=yes` as the explicit opt-in to run open.

### Binding, and why exposure is deliberate

mw-server binds `127.0.0.1` by DEFAULT. That is the right default - a service
that listens on every interface the moment it starts is a service that gets
exposed by accident - but it means a container is unreachable until it says
otherwise. Both the compose file and the chart's ConfigMap therefore set
`MW_BIND=0.0.0.0` explicitly.

The combination that leaks a system of record is **open mode on a reachable
interface**: with no credential configured, every request is an anonymous admin,
so anyone who can route to the port is that admin. The server REFUSES TO START
in that combination - a non-loopback bind with `AuthConfig::Open` and no
`MW_ALLOW_OPEN=yes` is an error naming both ways to fix it. The container
entrypoint applies the same rule one layer earlier, so a misconfigured
deployment fails at `docker run` rather than at the first unauthorized request.

To run a deliberately open pilot:

    MW_BIND=0.0.0.0 MW_ALLOW_OPEN=yes mw-server


## Docker Compose (a trial on one machine)

    export POSTGRES_PASSWORD=...   # a real password, not the literal
    export MW_AUTH_TOKEN=...       # or: export MW_ALLOW_OPEN=yes  (laptop pilot)
    docker compose -f deploy/docker-compose.yml up --build

- Brings up `postgres:16` and `mw-server`, with a named volume for the
  database and one for the server's `/data`.
- The server refuses to start until `MW_AUTH_TOKEN` (or `MW_ALLOW_OPEN=yes`)
  is present, by design.
- The password and token are referenced via environment interpolation, never
  inlined.

## Helm chart (a real install)

    kubectl create secret generic modelwrite-secrets \
      --from-literal=auth-token=... \
      --from-file=jwks.json=... \           # or the bearer token, one of the two
      --from-literal=database-url=postgres://...

    helm install modelwrite deploy/helm/modelwrite \
      --set auth.tokenSecret=modelwrite-secrets \
      --set database.urlSecret=modelwrite-secrets

- Secrets are referenced by name (`auth.tokenSecret`, `auth.jwksSecret`,
  `database.urlSecret`) and never inlined in `values.yaml`; see
  `templates/secret.yaml.example` for the exact keys.
- Liveness and readiness probes both hit `GET /health`, which reports the auth
  mode, so an accidentally-open deployment is visible to monitoring.
- Resource requests and limits are set in `values.yaml`.
- Leave `database.urlSecret` empty to run on SQLite at `/data/modelwrite.db`.

## Air-gapped install

An air-gapped site has no pull access to a registry and may have no PostgreSQL.
Install the already-built image and its identity material:

1. **Transfer the image** — `docker save` it on a connected machine, carry the
   tar, and `docker load` it on the isolated host (or use a private registry on
   the isolated network).
2. **Mount the JWKS** — place the JWKS file where the container can read it and
   point `MW_AUTH_JWKS` at its path (the chart mounts a `jwksSecret` at
   `/jwks`). This is the air-gapped identity path: signed JWTs, verified
   locally, no network identity provider.
3. **Use a local database or none** — set `MW_DATABASE_URL` to a local
   PostgreSQL, or omit it and let the service run on SQLite at `/data/modelwrite.db`.

The gate evidence (`MW_EVIDENCE_DIR`) is written to `/data/evidence` and must
be on a volume you back up.

## Validation

Local (PyYAML 6.0.3): `docker-compose.yml`, `Chart.yaml`, `values.yaml` and
`templates/secret.yaml.example` all parse as YAML. CI goes further: the `deploy`
job parses those same files, runs `helm lint` on the chart (which renders the
Go-template files and checks them for real), and builds the image with
`docker build`. The Kubernetes API versions used (`apps/v1`, `v1`) are stable
and long-supported.
