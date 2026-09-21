# app.modelwrite.org — registered trial deployment

Status: DONE — the registered-trial tier is live at https://app.modelwrite.org and verified
end to end (registration → emailed login code → session → empty workspace → project create →
cross-trial isolation). The login code is emailed through Postmark (`MW_MAILER=smtp`); email
delivery is configured and proven on this deployment. See "Email (SMTP, proven)".

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
MW_MAILER=smtp
MW_MAIL_FROM=<provider-confirmed sender on the modelwrite.org domain>
MW_MAIL_SMTP_HOST=<the provider's SMTP submission host>
MW_MAIL_SMTP_PORT=<submission port; 587 is the default>
MW_MAIL_SMTP_USERNAME=<SMTP username / provider API token>
MW_MAIL_SMTP_PASSWORD=<SMTP password / provider API token>
```

The five `MW_MAIL_*` values live in the mode-600 env file; the credential values are
deliberately not reproduced here.

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
- `MW_MAILER=smtp` selects the real SMTP transport (below). `console` remains the
  compiled-in default, but this deployment does not use it.

## Email (SMTP, proven)

The mailer is PLUGGABLE and `MW_MAILER=smtp` selects a real SMTP transport
(`server/src/trial/mailer.rs`): it dials the provider over STARTTLS, authenticates, and
delivers the login code. Email delivery is CONFIGURED AND PROVEN on this deployment:

- preflight — STARTTLS reported `Verify return code: 0 (ok)`, the server advertised
  `AUTH PLAIN LOGIN`, and authentication returned `235 2.7.0 Authentication successful`;
- `POST /register` returned 200 and the transport logged
  `250 2.0.0 Ok: queued as 33C234056E1`;
- the provider recorded the message — `MessageID 7b77d796-25ce-4fd1-95ad-25cd9494f116`,
  `Status Sent`, subject "Your Modelwrite login code";
- the code was redeemed — `POST /login` → 303 to `/ui` with an `mw_session` cookie,
  `GET /projects` with that session returned an empty workspace, and the same request
  without the cookie returned 401.

The only operator-supplied input is the SMTP credential, and it is already in place here (the
variable names are in "Env" above). The transport is chosen by name from the environment:

- `MW_MAILER=console` (the compiled-in default) writes each code and its recipient to the
  operator log for an offline test; `MW_MAILER=file` appends to `MW_MAILER_FILE`. Neither
  is used on this deployment.
- `MW_MAILER=smtp` requires `MW_MAIL_SMTP_HOST`, `MW_MAIL_SMTP_USERNAME` and
  `MW_MAIL_SMTP_PASSWORD` (`MW_MAIL_SMTP_PORT` defaults to 587). If any is missing, the
  deployment gets a transport that FAILS every send with an error naming the missing variable
  — it does NOT fall back to the console, so a misconfigured deployment refuses registration
  loudly instead of looking like a delivered email. A build without the `smtp` feature fails
  the same way.

Read the log with `sudo journalctl -u modelwrite-app`.

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
sudo journalctl -u modelwrite-app -f              # logs (incl. mailer delivery)
sudo systemctl restart modelwrite-app             # restart
```

Update: `git -C /opt/modelwrite/src checkout <commit>`, rebuild the image CPU-bounded
(`docker build --cpuset-cpus=0-3 -f deploy/Dockerfile -t modelwrite/modelwrite:idc-1-app .`),
then `sudo systemctl restart modelwrite-app`.

## Blocker

None for email: the SMTP credential exists and delivery is proven (above). The remaining
honest limit is configuration, not capability — `MW_MAILER=smtp` with
`MW_MAIL_SMTP_HOST`, `MW_MAIL_SMTP_USERNAME` or `MW_MAIL_SMTP_PASSWORD` unset fails
the send and names the missing variable rather than logging the code.
