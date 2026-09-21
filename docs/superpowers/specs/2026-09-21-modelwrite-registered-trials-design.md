# Registered trials: the two-tier trial design

**Status:** design, approved framing, awaiting owner review of this spec.
**Date:** 2026-09-21
**Companion:** docs/superpowers/specs/2026-09-21-modelwrite-open-licensed-boundary.md (the open-versus-licensed boundary)

## 1. The framing

The trial is TWO things, and the website says so plainly:

- **The Showcase - "come and see what we can do."** Open, no registration, the six seeded models, hourly reset with the skip-guard, wreckable without consequence. That is what trial.modelwrite.org is today.
- **The Registered Trial - "load and analyse your own model."** Fourteen days, one person, their model. Registration, an isolated workspace, their own import, their own loss report, their own orphans and isolated groups, their own coverage - the answer to "what is my model worth?"

The distinction is the product: the showcase answers "what can this do?", the registered trial answers "what is MY model worth?"

## 2. Decisions, recorded from the owner

1. **Registration: email + a login code.** A form takes an email; a single-use code is sent; entering the code starts a session. No third-party identity dependency; the same machinery grows into enterprise SSO later.
2. **Extension: by ACTIVITY, not payment.** The trial is a ROLLING WINDOW: genuine use extends it; dormancy lets it expire. Nobody loses work the day the clock runs out. "Your trial ended, your report is still readable" is the right ending - and the honest one, because a trial that ended this way was one nobody was using.
3. **Payment extension: DEFERRED, not built now.** At this stage an active user is enough: the registered trial exists to prove the product, not to monetise it. The checkout design in section 7 stays in this spec as the later path, ready if the owner ever enables it - and the trial's audit trail already has a place for an extension record.
4. **The registered tier lives at app.modelwrite.org.** The showcase keeps trial.modelwrite.org. Two doors, clean separation, one product.

## 3. The tiers

| | Showcase | Registered trial |
|---|---|---|
| URL | trial.modelwrite.org | **app.modelwrite.org** |
| Registration | none | email + code |
| Workspace | shared, six seeded models | **one per trial, empty, their own model** |
| Lifetime | resets hourly, skip-guard | **14 d active + 7 d read-only + archive** |
| Extension | n/a | **payment** |
| Isolation | open | **per-trial database, enforced at every store access** |

## 4. Registration and identity

- A registration form takes an email; verification by a **single-use, 10-minute-expiring code** (not a link - a code the person types back). Codes are **hashed at rest**, never stored in the clear.
- Entering the code creates the trial account and a **session** (a bearer token with an expiry; sessions refresh on use within reason, expire after inactivity).
- **Rate limits from day one**, because a public signup that creates a database per signup is a resource-exhaustion vector: N codes per email per hour, M registrations per IP per day, and a **cap on the total number of live trials** the deployment allows, with the operator able to raise it.
- Passwords do not exist in this design. The code IS the credential. This keeps the attack surface small and avoids the entire password-reset machinery.

## 5. Isolation - the security boundary that matters

- **One SQLite database per trial**, created at registration, in a per-trial directory under the deployment's data root. A trial database is on the order of 1-2 MB; a hundred trials is a couple of hundred megabytes. Cheap, and cheap to back up or archive individually.
- **The trial identity is resolved from the session at every request, and the store path is derived from it.** The path is NEVER taken from the request. A user can name any project they like; they cannot reach another trial's database by construction, and there is a test that proves it: user A's session cannot read user B's project even with B's project id in hand.
- The parameterised deployment from the showcase work (port, database path, service name) is what makes a second tier cheap: the registered tier is the SAME server binary with a data root per trial, not a new service per trial.

## 6. The lifecycle

1. **Active:** full read/write on the person's own workspace. **Any authenticated activity - a read, a write, an import - refreshes the window to a full 14 days from that moment.** A trial that is genuinely used does not expire.
2. **Read-only:** after 14 days WITHOUT activity, reads keep working for 7 more days and writes are refused with a message naming the trial's end and how to start again. The user can still export, screenshot and decide.
3. **Frozen:** after 21 days without activity, the database is archived (copied to an archive root, checksum recorded), the live copy removed, and the account retained so the same email can return to a fresh trial.
4. **Extension:** activity is the extension. The audit trail records the last-activity refresh so the operator can see, plainly, which trials were used and which were not. (The deferred payment path, if ever enabled, would also write an extension record here - with the provider's reference, never card details.)

## 7. The payment bridge - DEFERRED, design kept for later

- NOT BUILT NOW. Activity extends the trial; payment does not enter this tranche.
- When the owner wants a paid path, the design is: a **checkout link from a payment provider** (Stripe recommended: hosted checkout, no card data ever touches our server). On the webhook confirming payment, the trial's window extends (default +30 days per payment; the amount and period are the owner's to set) and the audit trail records the provider reference.
- It would need owner-provided payment credentials - which is one of the reasons it is deferred rather than blocking this tranche.

## 8. Resource protection

- Caps on live trials (configurable), caps on code requests per email and per IP, and a **nightly reaper** that archives trials past day 21 and deletes archived trials older than a retention window (default 90 days, configurable).
- The showcase's hourly reset is UNTOUCHED by this work and keeps serving its purpose: demonstrating the product without consuming registered-trial resources.

## 9. What exists and what is new

**Exists:** the parameterised deployment pattern, the import path and loss reports, graph-gap detection (orphans, isolated groups), coverage, the gate, the analytics inventory work in flight.

**New:** the identity store (emails, hashed codes, sessions), the provisioning path (create a trial database on registration), the per-request trial resolution, the lifecycle enforcement (active/read-only/frozen), the extension record, the rate limits, the reaper, and the two-door website copy.

**Out of scope, stated:** enterprise SSO, row-level security by classification, the pull audit - that is the licensed tier, and it is the NEXT brief, not this one. The registered trial is single-user by design; it proves the isolation machinery without pretending to be multi-user governance.

## 10. Acceptance criteria

1. From a clean browser: register with an email, receive a code, enter it, land in an EMPTY workspace.
2. Import a model (XMI) and see its loss report, coverage, and the orphan/isolated-group analytic.
3. A second registration gets a DIFFERENT workspace; cross-trial access is refused by construction and proven by test.
4. Time-travelled: 13 days without activity -> writes refused with the end message, reads still work. Activity on day 13 -> the window refreshes to a full 14 days from that moment. 21 days without activity -> archived; the same email can start a fresh trial.
5. Rate limits demonstrably stop a burst of code requests.
6. The showcase at trial.modelwrite.org is byte-for-byte unchanged in behaviour.

## 11. Unknowns, stated

- **Email delivery:** the code must reach a person. This needs an email provider credential (a transactional mail service, or the host's relay). BLOCKED on owner-provided credentials - and now the ONLY credential gate, since payment is deferred.
- **The analytics layer:** the registered trial's headline value - trends and exports - lands with the analytics-interfaces work; until then the registered trial still shows loss reports, coverage and graph gaps, which are already built.

## 12. Order of work

1. Identity + sessions + provisioning + isolation (the security core), tested by the cross-trial refusal.
2. Lifecycle enforcement + reaper.
3. The two-door website copy and the registration pages.
4. The analytics interfaces behind the registered trial as they land.
5. Payment extension - only if and when the owner enables it.
