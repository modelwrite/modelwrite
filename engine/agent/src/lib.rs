// SPDX-License-Identifier: AGPL-3.0-or-later
//! The agent contract: the proposals an agent may make about a model, and the
//! review artifact a human reads before any of them is committed.
//!
//! The two decisions that shape this crate and every one after it:
//!
//! 1. An agent is a client, never a privileged path. Nothing in this crate can
//!    change a model; it only produces Proposal values.
//! 2. No model change is committed by an agent alone. What a reasoner returns is
//!    a ReviewArtifact for a human to read; there is no type here that can
//!    express "commit without a human".
//!
//! The reasoning itself is a trait (Reasoner), so the shape of the interaction
//! is defined and tested with a scripted implementation before anything is
//! connected to a live provider.

mod report;

use std::fmt;

use binding::LossReport;
use okf::diff::DiffReport;
use okf::types::OkfRoot;
use serde::{Deserialize, Serialize};

/// How confident the agent is in a single proposal. Confidence belongs to the
/// proposal, not the agent, so a mixed batch still flags the one the agent is
/// unsure about instead of burying it under the ones it is sure about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Confidence {
    High,
    Medium,
    Low,
}

/// What the agent proposes to do about the material it was given.
///
/// The variant set is closed on purpose: these are the only things an agent may
/// propose, and whether a proposal changes the model is a property of the
/// variant, so it cannot be mislabelled. The two variants that change the model
/// (EditElement and DraftText) are the ones a human must accept before anything
/// is committed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProposedAction {
    /// Accept that something was lost or left unmapped. A decision recorded in
    /// the audit trail; it does not edit the model.
    AcceptLoss,
    /// Reject the material: the import, diff or proposal is not acceptable. A
    /// decision, not a model edit.
    Reject,
    /// Change an element in the model. This changes the model.
    EditElement,
    /// Draft text for the model (a requirement, a description). This changes the
    /// model.
    DraftText,
    /// There is nothing to do. An observation, not a change.
    NoAction,
}

impl ProposedAction {
    /// Whether acting on this proposal would change the model. A human reads the
    /// short list of proposals that return true here; everything else is an
    /// observation or a decision that changes nothing in the model.
    pub fn changes_model(&self) -> bool {
        matches!(
            self,
            ProposedAction::EditElement | ProposedAction::DraftText
        )
    }
}

/// One thing an agent proposes, with the reasoning and the confidence a human
/// needs to decide whether to accept it. A proposal is never a commit: it is a
/// suggestion with a subject, a rationale and a confidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub subject: String,
    pub action: ProposedAction,
    pub rationale: String,
    pub confidence: Confidence,
}

impl Proposal {
    /// Whether acting on this proposal would change the model.
    pub fn changes_model(&self) -> bool {
        self.action.changes_model()
    }

    /// Whether this proposal is marked for attention because the agent is not
    /// confident in it. A low-confidence proposal must be surfaced, never buried
    /// among the ones the agent is sure about.
    pub fn needs_attention(&self) -> bool {
        self.confidence == Confidence::Low
    }
}

/// The material an agent reasons over. The variant names what it is, so the
/// review artifact can say "about a loss report" or "about a document" without
/// the reader needing to know the types.
///
/// The variants differ in size because the material itself does: a whole OKF
/// document is legitimately larger than a diff or a loss report. This is a
/// contract payload, not a bulk collection, so indirection would add a Box to
/// the public API for no reader benefit.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize)]
pub enum Material {
    /// A binding's own account of what it could not carry into OKF.
    LossReport(LossReport),
    /// The engine's measurement of a round trip, from the fidelity harness.
    Diff(DiffReport),
    /// A document (or the excerpt in scope) the agent is asked to reason about.
    Document(OkfRoot),
}

impl Material {
    /// A one-line description for the review artifact, so the text names the
    /// material without requiring the reader to know the types behind it.
    pub fn describe(&self) -> String {
        match self {
            Material::LossReport(report) => format!(
                "loss report for binding {} ({} mappings, {} blocking)",
                report.binding.id,
                report.mappings.len(),
                report.blocking().len()
            ),
            Material::Diff(diff) => {
                if diff.equal {
                    "an engine diff (documents are equal)".to_string()
                } else {
                    format!(
                        "an engine diff ({} missing elements, {} extra elements, {} changed attributes, {} missing edges, {} extra edges)",
                        diff.missing_elements.len(),
                        diff.extra_elements.len(),
                        diff.changed_attributes.len(),
                        diff.missing_edges.len(),
                        diff.extra_edges.len()
                    )
                }
            }
            Material::Document(root) => {
                let elements = root.structure.len() + root.interfaces.len() + root.signals.len();
                format!(
                    "a document for project {} ({} elements, {} requirements)",
                    root.project,
                    elements,
                    root.requirements.len()
                )
            }
        }
    }
}

