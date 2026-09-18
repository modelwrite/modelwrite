# Slice 5 (tranche 1) - The agent loop: automating the work nobody has time for

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax.

**Goal:** the platform can be driven by an AI agent that does the tedious, high-volume work - reading a migration loss report, proposing resolutions, drafting requirement text - with every action going through the same permissions, locks, audit log and gate as a human's, and with a human approving anything that changes a model.

**Why this slice exists:** the mandate is explicit - there are not enough engineers who can use MBSE tools, so the tooling has to be usable by agents as well as people. Slice 1 shipped an MCP server and a published agent contract; Slices 2-4 gave that agent something worth doing. Slice 4's loss report is the first real job: a migration can produce thousands of lossy and unmappable entries, and no human team has time to read them.

**Architecture:** an agent is a CLIENT, never a special path. It authenticates with the same tokens, takes the same locks, writes the same audit entries, and is refused by the same gate. The engine gains an `agent` crate that turns a task description into a PLAN and a set of PROPOSALS, and a REVIEW artifact a human reads before anything is committed. Nothing in this slice lets an agent commit a model change on its own.

**Tech Stack:** Rust, the existing crates, and one new workspace member. No LLM call is made from inside these crates: the agent's reasoning is supplied as a trait, so tests run against a deterministic scripted reasoner and the crate has no network dependency.

## Global Constraints

- Rust stable; the engine crates stay at rust-version 1.75; the server is at 1.88.
- Every source file begins with: // SPDX-License-Identifier: AGPL-3.0-or-later
- Cargo is not on PATH in fresh shells: begin every shell command sequence with $env:Path = "$env:USERPROFILE\\.cargo\\bin;" + $env:Path
- Use --no-fail-fast when running a test suite.
- Stage and commit only the paths a task owns. Never amend or rewrite a commit.
- **AN AGENT IS A CLIENT, NOT A PRIVILEGED PATH.** Every agent action goes through the same handlers, permissions, locks, audit entries and gate as a human's. There is no "agent mode" that skips a check.
- **NO MODEL CHANGE IS COMMITTED BY AN AGENT ALONE.** The agent produces proposals; a human accepts them; the acceptance is recorded. The design says so, and this slice must not quietly relax it.
- **NO NETWORK IN ANY TEST.** The reasoning is a trait; tests use a scripted implementation.
- **ATTRIBUTION IS NOT OPTIONAL.** Every agent action is audited with the agent's identity and the mechanism, and the record says it was an agent, so a reader a year later can tell who decided.

---

## Task 1: The proposal contract and the scripted reasoner

**Files:** create engine/agent/src/lib.rs (the trait, the proposal types, the review artifact), engine/agent/src/report.rs; modify Cargo.toml; test engine/agent/tests/proposals.rs.

**Interfaces:**
- `pub trait Reasoner { fn propose(&self, task: &AgentTask) -> Result<Vec<Proposal>, AgentError>; }` where `AgentTask` carries the goal, the material (a loss report, a diff, a document excerpt) and the constraints.
- `pub struct Proposal { pub subject: String, pub action: ProposedAction, pub rationale: String, pub confidence: Confidence }` with `ProposedAction::{AcceptLoss, Reject, EditElement, DraftText, NoAction}` and `Confidence::{High, Medium, Low}`.
- `pub struct ReviewArtifact { pub task: AgentTask, pub proposals: Vec<Proposal>, pub agent: String, pub rationale_summary: String }` with `blocking()` and `render_text()` - the thing a human reads.
- `pub struct ScriptedReasoner` for tests.

**Acceptance:** a scripted reasoner's proposals render as a review artifact a human can read; a proposal with LOW confidence is marked for attention rather than buried; the artifact states which proposals CHANGE a model and which do not.

---

## Task 2: The loss-report resolver

**Files:** create engine/agent/src/losses.rs; test engine/agent/tests/losses.rs.

**Interfaces:** `pub fn propose_loss_resolutions(report: &binding::LossReport, reasoner: &dyn Reasoner) -> Result<ReviewArtifact, AgentError>` which turns a binding's loss report into proposals, one per blocking entry, and NEVER proposes accepting a loss it cannot explain.

**Rulings:** an `Unmappable` loss is never proposed for acceptance by the agent alone (there is nothing to map it to); a `Lossy` loss is proposed with the reason taken from the report's own note, not invented; and the artifact lists what the agent does NOT know, because a review that hides its gaps is the failure this whole platform is built against.

**Acceptance:** the coffee-grinder fixture's six named losses produce six proposals; an Unmappable entry produces a Reject-with-reason or NoAction rather than an acceptance; the artifact names the entries the reasoner had nothing to say about.

---

## Task 3: The agent identity and the audit trail

**Files:** modify server/src/auth.rs (an agent mechanism), server/src/audit.rs, server/src/api.rs if a route is needed for agent actions; test server/tests/agent_audit.rs.

**Interfaces:** the auth mechanism gains `agent` alongside open/static/jwt, so an action taken with an agent token is recorded as having been taken by an agent; the audit entry carries the agent's subject AND the human or service that authorised it where one exists.

**Acceptance:** an action taken with an agent token is audited with mechanism `agent`; a reader can tell an agent action from a human one from the audit log alone; an agent cannot take an action its permissions do not allow, proven by a denial test.

---

## Task 4: The agent's HTTP contract, documented and tested against the real API

**Files:** modify docs/agents/mcp-agent.md and docs/agents/mcp-tools.json; test server/tests/agent_contract.rs.

**Interfaces:** the published agent contract (written in Slice 1) is brought up to date with everything Slices 2-4 added - import, the loss report, gate runs, locks, the audit log - and a test asserts that every tool the contract names EXISTS as a route, so the contract cannot drift from the service.

**Acceptance:** every endpoint named in the agent contract resolves; a tool that was renamed or removed fails the test rather than disappointing an agent later.

---

## Completion criteria

- [ ] cargo test --workspace --no-fail-fast passes; fmt and clippy clean.
- [ ] A loss report of thousands of entries can be turned into a review artifact a human can act on.
- [ ] No path exists by which an agent commits a model change alone.
- [ ] An agent action is distinguishable from a human one in the audit log.
- [ ] The agent contract matches the service, enforced by a test.

## Later tranches of this slice

- The live reasoner behind the trait, as a SEPARATE process with its own credentials, so the platform never embeds a model provider.
- The review UI: proposals rendered in the workbench with accept/reject, which is where a human actually decides.
- Evaluations: a corpus of tasks with expected proposals, so a change to a prompt or a model can be measured rather than felt.
