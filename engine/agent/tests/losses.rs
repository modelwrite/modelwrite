// SPDX-License-Identifier: AGPL-3.0-or-later
//! The loss-report resolver, tested against the real binding fixtures rather
//! than an invented report: coffee-grinder.xmi (six named losses) and
//! unknown-element.xmi (two Unmappable entries plus a Lossy model id).

use agent::losses::{entry_identity, propose_loss_resolutions};
use agent::{Confidence, Proposal, ProposedAction, ScriptedReasoner};
use binding::{Binding, BindingInfo, Direction, LossReport, Mapping, MappingVerdict};
use binding_xmi::XmiBinding;

/// Import a hand-written XMI fixture from the binding-xmi crate and return its
/// loss report.
fn report_for(name: &str) -> LossReport {
    let path = format!(
        "{}/../binding-xmi/fixtures/{}",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    let bytes = std::fs::read_to_string(path).expect("fixture must exist");
    let (_, report) = XmiBinding::new()
        .import(bytes.as_bytes())
        .expect("fixture must import");
    report
}

/// A reasoner that proposes to accept every blocking entry, with a deliberately
/// invented rationale, so the resolver's enforcement is what a test observes.
fn accept_everything(report: &LossReport) -> ScriptedReasoner {
    let proposals = report
        .blocking()
        .iter()
        .map(|m| Proposal {
            subject: entry_identity(m),
            action: ProposedAction::AcceptLoss,
            rationale: "the script accepts whatever it is shown".to_string(),
            confidence: Confidence::High,
        })
        .collect();
    ScriptedReasoner::new("scripted-accept-all", proposals)
}

#[test]
fn the_coffee_grinder_six_named_losses_produce_six_proposals() {
    let report = report_for("coffee-grinder.xmi");
    assert_eq!(report.blocking().len(), 6);

    let artifact = propose_loss_resolutions(&report, &accept_everything(&report))
        .expect("the resolver must succeed");

    assert_eq!(
        artifact.proposals.len(),
        6,
        "one proposal per blocking entry"
    );
    assert!(artifact.gaps.is_empty(), "the script answered every entry");

    // Each of the six is a Lossy loss, proposed for acceptance with the reason
    // taken from the report's own note, not the script's invented rationale.
    for mapping in report.blocking() {
        let proposal = artifact
            .proposals
            .iter()
            .find(|p| p.subject == entry_identity(mapping))
            .expect("every blocking entry must have a proposal");
        assert_eq!(proposal.action, ProposedAction::AcceptLoss);
        assert_eq!(proposal.rationale, mapping.note);
    }

    // And the six subjects are exactly the six named losses.
    let mut subjects: Vec<&str> = artifact
        .proposals
        .iter()
        .map(|p| p.subject.as_str())
        .collect();
    subjects.sort_unstable();
    assert_eq!(
        subjects,
        vec![
            "uml:Comment doc-grinder [lossy]",
            "uml:Dependency dep-satisfy [lossy]",
            "uml:Model model-grinder [lossy]",
            "uml:Package pkg-structure (Structure) [lossy]",
            "uml:Property prop-capacity [lossy]",
            "uml:Property prop-motor [lossy]",
        ]
    );
}

#[test]
fn an_unmappable_entry_is_never_proposed_for_acceptance() {
    let report = report_for("unknown-element.xmi");
    let blocking = report.blocking();
    // One Lossy (the model id) and two Unmappable entries.
    assert_eq!(blocking.len(), 3);
    assert_eq!(
        blocking
            .iter()
            .filter(|m| m.verdict == MappingVerdict::Unmappable)
            .count(),
        2
    );

    let artifact = propose_loss_resolutions(&report, &accept_everything(&report))
        .expect("the resolver must succeed");

    assert_eq!(artifact.proposals.len(), 3);
    assert!(artifact.gaps.is_empty());

    // The two Unmappable entries are Reject-with-reason, never an acceptance.
    for mapping in blocking
        .iter()
        .filter(|m| m.verdict == MappingVerdict::Unmappable)
    {
        let proposal = artifact
            .proposals
            .iter()
            .find(|p| p.subject == entry_identity(mapping))
            .expect("every blocking entry must have a proposal");
        assert_eq!(
            proposal.action,
            ProposedAction::Reject,
            "an Unmappable entry must not be accepted, got {:?}",
            proposal.action
        );
        assert!(
            proposal.rationale.contains("nothing to map"),
            "the rejection must carry a reason, got: {}",
            proposal.rationale
        );
    }

    // The Lossy model id is still proposed for acceptance with the report's note.
    let lossy = blocking
        .iter()
        .find(|m| m.verdict == MappingVerdict::Lossy)
        .expect("a Lossy entry is present");
    let proposal = artifact
        .proposals
        .iter()
        .find(|p| p.subject == entry_identity(lossy))
        .expect("the Lossy entry must have a proposal");
    assert_eq!(proposal.action, ProposedAction::AcceptLoss);
    assert_eq!(proposal.rationale, lossy.note);
}

#[test]
fn the_artifact_names_the_entries_the_reasoner_had_nothing_to_say_about() {
    let report = report_for("coffee-grinder.xmi");
    let blocking = report.blocking();

    // A reasoner that answers only the first two entries; the other four are gaps.
    let proposals = blocking
        .iter()
        .take(2)
        .map(|m| Proposal {
            subject: entry_identity(m),
            action: ProposedAction::AcceptLoss,
            rationale: "scripted".to_string(),
            confidence: Confidence::High,
        })
        .collect();
    let reasoner = ScriptedReasoner::new("scripted-partial", proposals);

    let artifact = propose_loss_resolutions(&report, &reasoner).expect("the resolver must succeed");

    assert_eq!(artifact.proposals.len(), 2);
    assert_eq!(
        artifact.gaps.len(),
        4,
        "the four unanswered entries are gaps"
    );

    let mut gaps = artifact.gaps.clone();
    gaps.sort_unstable();
    let mut expected: Vec<String> = blocking.iter().skip(2).map(|m| entry_identity(m)).collect();
    expected.sort_unstable();
    assert_eq!(gaps, expected);

    // The rendered artifact lists what the agent does not know, by name.
    let text = artifact.render_text();
    assert!(text.contains("WHAT THE AGENT DOES NOT KNOW"));
    for gap in &artifact.gaps {
        assert!(
            text.contains(gap.as_str()),
            "the artifact must name gap {}",
            gap
        );
    }
}

#[test]
fn two_entries_sharing_a_subject_are_two_decisions() {
    // A single XMI comment can produce a Lossy entry for its dropped id and an
    // Unmappable entry for its dropped body. A resolver keyed on the subject
    // alone would collapse them; the (subject, verdict) identity keeps them two
    // decisions, so an acceptance of one never silently accepts the other.
    let report = LossReport {
        binding: BindingInfo {
            id: "test-binding".to_string(),
            version: "1.0".to_string(),
            direction: Direction::ImportOnly,
            description: "test".to_string(),
        },
        mappings: vec![
            Mapping {
                subject: "uml:Comment c1".to_string(),
                verdict: MappingVerdict::Lossy,
                note: "uml:Comment xmi:id dropped".to_string(),
            },
            Mapping {
                subject: "uml:Comment c1".to_string(),
                verdict: MappingVerdict::Unmappable,
                note: "uml:Comment body dropped".to_string(),
            },
        ],
        artifact_hash: "abc".to_string(),
    };

    let reasoner = ScriptedReasoner::new(
        "scripted",
        vec![
            Proposal {
                subject: "uml:Comment c1 [lossy]".to_string(),
                action: ProposedAction::AcceptLoss,
                rationale: "scripted".to_string(),
                confidence: Confidence::High,
            },
            Proposal {
                subject: "uml:Comment c1 [unmappable]".to_string(),
                action: ProposedAction::Reject,
                rationale: "scripted".to_string(),
                confidence: Confidence::High,
            },
        ],
    );

    let artifact = propose_loss_resolutions(&report, &reasoner).expect("the resolver must succeed");

    assert_eq!(
        artifact.proposals.len(),
        2,
        "two entries sharing a subject must be two decisions"
    );
    assert!(artifact.gaps.is_empty());

    let lossy = artifact
        .proposals
        .iter()
        .find(|p| p.subject == "uml:Comment c1 [lossy]")
        .expect("the Lossy decision must exist");
    let unmappable = artifact
        .proposals
        .iter()
        .find(|p| p.subject == "uml:Comment c1 [unmappable]")
        .expect("the Unmappable decision must exist");

    assert_eq!(lossy.action, ProposedAction::AcceptLoss);
    assert_eq!(lossy.rationale, "uml:Comment xmi:id dropped");
    assert_eq!(unmappable.action, ProposedAction::Reject);
}

#[test]
fn the_artifact_states_the_measurement_boundary() {
    let report = report_for("coffee-grinder.xmi");
    let artifact = propose_loss_resolutions(&report, &accept_everything(&report))
        .expect("the resolver must succeed");

    // The boundary is a named constraint on the task, and it renders into the
    // artifact so a reader is never told the report is verified.
    assert!(
        artifact
            .task
            .constraints
            .iter()
            .any(|c| c.contains("not ground truth")),
        "the boundary must be a task constraint"
    );
    let text = artifact.render_text();
    assert!(
        text.contains("not ground truth"),
        "the rendered artifact must carry the boundary"
    );
}

#[test]
fn a_lossless_report_produces_no_proposals_and_no_gaps() {
    let report = LossReport {
        binding: BindingInfo {
            id: "test-binding".to_string(),
            version: "1.0".to_string(),
            direction: Direction::ImportOnly,
            description: "test".to_string(),
        },
        mappings: vec![Mapping {
            subject: "block A".to_string(),
            verdict: MappingVerdict::Exact,
            note: "carried fully".to_string(),
        }],
        artifact_hash: "abc".to_string(),
    };
    assert!(report.is_lossless());

    let artifact = propose_loss_resolutions(&report, &accept_everything(&report))
        .expect("the resolver must succeed");
    assert!(artifact.proposals.is_empty());
    assert!(artifact.gaps.is_empty());
}
