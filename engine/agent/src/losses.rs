// SPDX-License-Identifier: AGPL-3.0-or-later
//! The loss-report resolver: turns a binding's loss report into the proposals a
//! human reads before any loss is accepted.
//!
//! A migration can produce thousands of lossy and unmappable entries, and no
//! human team has time to read them. This is the agent's first real job: read
//! the report, propose a resolution for every blocking entry, and never pretend
//! to a judgement it cannot make.
//!
//! Two boundaries shape everything here, and both are carried into the artifact:
//!
//! 1. The loss report is not ground truth. The engine measures only the binding's
//!    own OKF->XMI->OKF round trip; the native XMI->OKF read - the actual
//!    migration - is not independently measured. The report is the binding's
//!    self-account, so this resolver never presents its contents as verified.
//! 2. Loss entries are not uniquely identified by subject alone. A single XMI
//!    comment can produce a Lossy entry for its dropped id and an Unmappable
//!    entry for its dropped body. Keying on the subject string would collapse
//!    them into one decision, and an acceptance flowing through the same key
//!    would accept both when the human meant one. Every key here is the
//!    (subject, verdict) pair, encoded as the entry identity.

use std::collections::HashMap;

use binding::{LossReport, Mapping, MappingVerdict};

use crate::{
    AgentError, AgentTask, Confidence, Material, Proposal, ProposedAction, Reasoner, ReviewArtifact,
};

/// The agent identity recorded on the artifact this resolver produces.
const RESOLVER_AGENT: &str = "loss-report-resolver";

/// The measurement boundary drawn in the interoperability fix: the loss report
/// is the binding's own account, not an engine verification. The engine measures
/// only the binding's OKF->XMI->OKF round trip; the native XMI->OKF read - the
/// actual migration - is not independently measured. This resolver carries that
/// boundary into every artifact so the report is never mistaken for ground truth.
const MEASUREMENT_BOUNDARY: &str = "The loss report is not ground truth: the engine measures only the binding's own OKF->XMI->OKF round trip; the native XMI->OKF read is not independently measured.";

/// The reason recorded when a reasoner asks to accept an Unmappable entry, which
/// the agent must refuse: there is nothing to map it to.
const UNMAPPABLE_REJECT_REASON: &str =
    "there is nothing to map this entry to, so the agent cannot propose accepting it";

/// The single word a reader sees for a verdict, used inside an entry identity.
fn verdict_word(verdict: MappingVerdict) -> &'static str {
    match verdict {
        MappingVerdict::Exact => "exact",
        MappingVerdict::Lossy => "lossy",
        MappingVerdict::Unmappable => "unmappable",
    }
}

/// The identity a proposal uses to name one loss entry unambiguously.
///
/// Two different losses can share a [`Mapping::subject`]: a single XMI comment
/// drops its id as a Lossy entry and its body as an Unmappable entry. Keying a
/// resolution on the subject alone would collapse the two, and an acceptance
/// flowing through the same key would accept both when the human meant one. The
/// identity is the (subject, verdict) pair, so the two stay two decisions.
pub fn entry_identity(mapping: &Mapping) -> String {
    format!("{} [{}]", mapping.subject, verdict_word(mapping.verdict))
}

