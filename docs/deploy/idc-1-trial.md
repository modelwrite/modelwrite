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
- Env file: `/opt/modelwrite/deploy/modelwrite.env` (mode 600) — `MW_AUTH_TOKEN=` (blanked:
  the line is present but empty), `MW_ALLOW_OPEN=yes`, and `MW_BIND=0.0.0.0` (in-container
  bind; host loopback is enforced by `-p 127.0.0.1:3102:8080`). The trial is intentionally
  open. The old token value is preserved in a comment line in that same env file and in
  `.superpowers/sdd/idc-1-trial-deploy-2-report.md`, never in this repo.

## Source pinned

- Repository: `https://github.com/modelwrite/modelwrite.git` (public)
- Commit built from: `55f653ceaf2222c7314d818f2d72bbc27330bf72`
  (`docs: STPA completeness is in progress, not deployed` on top of
  `0b369b5 STPA/STAMP completeness check (T1-T3)`)
- Image: `modelwrite/modelwrite:idc-1-trial` (id `d767a42f08f7`, ~149 MB), built on the
  host from the public repo — no registry pull, no registry credentials.
- Previous (revert): image `50c8623ed6ac`, commit `47efbf1035a8250e227c9cb2d703a14c1dddf730`.

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

The trial is OPEN by decision (2026-09-21): `MW_AUTH_TOKEN` is blanked (the line is
present but its value is empty), so the server runs in open mode and accepts every request
as an anonymous admin — the UI works in a plain browser with no header. The server binds
`MW_BIND=0.0.0.0` (non-loopback) so it is reachable inside the container, and in open mode a
non-loopback bind is refused unless `MW_ALLOW_OPEN=yes` is set (both the container entrypoint
and `server::resolve_bind` enforce this). `MW_ALLOW_OPEN=yes` is therefore the explicit
opt-in that permits the open bind, not an override of the token. This is deliberate; it is
bounded by the hourly reset below, which is now load-bearing.

- To re-enable auth, make this one-line change in `/opt/modelwrite/deploy/modelwrite.env` —
  set the token value back into the blank line (the value is preserved in the
  `#   MW_AUTH_TOKEN=…` comment line directly above it, and in
  `.superpowers/sdd/idc-1-trial-deploy-2-report.md`) and delete the `MW_ALLOW_OPEN=yes`
  line, then `sudo systemctl restart modelwrite`:
  `MW_AUTH_TOKEN=<token value>`. A non-empty `MW_AUTH_TOKEN` switches the server from open
  to static auth (the `MW_ALLOW_OPEN` gate only applies while auth is open), so setting the
  token is the change that actually re-enables auth; deleting `MW_ALLOW_OPEN=yes` is the
  clean-state complement.
- Do NOT delete the blank `MW_AUTH_TOKEN=` line: `deploy/capture-seed.py` reads it and
  raises `MW_AUTH_TOKEN not found` if the line is absent. The line stays, blank, so the
  capture and reset scripts keep working in open mode.
- While static auth is on, `/health` and `/version` stay public; identity is
  `{ subject: "admin", roles: ["admin"] }`, and a configured-mode commit must name its own
  verified subject as `author` (the seed uses `author: "admin"`; a non-matching name is
  refused with 403).

## Seeded content

Seven projects — `coffee-machine`, `sandwich-toaster`, `purchasing-terminal`,
`floor-robot`, `microduck`, `cafe-stand`, and `fire-suppression` (the defective
STPA example). `cafe-stand` has two branches:

- `main`: floor-care reference to `floor-robot`, `crossModelEdges`
  `[{"from":"req-floor-clear","relation":"satisfiedBy","to":"collector"}]`
- `with-microduck`: floor-care re-pinned to `microduck` (bounds
  `gripper-arm`, `gripper-camera`, `microduck`), `crossModelEdges`
  `[{"from":"req-floor-clear","relation":"satisfiedBy","to":"gripper-arm"}]`

All four platform references resolve at their pinned revisions.

