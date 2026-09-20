# idc-1 trial deployment

Status: DONE — modelwrite trial running on idc-1 at https://trial.modelwrite.org,
seeded, verified from outside, with an hourly sandbox reset. The trial is
intentionally OPEN (no auth) so a browser can use it; see the Auth section.

## Host and service

- Host: idc-1 (Tailscale 100.86.3.50)
- Unit: `modelwrite.service` (enabled; Restart=on-failure)
- Listen: `127.0.0.1:3102` (loopback only; the 80xx range belongs to the live model fleet)
- Public hostname: `trial.modelwrite.org` -> hatch tunnel -> `http://localhost:3102`
- Data: `/opt/modelwrite/data` (SQLite `modelwrite.db` + `evidence/`), owned 65532:65532,
  bind-mounted to `/data` in the container.
- Env file: `/opt/modelwrite/deploy/modelwrite.env` (mode 600) — `MW_AUTH_TOKEN=` (blanked),
  `MW_ALLOW_OPEN=yes`, and `MW_BIND=0.0.0.0` (in-container bind; loopback is enforced by
  `-p 127.0.0.1:3102:8080`). The trial is intentionally open; the old token is recorded in
  the deployment report only, never in this repo.

## Source pinned

- Repository: `https://github.com/modelwrite/modelwrite.git` (public)
- Commit built from: `d66261e8483a98f64322aab1236d7afe5ea41519`
  (`fix(binding-xmi): name idref-only unknown elements by their reference`)
- Image: `modelwrite/modelwrite:idc-1-trial` (id `7f429a4147a2`, ~149 MB), built on the
  host from the public repo — no registry pull, no registry credentials.

> **Why not the v0.2.0 tag.** The tag predates the S2 crossModelEdges feature
> (`engine/okf/src/types.rs`, `server/src/composition.rs`). The trial's variant-impact
> comparison — CS-4 `satisfiedBy` `collector` (floor-robot) vs `gripper-arm` (microduck)
> — requires it, so the build is pinned to the exact S2 commit.

## Build (CPU-bounded)

```sh
cd /opt/modelwrite/src
docker build --cpuset-cpus=0-3 -f deploy/Dockerfile -t modelwrite/modelwrite:idc-1-trial .
```

`--cpuset-cpus=0-3` bounds the cargo build to 4 CPUs. idc-1 runs the live vLLM fleet
(two RTX 3090s at full utilisation); an unbounded build would starve it.

## Auth (intentionally OPEN)

The trial is OPEN by decision (2026-09-21): `MW_AUTH_TOKEN` is blanked and
`MW_ALLOW_OPEN=yes`, so every request is accepted as an anonymous admin and the UI works
in a plain browser with no header. This is deliberate; it is bounded by the hourly reset
below, which is now load-bearing.

- To re-enable auth: restore the `MW_AUTH_TOKEN` line and remove `MW_ALLOW_OPEN=yes` in
  `/opt/modelwrite/deploy/modelwrite.env`, then `sudo systemctl restart modelwrite`.
  The token value is recorded in the deployment report (`.superpowers/sdd/...`), never here.
- While static auth is on, `/health` and `/version` stay public; identity is
  `{ subject: "admin", roles: ["admin"] }`, and a configured-mode commit must name its own
  verified subject as `author` (the seed uses `author: "admin"`; a non-matching name is
  refused with 403).

## Seeded content

Six projects — `coffee-machine`, `sandwich-toaster`, `purchasing-terminal`,
`floor-robot`, `microduck`, `cafe-stand`. `cafe-stand` has two branches:

- `main`: floor-care reference to `floor-robot`, `crossModelEdges`
  `[{"from":"req-floor-clear","relation":"satisfiedBy","to":"collector"}]`
- `with-microduck`: floor-care re-pinned to `microduck` (bounds
  `gripper-arm`, `gripper-camera`, `microduck`), `crossModelEdges`
  `[{"from":"req-floor-clear","relation":"satisfiedBy","to":"gripper-arm"}]`

All four platform references resolve at their pinned revisions.

## Ingress (trial.modelwrite.org moved to idc-1)

The hostname was moved from a workstation cloudflared tunnel to idc-1's hatch tunnel:

