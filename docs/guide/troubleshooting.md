# Troubleshooting

Short. Each entry is a failure you will actually see, what it means, and what to do.

## An edit is refused: "Held by another holder"

Meaning: another editor holds a lease on that element, and the commit would overwrite their
work. The page names the holder and the expiry. What to do: wait for the lease to expire, or
ask the holder to release it, then reload the form. Leases always expire, so a crashed client
cannot block an element forever.

## An import is refused: "Import refused: blocking losses"

Meaning: the migration is lossy, and not every blocking loss has been accepted. Nothing was
committed; the source artifact was still retained and content-addressed. What to do: read the
loss report, check every blocking loss by name, and submit again. A loss left unchecked
refuses the import again.

## A page is stale, or a link 404s

Meaning: the trial is open-mode and ephemeral, and it restarts. The six models are redeployed,
and their commit hashes change. What to do: reload from the project list rather than trusting
a bookmarked commit hash. On a configured deployment this should not happen; a 404 there means
the named project, commit, branch or import does not exist.

## A diagram will not fit

Meaning: the diagram is larger than the viewport, which is normal for a real model. What to
do: use the zoom controls in the diagram toolbar (the minus and plus buttons, and "Fit"), or
scroll the viewport. The Structure and Process views are toggled by links in the same toolbar.

## "Sign in required" (401)

Meaning: the workbench is protected by authentication and the request carried no token. What
to do: present your credentials (an Authorization: Bearer header) and reload.

## A refusal with no obvious cause (403)

Meaning: the authenticated caller lacks the required permission, or the project is out of
scope. What to do: check the role and project scope on the token. An agent token can only hold
viewer and reviewer, so a write route always refuses an agent with 403.

## A commit or paste is refused as invalid (422)

Meaning: the OKF document failed validation. What to do: read the validator's errors, which
are shown next to the form, and fix them. A requirement needs a non-empty reqId; ids must be
unique and non-empty; every edge endpoint must resolve to a node.
