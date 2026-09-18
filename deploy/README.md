# Deploying modelwrite

This directory turns the repository into software an organisation can install: a
container image, a Docker Compose trial stack, and a Helm chart for Kubernetes.

**WARNING — OPEN MODE. mw-server runs WITHOUT authentication when neither a token
nor a JWKS is configured, and in that mode every request is accepted as an
anonymous administrator. The container image refuses to start in that state
unless the operator sets `MW_ALLOW_OPEN=yes` explicitly. Never run anything
reachable by more than one person without `MW_AUTH_TOKEN` (a shared bearer
token) or `MW_AUTH_JWKS` (a JWKS file for signed JWTs).**

## What was and was not verified

- **Verified here:** every YAML file in this directory parses cleanly, the Helm
  templates are well-formed, and the Kubernetes API fields used are real for
  `apps/v1`/`v1` (see `deploy/README.md` "Validation" below).
- **NOT verified here:** the image was **never built** and the chart was **never
  installed**. This machine has no Docker and no Kubernetes cluster, so none of
  these files has been run. The Dockerfile's stages, paths and commands are
  consistent by inspection only; a build failure would surface the first time the
  image is actually built in CI or on an operator's machine.

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
(PostgreSQL URL) or `MW_DB` (SQLite path), `MW_EVIDENCE_DIR`,
`MW_AUTH_TOKEN`, `MW_AUTH_JWKS` (a file path), `MW_AUTH_ISSUER`,
`MW_AUTH_AUDIENCE`, `MW_AUTH_ROLES_CLAIM`, `MW_AUTH_PROJECTS_CLAIM`, and
`MW_PORT`. The entrypoint additionally honours `MW_ALLOW_OPEN=yes` as the
explicit opt-in to run open.

### Known limitation: the service binds to loopback

As of this tranche, mw-server binds to `127.0.0.1` and there is **no**
`MW_BIND`/`MW_HOST` variable to change it. Inside a container that means the
service is reachable only from its own container's network namespace, not through
the published port (Compose) or the Service (Kubernetes). The probes, the port
mapping and the Service here are the correct eventual configuration, but until a
bind-address variable is added to the server they will not carry real traffic.
This is a server-code change and is deliberately out of scope for this deployment
task.

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

Performed with Python's PyYAML (6.0.3): `docker-compose.yml`, `Chart.yaml`,
`values.yaml` and `templates/secret.yaml.example` all parse as YAML. The
Go-template files (`deployment.yaml`, `service.yaml`, `configmap.yaml`,
`pvc.yaml`, `_helpers.tpl`) were reviewed for well-formed output; they cannot
be parsed as YAML before Helm renders them, and Helm is not installed here. The
Kubernetes API versions used (`apps/v1`, `v1`) are stable and long-supported.
