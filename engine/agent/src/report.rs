// SPDX-License-Identifier: AGPL-3.0-or-later
//! Renders a ReviewArtifact to the plain text a human reads.
//!
//! The text is the contract: it must be readable with no knowledge of the
//! types, it must separate the proposals that change a model from the ones that
//! do not, it must mark a low-confidence proposal instead of burying it, and it
//! must list the entries the agent had nothing to say about rather than hiding
//! them. Nothing here can panic; it is pure formatting over borrowed data.

use crate::{Confidence, Proposal, ProposalCheck, ProposedAction, ReviewArtifact};

/// The section headings are kept as constants so a change to the rendered
/// vocabulary is a deliberate edit in one place rather than a typo that
/// silently splits the sections.
const HEADER: &str = "MODELWRITE AGENT REVIEW";

const CHANGES_MODEL: &str = "PROPOSALS THAT CHANGE THE MODEL";
const OBSERVATIONS: &str = "PROPOSALS THAT DO NOT CHANGE THE MODEL";
const ATTENTION: &str = " *** NEEDS ATTENTION ***";
const UNKNOWN: &str = "WHAT THE AGENT DOES NOT KNOW";
const GATE_CHECK: &str = "GATE CHECK OF THE CANDIDATE";

pub(crate) fn render(artifact: &ReviewArtifact) -> String {
    let mut out = String::new();

    out.push_str(HEADER);
    out.push_str("\n\n");
    // Attribution is not optional: the very first thing a reader sees is that an
    // automated agent produced this, and that nothing here has changed a model.
    out.push_str("This artifact was produced by an automated agent. Nothing here has changed\n");
    out.push_str(
        "a model; every entry below is a proposal for a human to read, accept or reject.\n",
    );
    out.push('\n');
    out.push_str(&format!("Agent: {}\n", artifact.agent));
    out.push_str(&format!("Goal: {}\n", artifact.task.goal));

    out.push_str("\nSummary\n-------\n");
    out.push_str(&artifact.rationale_summary);
    out.push('\n');

    out.push_str("\nMaterial\n--------\n");
    out.push_str(&artifact.task.material.describe());
    out.push('\n');

    out.push_str("\nConstraints\n-----------\n");
    if artifact.task.constraints.is_empty() {
        out.push_str("(none)\n");
    } else {
        for c in &artifact.task.constraints {
            out.push_str(&format!("- {}\n", c));
        }
    }

    let changing = artifact.blocking();
    let attention = artifact
        .proposals
        .iter()
        .filter(|p| p.needs_attention())
        .count();

    out.push_str(&format!(
        "\nProposals: {} total - {} change the model, {} require attention\n",
        artifact.proposals.len(),
        changing.len(),
        attention
    ));

    // The gaps are listed as their own section, before the proposals: what the
    // agent does not know matters as much as what it proposes, and hiding it
    // would be the failure this platform is built against.
    if !artifact.gaps.is_empty() {
        out.push('\n');
        out.push_str(UNKNOWN);
        out.push('\n');
        out.push_str(&"-".repeat(UNKNOWN.len()));
        out.push('\n');
        for gap in &artifact.gaps {
            out.push_str(&format!("- {}\n", gap));
        }
    }

    // The gate check of the candidate is its own section, so a human reads, before
    // accepting, that the result would leave an isolated node or a coverage regression.
    if let Some(check) = &artifact.check {
        render_check(&mut out, check);
    }

    out.push('\n');
    out.push_str(CHANGES_MODEL);
    out.push('\n');
    out.push_str(&"-".repeat(CHANGES_MODEL.len()));
    out.push('\n');
    render_proposals(&mut out, &artifact.proposals, true);

    out.push('\n');
    out.push_str(OBSERVATIONS);
    out.push('\n');
    out.push_str(&"-".repeat(OBSERVATIONS.len()));
    out.push('\n');
    render_proposals(&mut out, &artifact.proposals, false);

    out
}

/// Render the gate check of the candidate: the verdict, any validation errors, the isolated
/// nodes, the component count and the coverage delta. Every line is absent when it has nothing
/// to say, so a clean candidate reads as a short, plain pass.
fn render_check(out: &mut String, check: &ProposalCheck) {
    out.push('\n');
    out.push_str(GATE_CHECK);
    out.push('\n');
    out.push_str(&"-".repeat(GATE_CHECK.len()));
    out.push('\n');
    out.push_str(&format!(
        "passed: {}\n",
        if check.passed { "yes" } else { "NO" }
    ));
    if check.validation_errors.is_empty() {
        out.push_str("validation errors: (none)\n");
    } else {
        for e in &check.validation_errors {
            out.push_str(&format!("- validation error: {}\n", e));
        }
    }
    if check.isolated_nodes.is_empty() {
        out.push_str("isolated nodes: (none)\n");
    } else {
        for id in &check.isolated_nodes {
            out.push_str(&format!("- isolated node: {}\n", id));
        }
    }
    out.push_str(&format!(
        "connected components: {}\n",
        check.component_count
    ));
    let new_uncovered = check
        .uncovered_requirements
        .iter()
        .filter(|id| !check.prior_uncovered_requirements.contains(id))
        .count();
    out.push_str(&format!(
        "uncovered requirements: {} ({} new since current model)\n",
        check.uncovered_requirements.len(),
        new_uncovered
    ));
}

/// Render one proposal, preserving the order the reasoner returned them in
/// within each section. The order is stable because the script is stable, and a
/// test can pin the exact text.
fn render_proposals(out: &mut String, proposals: &[Proposal], changing: bool) {
    let mut index = 0;
    for p in proposals {
        if p.changes_model() != changing {
            continue;
        }
        index += 1;
        let attention = if p.needs_attention() { ATTENTION } else { "" };
        out.push_str(&format!(
            "[{}] {}  (action: {}, confidence: {}{})\n    rationale: {}\n",
            index,
            p.subject,
            action_name(&p.action),
            confidence_name(p.confidence),
            attention,
            p.rationale
        ));
    }
    if index == 0 {
        out.push_str("(none)\n");
    }
}

/// The rendered name of an action, so the text reads as prose and the reader
/// never has to know the enum. These are the words the contract publishes.
fn action_name(action: &ProposedAction) -> &'static str {
    match action {
        ProposedAction::AcceptLoss => "accept loss",
        ProposedAction::Reject => "reject",
        ProposedAction::EditElement => "edit element",
        ProposedAction::DraftText => "draft text",
        ProposedAction::NoAction => "no action",
    }
}

fn confidence_name(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::High => "High",
        Confidence::Medium => "Medium",
        Confidence::Low => "Low",
    }
}
