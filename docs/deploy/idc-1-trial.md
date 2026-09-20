# idc-1 trial deployment

Status: DONE — modelwrite trial running on idc-1, seeded and verified from outside.

## Host and service

- Host: idc-1 (Tailscale 100.86.3.50)
- Unit: `modelwrite.service` (enabled; Restart=on-failure)
- Listen: `127.0.0.1:3102` (loopback only; the 80xx range belongs to the live model fleet)
- Data: `/opt/modelwrite/data` (SQLite `modelwrite.db` + `evidence/`), bind-mounted to
  `/data` in the container, owned by uid 65532 so the non-root runtime can write it.
- Env file: `/opt/modelwrite/deploy/modelwrite.env` (mode 600) — `MW_AUTH_TOKEN` and
  `MW_BIND=0.0.0.0` (in-container bind; loopback is enforced by the `-p 127.0.0.1:3102:8080`
  port mapping). The token is recorded in the deployment report only and never in this repo.

## Source pinned

- Repository: `https://github.com/modelwrite/modelwrite.git` (public)
- Commit built from: `d66261e8483a98f64322aab1236d7afe5ea41519`
  (`fix(binding-xmi): name idref-only unknown elements by their reference`)
- Image: `modelwrite/modelwrite:idc-1-trial` (image id `7f429a4147a2`, ~149 MB), built on
  the host from the public repo — no registry pull, no registry credentials.

> **Why not the v0.2.0 tag.** The tag predates the S2 crossModelEdges feature
> (`engine/okf/src/types.rs`, `server/src/composition.rs`). The trial's variant-impact
> comparison — CS-4 `satisfiedBy` `collector` (floor-robot) vs `gripper-arm` (microduck)
> — requires that feature, so the build is pinned to the exact S2 commit instead.

## Build (CPU-bounded)

```sh
cd /opt/modelwrite/src
docker build --cpuset-cpus=0-3 -f deploy/Dockerfile -t modelwrite/modelwrite:idc-1-trial .
```

`--cpuset-cpus=0-3` bounds the cargo build to 4 CPUs. idc-1 runs the live vLLM model
fleet (two RTX 3090s at full utilisation); an unbounded `cargo build` would starve it.

## Auth

- `MW_AUTH_TOKEN` (static shared bearer token) gates every request — JSON API and `/ui`
  pages alike — with `401` unless `Authorization: Bearer <token>` is present. `/health`
  and `/version` stay public. Identity is `{ subject: "admin", roles: ["admin"] }`.
- A configured-mode commit must name its own verified subject as `author`, so the seed
  uses `author: "admin"` (a non-matching name is refused with 403).

## Seeded content

Six projects — `coffee-machine`, `sandwich-toaster`, `purchasing-terminal`,
`floor-robot`, `microduck`, `cafe-stand` — committed via the JSON API
(`POST /projects`, `POST /projects/:p/commits`). `cafe-stand` has two branches:

- `main`: floor-care reference to `floor-robot`, `crossModelEdges`
  `[{"from":"req-floor-clear","relation":"satisfiedBy","to":"collector"}]`
- `with-microduck`: floor-care re-pinned to `microduck` (bounds
  `gripper-arm`, `gripper-camera`, `microduck`), `crossModelEdges`
  `[{"from":"req-floor-clear","relation":"satisfiedBy","to":"gripper-arm"}]`

All four platform references resolve at their pinned revisions.

## Operate

```sh
sudo journalctl -u modelwrite -f          # logs
systemctl status modelwrite               # status
sudo systemctl restart modelwrite         # restart
```

To update: `git -C /opt/modelwrite/src checkout <commit>`, rebuild with the
CPU-bounded `docker build` above, then `sudo systemctl restart modelwrite`.

## Ingress (pending hostname decision)

The service is loopback-only; Cloudflare DNS and the tunnel ingress were intentionally left
unchanged. The single ingress line needed, once a hostname is chosen, is:

```yaml
- hostname: <modelwrite-trial-hostname>
  service: http://localhost:3102
```

added to `/etc/cloudflared/hatch-config.yml` (the `trial.podataka.com → localhost:3101`
analogue), then `sudo systemctl restart cloudflared-hatch.service`.

## Browser note

There is no login form: a human visitor must send `Authorization: Bearer <token>` as a
header (curl/CLI works; a browser needs a header-setting extension). A public hostname
would therefore need Cloudflare Access (or equivalent) to inject the header for visitors.
