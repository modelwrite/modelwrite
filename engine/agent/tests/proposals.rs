// SPDX-License-Identifier: AGPL-3.0-or-later
use agent::{
    AgentError, AgentTask, Confidence, Material, Proposal, ProposedAction, Reasoner,
    ReviewArtifact, ScriptedReasoner,
};
use binding::{BindingInfo, Direction, LossReport, Mapping, MappingVerdict};

fn loss_report() -> LossReport {
    LossReport {
        binding: BindingInfo {
            id: "test-binding".to_string(),
            version: "1.0".to_string(),
            direction: Direction::ImportOnly,
            description: "test binding".to_string(),
        },
        mappings: vec![
            Mapping {
                subject: "BLOCK_A".to_string(),
                verdict: MappingVerdict::Exact,
                note: "carried fully".to_string(),
            },
            Mapping {
                subject: "LOST_SIGNAL".to_string(),
                verdict: MappingVerdict::Unmappable,
                note: "cannot express".to_string(),
            },
        ],
        artifact_hash: "abc123".to_string(),
    }
}

fn task() -> AgentTask {
    AgentTask {
        goal: "review the import loss".to_string(),
        material: Material::LossReport(loss_report()),
        constraints: vec!["do not edit requirements".to_string()],
    }
}

fn proposal(
    subject: &str,
    action: ProposedAction,
    rationale: &str,
    confidence: Confidence,
) -> Proposal {
    Proposal {
        subject: subject.to_string(),
        action,
        rationale: rationale.to_string(),
        confidence,
    }
}

// Two model-changing proposals (one of them low confidence), two recorded
// decisions, and one observation: the full spread a script must render.
fn script() -> Vec<Proposal> {
    vec![
        proposal(
            "block A",
            ProposedAction::EditElement,
            "rename it to match the source",
            Confidence::High,
        ),
        proposal(
            "lost signal",
            ProposedAction::AcceptLoss,
            "documented in the audit trail",
            Confidence::High,
        ),
        proposal(
            "requirement text",
            ProposedAction::DraftText,
            "needs wording a human should approve",
            Confidence::Low,
        ),
        proposal(
            "unmappable note",
            ProposedAction::Reject,
            "reimport after fixing the binding",
            Confidence::Medium,
        ),
        proposal(
            "already exact",
            ProposedAction::NoAction,
            "nothing to do",
            Confidence::High,
        ),
    ]
}

fn artifact() -> ReviewArtifact {
    ScriptedReasoner::new("scripted-agent", script())
        .review(task())
        .expect("scripted reasoner must produce an artifact")
}

#[test]
fn changes_model_is_a_property_of_the_action() {
    assert!(ProposedAction::EditElement.changes_model());
    assert!(ProposedAction::DraftText.changes_model());
    assert!(!ProposedAction::AcceptLoss.changes_model());
    assert!(!ProposedAction::Reject.changes_model());
    assert!(!ProposedAction::NoAction.changes_model());
}

#[test]
fn blocking_returns_only_proposals_that_change_the_model() {
    let artifact = artifact();
    let blocking = artifact.blocking();
    let subjects: Vec<&str> = blocking.iter().map(|p| p.subject.as_str()).collect();
    // The short list a human must decide on: exactly the two model-changing
    // proposals, and none of the decisions or observations.
    assert_eq!(subjects, vec!["block A", "requirement text"]);
}

#[test]
fn a_low_confidence_proposal_is_marked_for_attention() {
    let artifact = artifact();
    let draft = artifact
        .proposals
        .iter()
        .find(|p| p.action == ProposedAction::DraftText)
        .expect("the script has a draft-text proposal");
    assert!(draft.needs_attention());
    assert!(draft.confidence == Confidence::Low);

    let text = artifact.render_text();
    // The marker must sit next to the low-confidence proposal's own line, not
    // anywhere else in the artifact.
    assert!(
        text.contains("confidence: Low *** NEEDS ATTENTION ***"),
        "low-confidence proposal must be marked for attention, got:
{}",
        text
    );
    // A high-confidence proposal is not marked.
    let edit_line = text
        .lines()
        .find(|l| l.contains("confidence: High") && l.contains("edit element"))
        .expect("edit-element line must exist");
    assert!(!edit_line.contains("NEEDS ATTENTION"));
}