`fire-suppression` is the defective STPA/STAMP example
(`sample/stpa/fire-suppression-defective.json`): one controller, one controlled
process, one control action, no feedback — so the STPA completeness screen
(`/ui/projects/fire-suppression/stpa`) reports five findings (1 unanalysed control
action, 1 control loop with no feedback, 1 hazard with no constraint, 1 constraint
reaching no element, 1 UCA with no loss scenario).

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
`{"authMode":"open","status":"ok"}` and `/projects` returns the seven models with no token,
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
  2. verifies the seven models are present and that cafe-stand's four references resolve;
  3. stops `modelwrite.service` only at the very end, backs up the outgoing DB to
     `/opt/modelwrite/backups/<date>T<time>.db` (keeps the last 48), swaps the fresh DB in,
     restarts, and logs. On any failure the service is restarted on the last good database
     (never left down).
- Backups are timestamped (date+time), so every hourly reset is a distinct file. A skip
  takes NO backup (the outgoing DB is just the seed) and does NOT restart the service, so a
  quiet hour costs nothing.
- Seed container: `reset-trial.sh` runs the throwaway replay container on `127.0.0.1:3104`
  (`SEED_PORT=3104`) — a port kept clear of `modelwrite.service` (3102) and
  `modelwrite-app.service` (3103). 2026-09-21: this was 3103 until the registered tier took
  3103, which broke the hand-run reset with "Bind for 127.0.0.1:3103 failed: port is already
  allocated"; moved to 3104.

## Weekly forced reset (canary)

The hourly reset's change-detection guard skips the rebuild whenever the trial is
already clean - so a broken rebuild can go unnoticed: in the log, "no changes since
the last reset, skipping" and a failed rebuild looked identical. That happened
2026-09-21, when the seed container's port (then `SEED_PORT=3103`) collided with the
registered tier's `modelwrite-app.service`, and the breakage was only exposed by a
hand-run `FORCE=1`. A weekly FORCED run makes that class of masking impossible.

- Timer: `modelwrite-trial-verify.timer`, `OnCalendar=Mon *-*-* 03:30:00` UTC
  (weekly, `Persistent=true`). Monday 03:30 UTC is a quiet hour, offset from `:00`
  so it cannot collide with the hourly reset (which fires at `:00:00`).
- Service: `modelwrite-trial-verify.service` (Type=oneshot) runs the SAME
  `reset-trial.sh` with `Environment=FORCE=1`, so it always performs the full
  rebuild instead of skipping. A full rebuild once a week is cheap; every hour
  would defeat the guard entirely.
- Loud failure, not just logged: on failure the script already prints
  `RESET FAILED - restarting modelwrite.service on the last good database`, and the
  unit now exits non-zero (so `systemctl status` shows `failed`) and fires
  `OnFailure=algolotl-failure-notify@%n.service` - the host's existing
  business-events pager already used by the algolotl fleet. No new email or
  webhook dependency was introduced.
- Backups: the forced run takes one extra backup a week under the existing policy.
  Retention (48, ~2 days of hourly churn) is unaffected - one ~221 KB file a week
  is negligible, so there is no backup storm.

Check it:

```sh
systemctl list-timers modelwrite-trial-reset.timer modelwrite-trial-verify.timer
systemctl status modelwrite-trial-verify.service
```

## Seed (canonical + reproducible)

- Canonical capture: `/opt/modelwrite/seed/manifest.json` + `documents/` (exact stored
  documents + replayable operations with expected hashes). Regenerate with
  `capture-seed.py`; replay with `seed-from-manifest.py`.
- Reproducible from this repository: `docs/deploy/seed-trial.mjs` reads
  `e2e/models/*.json` + the coffee-machine corpus straight from a checkout and rebuilds
  the same seven models with identical commit hashes (verified). The same logic also runs
  on the host as `/opt/modelwrite/seed/seed.mjs` (with `models/` copies) - see
  `/opt/modelwrite/seed/README.md`.

## Operate

```sh
sudo journalctl -u modelwrite -f                 # logs
sudo systemctl start modelwrite-trial-reset.service   # reset by hand (skips if unchanged)
sudo env FORCE=1 /opt/modelwrite/deploy/reset-trial.sh   # force a reset, ignoring the guard
sudo systemctl start modelwrite-trial-verify.service   # weekly FORCE=1 canary (full rebuild; check for failed)
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
backup of the outgoing DB return the trial to the seven seeded models).