/// Turn a binding's loss report into a review artifact: one proposal per
/// blocking entry, never an acceptance the agent cannot explain.
///
/// The reasoner is consulted once with the whole report, and its proposals are
/// matched back to entries by [`entry_identity`]. Every blocking entry the
/// reasoner said nothing about is listed in the artifact's `gaps`, because a
/// review that hides its gaps is the failure this platform is built against.
///
/// The rulings enforced here, whatever the reasoner returns:
///
/// - An `Unmappable` loss is never proposed for acceptance. There is nothing to
///   map it to, so an `AcceptLoss` from the reasoner is rewritten to a `Reject`
///   that says so.
/// - A `Lossy` loss is proposed for acceptance only with the reason taken from
///   the report's own note, never one the agent invents.
/// - `EditElement` and `DraftText` change the model; resolving a loss is not
///   editing a model, so those actions are treated as "the reasoner said nothing
///   about resolving this loss".
pub fn propose_loss_resolutions(
    report: &LossReport,
    reasoner: &dyn Reasoner,
) -> Result<ReviewArtifact, AgentError> {
    let blocking = report.blocking();
    let total = blocking.len();

    let task = AgentTask {
        goal: format!(
            "resolve the blocking losses in the {} binding's report",
            report.binding.id
        ),
        material: Material::LossReport(report.clone()),
        constraints: vec![
            MEASUREMENT_BOUNDARY.to_string(),
            "Never propose accepting an Unmappable loss; there is nothing to map it to."
                .to_string(),
            "A Lossy loss may be accepted only with the reason the report itself gives."
                .to_string(),
            "Name each entry by its identity 'subject [verdict]' (for example 'uml:Comment c1 [lossy]'); two entries may share a subject but differ in verdict."
                .to_string(),
        ],
    };

    let raw = reasoner.propose(&task)?;

    // Match the reasoner's proposals back to entries by (subject, verdict),
    // encoded as the entry identity. Keying on the subject alone would collapse
    // a comment's Lossy id-drop with its Unmappable body-drop; the identity
    // carries the verdict, so the two stay two decisions. (The key is the
    // encoded string rather than a (String, MappingVerdict) tuple because
    // MappingVerdict is not Hash.)
    let mut by_identity: HashMap<String, Proposal> = HashMap::new();
    for proposal in raw {
        by_identity.insert(proposal.subject.clone(), proposal);
    }

    let mut proposals = Vec::with_capacity(blocking.len());
    let mut gaps = Vec::new();
    for mapping in blocking {
        let key = entry_identity(mapping);
        match by_identity.remove(&key) {
            None => gaps.push(key),
            Some(proposal) => match resolve(mapping, proposal) {
                Some(resolved) => proposals.push(resolved),
                None => gaps.push(key),
            },
        }
    }

    let rationale_summary = format!(
        "resolved {} of {} blocking losses; {} listed as not known",
        proposals.len(),
        total,
        gaps.len()
    );

    Ok(ReviewArtifact {
        task,
        proposals,
        agent: RESOLVER_AGENT.to_string(),
        rationale_summary,
        gaps,
    })
}

/// Apply the loss-resolution rulings to one reasoner proposal for one entry.
///
/// Returns `None` when the proposal is not a loss resolution at all (a model
/// edit), in which case the caller records the entry as a gap.
fn resolve(mapping: &Mapping, proposal: Proposal) -> Option<Proposal> {
    let Proposal {
        action,
        rationale,
        confidence,
        subject: _,
    } = proposal;
    let subject = entry_identity(mapping);

    match mapping.verdict {
        MappingVerdict::Lossy => match action {
            ProposedAction::AcceptLoss => Some(Proposal {
                subject,
                action: ProposedAction::AcceptLoss,
                rationale: mapping.note.clone(),
                confidence,
            }),
            ProposedAction::Reject | ProposedAction::NoAction => Some(Proposal {
                subject,
                action,
                rationale,
                confidence,
            }),
            ProposedAction::EditElement | ProposedAction::DraftText => None,
        },
        MappingVerdict::Unmappable => match action {
            ProposedAction::AcceptLoss => Some(Proposal {
                subject,
                action: ProposedAction::Reject,
                rationale: UNMAPPABLE_REJECT_REASON.to_string(),
                confidence: Confidence::Low,
            }),
            ProposedAction::Reject | ProposedAction::NoAction => Some(Proposal {
                subject,
                action,
                rationale,
                confidence,
            }),
            ProposedAction::EditElement | ProposedAction::DraftText => None,
        },
        MappingVerdict::Exact => None,
    }
}