/// Everything a reasoner needs to reason: what is being asked, the material it
/// is asked about, and the constraints it must respect. Constraints are plain
/// strings a human wrote, so the agent's bounds are visible in the artifact.
#[derive(Debug, Clone, Serialize)]
pub struct AgentTask {
    pub goal: String,
    pub material: Material,
    pub constraints: Vec<String>,
}

impl AgentTask {
    /// The task must name a goal; otherwise there is nothing to reason about and
    /// no artifact worth reading. Reasoners are expected to call this and turn
    /// the result into a typed error rather than proceeding or panicking.
    pub fn validate(&self) -> Result<(), AgentError> {
        if self.goal.trim().is_empty() {
            return Err(AgentError::EmptyGoal);
        }
        Ok(())
    }
}

/// The single error type the agent contract uses. Reasoners cannot return
/// arbitrary strings or panic; every failure is one of these, so a caller can
/// react without parsing prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentError {
    /// The task has no goal, so there is nothing to reason about.
    EmptyGoal,
    /// The material could not be used (for example, a document that is not a
    /// valid OKF document).
    InvalidMaterial(String),
    /// The reasoner declined to produce proposals, with the reason.
    Refused(String),
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentError::EmptyGoal => write!(f, "agent task has no goal"),
            AgentError::InvalidMaterial(m) => write!(f, "agent material is not usable: {}", m),
            AgentError::Refused(m) => write!(f, "agent refused to propose: {}", m),
        }
    }
}

impl std::error::Error for AgentError {}

/// Turns a task into proposals.
///
/// This is a trait so the contract can be exercised by a scripted
/// implementation in tests before any live model is connected. Nothing in this
/// crate knows what produced the proposals, and nothing in this crate can turn
/// them into a model change: the only thing a reasoner may return is proposals
/// for a human to read.
pub trait Reasoner {
    /// Produce proposals for a task. The result is suggestions only; no model
    /// change happens here.
    fn propose(&self, task: &AgentTask) -> Result<Vec<Proposal>, AgentError>;
}

/// The record a human reads before anything is committed.
///
/// It carries the task, the proposals, the agent's identity and a summary, and
/// it is the audit artifact: it says who (an agent) proposed what, so a reader
/// a year later can tell who decided.
#[derive(Debug, Clone, Serialize)]
pub struct ReviewArtifact {
    pub task: AgentTask,
    pub proposals: Vec<Proposal>,
    pub agent: String,
    pub rationale_summary: String,
}

impl ReviewArtifact {
    /// The proposals a human must decide on before anything is committed: the
    /// ones that would change the model. Observations and recorded decisions
    /// (accepting a loss, rejecting material, doing nothing) do not block.
    pub fn blocking(&self) -> Vec<&Proposal> {
        self.proposals
            .iter()
            .filter(|p| p.changes_model())
            .collect()
    }

    /// Render the artifact as the plain text a human reads. The text needs no
    /// knowledge of the types, separates the proposals that change a model from
    /// the ones that do not, and marks low-confidence proposals for attention.
    pub fn render_text(&self) -> String {
        report::render(self)
    }
}

/// A reasoner with its answers written down in advance. It exists so tests can
/// pin the exact artifact a deterministic set of proposals renders to, before
/// any live model is connected.
#[derive(Debug, Clone)]
pub struct ScriptedReasoner {
    agent: String,
    proposals: Vec<Proposal>,
}

impl ScriptedReasoner {
    /// Build a scripted reasoner that answers every task with the same
    /// proposals, in the order given, under the given agent identity.
    pub fn new(agent: impl Into<String>, proposals: Vec<Proposal>) -> Self {
        ScriptedReasoner {
            agent: agent.into(),
            proposals,
        }
    }

    /// The identity this reasoner records as its agent.
    pub fn agent(&self) -> &str {
        &self.agent
    }

    /// Run this script over a task and wrap the result in a review artifact.
    /// The summary is fixed ("from its script"), so a test can pin the whole
    /// artifact deterministically.
    pub fn review(&self, task: AgentTask) -> Result<ReviewArtifact, AgentError> {
        let proposals = self.propose(&task)?;
        let rationale_summary = format!(
            "{} returned {} proposal(s) from its script",
            self.agent,
            proposals.len()
        );
        Ok(ReviewArtifact {
            task,
            proposals,
            agent: self.agent.clone(),
            rationale_summary,
        })
    }
}

impl Reasoner for ScriptedReasoner {
    fn propose(&self, task: &AgentTask) -> Result<Vec<Proposal>, AgentError> {
        task.validate()?;
        Ok(self.proposals.clone())
    }
}
