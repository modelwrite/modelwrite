# OKF as the platform's document format - installs, domains, and where it stops

## The instinct, and why it is right

An OKF document is: **validated, diffed, content-addressed, committed, gate-checked, and audited.** Today only models are OKF. But everything an organisation configures - a domain's vocabulary, a site's entitlements, which symbol packs are licensed - needs **exactly those same properties**, and today has none of them: it lives in environment variables and a database row nobody can diff.

So: **an install is an OKF**, and so is a domain. That gives configuration a revision, an author, a diff and a record - which for a defence programme is not a nicety, it is the compliance requirement ("prove what configuration was in force on this date").

## The three levels, using the primitive we already have

The system-of-systems work already built a **typed, pinned reference** (project + revision + role). The same primitive composes the configuration hierarchy:

| Level | What the document declares | References |
|---|---|---|
| **Domain** | the vocabulary: kinds, stereotypes, allowed analyses, symbol mappings | nothing below it |
| **Install** | sites, nodes, which domains are in force, which symbol packs and bindings are licensed, retention and auth policy | **domains, pinned** |
| **Model** | blocks, requirements, the graph, the process | **subsystem models, pinned** (S0) |

Each level is content-addressed and versioned, so "which vocabulary was in force when this model was gated?" has an answer, and **two installs can be diffed**.

## Where it STOPS, and this boundary matters

**IDENTITY IS NOT A DOCUMENT.** Users, sessions and tokens are operational identity: they authenticate, they change minute to minute, and making a login a commit would be absurd. Authentication stays where it is (static token, JWTs, roles).

**But POLICY is a document.** Which roles exist, what each may do, which projects a role reaches - that belongs in the install OKF, because it is configuration that should be reviewed and diffed. The split is:

- **Who you are** - identity, not a document.
- **What the system is configured to be** - a document, versioned and reviewable.

That single line answers "per user?" properly: **no per-user documents; per-install policy documents.**

## What this buys

1. **Configuration control.** An install has provenance: authored, imported, or unknown - the same vocabulary the commit path already records.
2. **Diffable environments.** Test and production differ by a readable diff, not by folklore.
3. **Domain packs that are real artefacts.** A domain's vocabulary and symbol mappings become a versioned pack an organisation can own, review, and license - which is exactly what the symbology work needs (the three licence classes: bundled, operator-supplied, never-bundled).
4. **The gate applies to configuration too.** An install that references a domain which does not resolve is a NAMED failure, not a startup crash at 3am.

## Rulings

1. **CONFIGURATION THAT SHOULD BE REVIEWED IS A DOCUMENT.** Domains, installs, policy. *Cost if wrong:* a little ceremony. *Benefit:* the same proof machinery for configuration that models already have.
2. **IDENTITY IS NOT A DOCUMENT.** Users and sessions are not versioned artefacts.
3. **THE SAME REFERENCE PRIMITIVE COMPOSES ALL THREE LEVELS** - pinned, typed, resolvable, with named failures. No second composition mechanism.
4. **AN INSTALL MUST RESOLVE OR REFUSE TO START**, naming what did not resolve - the platform's own rule, applied to its own configuration.

## Status

Design only. Nothing here is built. It is recorded because it is the right shape and because the alternative - configuration as environment variables and undocumented rows - is what the platform currently has, and it cannot answer "what were we running in March?"