- `/etc/cloudflared/hatch-config.yml` gained (validated with
  `cloudflared tunnel ingress validate` before reload):
  ```yaml
  - hostname: trial.modelwrite.org
    service: http://localhost:3102
  ```
- DNS: the `trial.modelwrite.org` CNAME now points at the hatch tunnel's
  `895153ce-0e3a-44dd-ae05-d2546302cdcb.cfargotunnel.com` (proxied).
- The workstation tunnel that used to serve it was stopped and its
  `~/.cloudflared/modelwrite-trial.yml` no longer claims the hostname.

Verified from the workstation: `https://trial.modelwrite.org/health` returns
`{"authMode":"open","status":"ok"}` and `/projects` returns the six models with no token,
proving it is idc-1 (the local 8080 trial was left unchanged).

## Hourly reset (sandbox)

The trial is a sandbox: visitors may create, edit or wreck anything, and every hour it
returns to the seeded baseline. (The reset was tightened from nightly 18:00 UTC to hourly
when the trial was opened, so a stranger's mess lasts minutes, not a day.)

- Timer: `modelwrite-trial-reset.timer`, `OnCalendar=hourly`, `Persistent=true`. The reset
  is CPU-trivial (no build) and only briefly restarts modelwrite; it never touches the
  live model fleet.
- Reset: `/opt/modelwrite/deploy/reset-trial.sh`, run by
  `modelwrite-trial-reset.service` (Type=oneshot). It:
  0. runs a change-detection guard: compares the live SQLite DB against the seed (per
     project: existence, the branch list, and every branch's tip hash). If they match
     exactly it logs "no changes since the last reset, skipping" and exits 0 WITHOUT
     stopping the service, taking a backup, or rebuilding anything. If the state is
     unreadable (missing manifest, unreadable DB, query error) the guard fails CLOSED and
     the full reset runs;
  1. builds a FRESH database off to the side by replaying `/opt/modelwrite/seed/manifest.json`
     into a throwaway seed container (`seed-from-manifest.py`), verifying every reproduced
     commit hash against the captured `expectedHash` - the live service keeps serving;
  2. verifies the six models are present and that cafe-stand's four references resolve;
  3. stops `modelwrite.service` only at the very end, backs up the outgoing DB to
     `/opt/modelwrite/backups/<date>T<time>.db` (keeps the last 48), swaps the fresh DB in,
     restarts, and logs. On any failure the service is restarted on the last good database
     (never left down).
- Backups are timestamped (date+time), so every hourly reset is a distinct file. A skip
  takes NO backup (the outgoing DB is just the seed) and does NOT restart the service, so a
  quiet hour costs nothing.

## Seed (canonical + reproducible)

- Canonical capture: `/opt/modelwrite/seed/manifest.json` + `documents/` (exact stored
  documents + replayable operations with expected hashes). Regenerate with
  `capture-seed.py`; replay with `seed-from-manifest.py`.
- Reproducible from this repository: `docs/deploy/seed-trial.mjs` reads
  `e2e/models/*.json` + the coffee-machine corpus straight from a checkout and rebuilds
  the same six models with identical commit hashes (verified). The same logic also runs
  on the host as `/opt/modelwrite/seed/seed.mjs` (with `models/` copies) - see
  `/opt/modelwrite/seed/README.md`.

## Operate

```sh
sudo journalctl -u modelwrite -f                 # logs
sudo systemctl start modelwrite-trial-reset.service   # reset by hand (skips if unchanged)
sudo env FORCE=1 /opt/modelwrite/deploy/reset-trial.sh   # force a reset, ignoring the guard
sudo systemctl restart modelwrite                # restart
```

Update: `git -C /opt/modelwrite/src checkout <commit>`, rebuild with the CPU-bounded
`docker build` above, then `sudo systemctl restart modelwrite`.

## Per-user trials (later)

Not built. The shape is parameterised so a second instance can be added without a
redesign: copy `modelwrite.service`, the reset units and `reset-trial.sh` with a
different `PORT`, `DATA_DIR`, seed-container port and `SERVICE` name.

## Open mode note

The trial is intentionally OPEN (no token): a browser click on
https://trial.modelwrite.org renders the workbench directly. This is by decision, and it
is bounded by the hourly reset above, which is now load-bearing (the reset and its
backup of the outgoing DB return the trial to the six seeded models).