#[test]
fn render_text_separates_model_changes_from_observations() {
    let text = artifact().render_text();

    let changes = text
        .find("PROPOSALS THAT CHANGE THE MODEL")
        .expect("changes section");
    let observations = text
        .find("PROPOSALS THAT DO NOT CHANGE THE MODEL")
        .expect("observations section");

    // Every model-changing subject sits under the changes heading, every other
    // subject sits under the observations heading.
    for subject in ["block A", "requirement text"] {
        let pos = text.find(subject).expect("subject must be present");
        assert!(
            pos > changes && pos < observations,
            "{} must be in the changes section",
            subject
        );
    }
    for subject in ["lost signal", "unmappable note", "already exact"] {
        let pos = text.find(subject).expect("subject must be present");
        assert!(
            pos > observations,
            "{} must be in the observations section",
            subject
        );
    }
}

#[test]
fn the_artifact_records_which_agent_proposed() {
    let text = artifact().render_text();
    assert!(text.contains("Agent: scripted-agent"));
    // Attribution is not optional: the record says an automated agent produced
    // it, and that nothing here is a commit.
    assert!(text.contains("produced by an automated agent"));
    assert!(text.contains("Nothing here has changed"));
}

#[test]
fn a_scripted_reasoner_renders_a_deterministic_artifact() {
    let expected = r#"MODELWRITE AGENT REVIEW

This artifact was produced by an automated agent. Nothing here has changed
a model; every entry below is a proposal for a human to read, accept or reject.

Agent: scripted-agent
Goal: review the import loss

Summary
-------
scripted-agent returned 5 proposal(s) from its script

Material
--------
loss report for binding test-binding (2 mappings, 1 blocking)

Constraints
-----------
- do not edit requirements

Proposals: 5 total - 2 change the model, 1 require attention

PROPOSALS THAT CHANGE THE MODEL
-------------------------------
[1] block A  (action: edit element, confidence: High)
    rationale: rename it to match the source
[2] requirement text  (action: draft text, confidence: Low *** NEEDS ATTENTION ***)
    rationale: needs wording a human should approve

PROPOSALS THAT DO NOT CHANGE THE MODEL
--------------------------------------
[1] lost signal  (action: accept loss, confidence: High)
    rationale: documented in the audit trail
[2] unmappable note  (action: reject, confidence: Medium)
    rationale: reimport after fixing the binding
[3] already exact  (action: no action, confidence: High)
    rationale: nothing to do
"#;

    let first = artifact().render_text();
    let second = artifact().render_text();
    assert_eq!(first, second, "rendering must be deterministic");
    assert_eq!(first, expected, "the pinned artifact must not drift");
}

#[test]
fn an_empty_goal_is_a_typed_error() {
    let reasoner = ScriptedReasoner::new("scripted-agent", script());
    let empty = AgentTask {
        goal: "   ".to_string(),
        material: Material::LossReport(loss_report()),
        constraints: Vec::new(),
    };
    let err = reasoner
        .propose(&empty)
        .expect_err("an empty goal must be refused");
    assert_eq!(err, AgentError::EmptyGoal);
}

#[test]
fn a_document_material_renders_without_knowing_the_types() {
    use okf::types::{OkfRoot, Summary};
    let document = OkfRoot {
        okf: "1.0".to_string(),
        project: "coffee".to_string(),
        exported_at: String::new(),
        summary: Summary::default(),
        structure: Vec::new(),
        interfaces: Vec::new(),
        signals: Vec::new(),
        requirements: Vec::new(),
        state_machine: None,
        activities: Vec::new(),
        graph: None,
        provenance: None,
        references: Vec::new(),
    };
    let task = AgentTask {
        goal: "summarize the document".to_string(),
        material: Material::Document(document),
        constraints: Vec::new(),
    };
    let artifact = ScriptedReasoner::new("scripted-agent", vec![])
        .review(task)
        .expect("an empty script is a valid script");
    let text = artifact.render_text();
    assert!(text.contains("a document for project coffee (0 elements, 0 requirements)"));
    assert!(text.contains(
        "Constraints
-----------
(none)"
    ));
}
