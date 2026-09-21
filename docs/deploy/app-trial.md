# app.modelwrite.org — registered trial deployment

Status: DONE — the registered-trial tier is live at https://app.modelwrite.org and verified
end to end (registration → console login code → session → empty workspace → project create →
cross-trial isolation). The login-code mailer is CONSOLE-ONLY until a real SMTP credential
exists; see "Email (console-only, by design)".

## Host and service

- Host: idc-1 (Tailscale 100.86.3.50)
- Unit: `modelwrite-app.service` (enabled; Restart=on-failure) — a second instance of the
  same `mw-server` binary as `modelwrite.service`, in REGISTERED mode.
- Listen: `127.0.0.1:3103` (loopback only; the 80xx range belongs to the live model fleet).
- Public hostname: `app.modelwrite.org` → hatch tunnel → `http://localhost:3103`.
- Container: `--name modelwrite-app`, image `modelwrite/modelwrite:idc-1-app`, port
  `-p 127.0.0.1:3103:8080`, `--read-only --tmpfs /tmp --security-opt no-new-privileges:true`
  — the same runtime posture as `modelwrite.service`.
- Data: `/opt/modelwrite/trials` (owned 65532:65532), bind-mounted to
  `/opt/modelwrite/trials` in the container. Holds `identity.db` (accounts, codes, sessions,
  trial rows) and `trials/<trial-id>.db` (one SQLite database per trial).
- Env file: `/opt/modelwrite/deploy/modelwrite-app.env` (mode 600).

## Env (the exact registered-mode variables)

```
MW_REGISTERED=yes
MW_TRIAL_DATA_ROOT=/opt/modelwrite/trials
MW_BIND=0.0.0.0
MW_ALLOW_OPEN=yes
MW_MAILER=console
```

- `MW_REGISTERED=yes` selects the registered tier (`server::run_registered`): identity
  store, per-trial databases, sessions, the rolling lifecycle.
- `MW_TRIAL_DATA_ROOT` names the data root (default `registered-data`); this deployment
  pins it to `/opt/modelwrite/trials`.
- `MW_BIND=0.0.0.0` is the in-container bind; host loopback is enforced by the unit's
  `-p 127.0.0.1:3103:8080`.
- `MW_ALLOW_OPEN=yes` is required even though registered mode has no `MW_AUTH_TOKEN`:
  registered mode authenticates with emailed one-time codes (session cookies), so both
  `MW_AUTH_TOKEN` and `MW_AUTH_JWKS` are absent and the image entrypoint's auth guard
  (`deploy/docker-entrypoint.sh`) refuses to start without this explicit opt-in. It is the
  same opt-in the showcase uses, not an override of a token.
- `MW_MAILER=console` is the default; it is written explicitly because it IS the current
  honest state (below).

## Email (console-only, by design)

The mailer is PLUGGABLE. Until an SMTP credential exists, `MW_MAILER=console` writes each
login code and its recipient to the operator log, so the whole flow is testable end to end
WITHOUT sending anything:

```
[mailer:console] to=<recipient> subject=Your Modelwrite login code body=Your Modelwrite login code is <code>. It expires in 10 minutes.
```

Read it with `sudo journalctl -u modelwrite-app`. A real visitor cannot yet receive a login
code; that is one SMTP credential away (see "Blocker"). The `Mailer` trait in
`server/src/trial/mailer.rs` is the seam.

## DNS + ingress

- Ingress: `/etc/cloudflared/hatch-config.yml` gained (validated with
  `cloudflared tunnel --config /etc/cloudflared/hatch-config.yml ingress validate`, then
  `sudo systemctl restart cloudflared-hatch` ONLY):

  ```yaml
  - hostname: app.modelwrite.org
    service: http://localhost:3103
  ```

- DNS: `app.modelwrite.org` is a proxied CNAME to the hatch tunnel
  `895153ce-0e3a-44dd-ae05-d2546302cdcb.cfargotunnel.com`, the same shape as
  `trial.modelwrite.org`. Cloudflare record id `1f643ec46f0e582cb91e416a7114ee47`.

## Verification (from outside, the workstation)

- `curl https://app.modelwrite.org/health` → `{"authMode":"session","status":"ok"}`.
- `curl https://app.modelwrite.org/` → 303 → `/register` (the no-JS registration form).
- Registration, code redemption, project creation and cross-trial isolation are verified; the
  verbatim run is in `.superpowers/sdd/app-trial-deploy-report.md`.

## Operate

```sh
sudo journalctl -u modelwrite-app -f              # logs (incl. console login codes)
sudo systemctl restart modelwrite-app             # restart
```

Update: `git -C /opt/modelwrite/src checkout <commit>`, rebuild the image CPU-bounded
(`docker build --cpuset-cpus=0-3 -f deploy/Dockerfile -t modelwrite/modelwrite:idc-1-app .`),
then `sudo systemctl restart modelwrite-app`.

## Blocker

Real transactional email delivery is blocked on an SMTP credential. Implement an SMTP
transport of the `Mailer` trait (`server/src/trial/mailer.rs`) behind `MW_MAILER=smtp`
plus a credential; nothing above the transport changes.
