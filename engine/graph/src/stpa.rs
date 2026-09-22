// SPDX-License-Identifier: AGPL-3.0-or-later
//! The STPA / STAMP completeness checks.
//!
//! The platform does NOT perform STPA. It holds the authored analysis as a typed OKF graph
//! (the stereotype convention declared in `sample/stpa/stpa-vocabulary.json`) and CHECKS that
//! the analysis is complete and internally consistent. Every check below is a pure function of
//! the document and every finding names what was computed over and what the model did not carry
//! - the basis rule, applied to the safety argument.
//!
//! ABOVE the five relational checks sits a METHOD-COVERAGE layer. Every relational check
//! tests a relationship BETWEEN STPA elements, so a model that carries none of them returns
//! zero findings - and zero findings over nothing used to read as "complete". That is a
//! vacuous truth, and for a safety engineer it is a false assurance. The coverage layer checks
//! that each STPA stage was PERFORMED AT ALL before its internal consistency is judged, and
//! answers with one of three states ([StpaVerdict]): not started (nothing was measured over),
//! performed with gaps found, and performed with no gaps. Only the last may read as clean.
//!
//! The checks are the five named in the STPA design note:
//!
//! 1. unanalysed control actions (a ControlAction missing one or more of the four UCA types),
//! 2. control loops with no feedback (a Controller with a ControlAction and no Feedback path),
//! 3. hazards with no constraint, and constraints reaching no element,
//! 4. UCAs with no LossScenario,
//! 5. trend across baselines (computed by the server over a branch's commits, not here).
//!
//! Check 2 reuses the same graph-adjacency reading the orphan/isolation detector
//! ([crate::graph_stats] / [crate::components]) is built on: it walks the same edges over the
//! same node index, so the feedback gap is a degree check, not a second connected-component
//! implementation. Check 3's "constraints reaching no element" half is literally
//! [crate::requirement_coverage] filtered to SystemConstraint requirements, so the page can
//! never disagree with the gate about what a constraint reaches.

use std::collections::{HashMap, HashSet};

use okf::types::{GraphNode, OkfRoot};
use serde::Serialize;

/// Stereotype names in the declared STPA domain pack.
pub const STEREOTYPE_LOSS: &str = "Loss";
pub const STEREOTYPE_HAZARD: &str = "Hazard";
pub const STEREOTYPE_CONSTRAINT: &str = "SystemConstraint";
pub const STEREOTYPE_CONTROLLER: &str = "Controller";
pub const STEREOTYPE_PROCESS: &str = "ControlledProcess";
pub const STEREOTYPE_CONTROL_ACTION: &str = "ControlAction";
pub const STEREOTYPE_FEEDBACK: &str = "Feedback";
pub const STEREOTYPE_UCA: &str = "UnsafeControlAction";
pub const STEREOTYPE_LOSS_SCENARIO: &str = "LossScenario";
pub const STEREOTYPE_CAUSAL_FACTOR: &str = "CausalFactor";

/// The four STPA UCA types, carried as the second stereotype of an UnsafeControlAction node.
pub const UCA_TYPES: &[&str] = &[
    "not-provided",
    "provided",
    "wrong-timing-or-order",
    "stopped-too-soon-or-applied-too-long",
];

/// Whether a graph node declares the given stereotype.
pub fn is_stereotype(node: &GraphNode, stereotype: &str) -> bool {
    node.stereotypes.iter().any(|s| s == stereotype)
}

/// The UCA type token of a node, when the node is an UnsafeControlAction carrying one of the
/// four declared types. A UCA without a recognised type is treated as untyped (its reference to
/// a control action still counts, but it never satisfies a type's coverage).
pub fn uca_type(node: &GraphNode) -> Option<&str> {
    if !is_stereotype(node, STEREOTYPE_UCA) {
        return None;
    }
    node.stereotypes
        .iter()
        .map(|s| s.as_str())
        .find(|s| UCA_TYPES.contains(s))
}

/// One STPA method stage the model has not performed at all.
///
/// A relational finding says the analysis contradicts itself. A method gap says the stage was
/// never run, so the relational check over it has no subject at all. The two are different
/// failures and must never be collapsed: a check with no subject returns no findings, and no
/// findings is not a pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MethodGap {
    /// Stable stage token, for a test or a client to key on: "losses", "hazards",
    /// "system-constraints", "control-structure", "unsafe-control-actions".
    pub stage: String,
    /// The stage's human name.
    pub stage_name: String,
    /// One line naming what the model does not carry.
    pub headline: String,
    /// What the stage is, and what has to exist before the checks below can say anything.
    pub detail: String,
    /// What the gap was computed over - the basis rule, applied to the method itself.
    pub basis: String,
}

/// The three states of an STPA completeness check.
///
/// "Nothing to measure" and "nothing wrong" are different states, and a check must never let
/// the first read as the second. This is the same distinction the project-health cards draw
/// between "no commits yet - nothing to measure" and "no gaps".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StpaVerdict {
    /// The model lacks the elements the method needs, so the stages were never performed and
    /// nothing was measured over. This is NEVER a pass, however few findings it produces.
    NotStarted,
    /// The stages were performed and at least one relational check found a gap.
    GapsFound,
    /// Every stage was performed and every relational check is silent. The only state in which
    /// a clean result may be shown.
    Performed,
}

/// Whether one relational check has a subject in the model at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckState {
    /// The model carries the elements the check relates, so its result is real - including a
    /// result of zero findings.
    Measured,
    /// The model carries none of the check's subject, so the check has nothing to relate. Its
    /// empty result means nothing was measured, not that nothing is wrong.
    NotMeasurable,
}

/// Per-check coverage: which of the relational checks has a subject in this model. A page must
/// render a [CheckState::NotMeasurable] check as "cannot be evaluated yet", never as clean.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckCoverage {
    pub unanalysed_actions: CheckState,
    pub feedback_loops: CheckState,
    pub hazards_without_constraints: CheckState,
    pub constraints_without_elements: CheckState,
    pub ucas_without_scenarios: CheckState,
}

/// One control action with incomplete UCA coverage: which of the four types the model did not
/// carry, and which it did.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnanalysedAction {
    pub action_id: String,
    pub action_name: String,
    pub missing_types: Vec<String>,
    pub covered_types: Vec<String>,
    pub basis: String,
}

/// One controller with a control action but no feedback path.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackGap {
    pub controller_id: String,
    pub controller_name: String,
    pub control_actions: Vec<String>,
    pub basis: String,
}

/// One hazard with no system-level constraint.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HazardFinding {
    pub hazard_id: String,
    pub hazard_name: String,
    pub basis: String,
}

/// One system-level constraint that reaches no design element.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstraintFinding {
    pub constraint_id: String,
    pub constraint_name: String,
    pub basis: String,
}

/// One UCA with no loss scenario.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UcaWithoutScenario {
    pub uca_id: String,
    pub uca_name: String,
    pub basis: String,
}

/// The whole completeness report, computed once from the document.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StpaReport {
    /// The elements the method-coverage layer measures its stages over. Losses are counted here
    /// for the first time because the performed-and-clean state has to name what it measured.
    pub losses: usize,
    pub control_actions: usize,
    pub controllers: usize,
    pub controlled_processes: usize,
    pub feedback: usize,
    pub hazards: usize,
    pub constraints: usize,
    pub ucas: usize,
    pub loss_scenarios: usize,
    /// The STPA stages the model has not performed. Empty only when every stage was performed,
    /// which is what makes [StpaReport::verdict] unable to report a vacuous pass.
    pub method_gaps: Vec<MethodGap>,
    pub unanalysed_actions: Vec<UnanalysedAction>,
    pub feedback_gaps: Vec<FeedbackGap>,
    pub hazards_without_constraints: Vec<HazardFinding>,
    pub constraints_without_elements: Vec<ConstraintFinding>,
    pub ucas_without_scenarios: Vec<UcaWithoutScenario>,
    pub total_findings: usize,
    pub basis: String,
}

impl StpaReport {
    fn empty(basis: String) -> Self {
        StpaReport {
            losses: 0,
            control_actions: 0,
            controllers: 0,
            controlled_processes: 0,
            feedback: 0,
            hazards: 0,
            constraints: 0,
            ucas: 0,
            loss_scenarios: 0,
            method_gaps: method_gaps(0, 0, 0, 0, 0, 0),
            unanalysed_actions: Vec::new(),
            feedback_gaps: Vec::new(),
            hazards_without_constraints: Vec::new(),
            constraints_without_elements: Vec::new(),
            ucas_without_scenarios: Vec::new(),
            total_findings: 0,
            basis,
        }
    }

    /// The three-state verdict, COMPUTED and never stored.
    ///
    /// Because it is a function of the method gaps, and the gaps are a function of the elements
    /// the model carries, there is no way for a report to claim a clean result over an empty
    /// stage: a model with no STPA elements lands in [StpaVerdict::NotStarted] by construction,
    /// not by a template happening to check a flag.
    pub fn verdict(&self) -> StpaVerdict {
        if !self.method_gaps.is_empty() {
            StpaVerdict::NotStarted
        } else if self.total_findings > 0 {
            StpaVerdict::GapsFound
        } else {
            StpaVerdict::Performed
        }
    }

    /// Whether the method was performed and every relational check is silent. This is the ONLY
    /// state in which a clean result may be shown.
    pub fn is_clean(&self) -> bool {
        self.verdict() == StpaVerdict::Performed
    }

    /// Whether the model lacks the elements the method needs, so nothing was measured over.
    pub fn is_not_started(&self) -> bool {
        self.verdict() == StpaVerdict::NotStarted
    }

    /// Per-check coverage: which relational check has a subject in this model. A check with no
    /// subject returns zero findings, and zero findings over nothing is not a pass.
    pub fn check_coverage(&self) -> CheckCoverage {
        let state = |subject: usize| {
            if subject == 0 {
                CheckState::NotMeasurable
            } else {
                CheckState::Measured
            }
        };
        CheckCoverage {
            unanalysed_actions: state(self.control_actions),
            feedback_loops: state(self.controllers),
            hazards_without_constraints: state(self.hazards),
            constraints_without_elements: state(self.constraints),
            ucas_without_scenarios: state(self.ucas),
        }
    }
}

/// The method-coverage checks: the STPA stages that have to be performed before the relational
/// checks can say anything. A stage is missing when the model carries none of the elements it
/// produces.
///
/// The last two are conditioned on the stage before them. A control structure is only missing
/// if there are hazards it should have been built to control, and the UCA enumeration is only
/// missing if there is a control structure to enumerate over - otherwise naming them would
/// report a stage that the method has not yet reached as though it had been skipped.
fn method_gaps(
    losses: usize,
    hazards: usize,
    constraints: usize,
    controllers: usize,
    control_actions: usize,
    ucas: usize,
) -> Vec<MethodGap> {
    let mut gaps = Vec::new();

    if losses == 0 {
        gaps.push(MethodGap {
            stage: "losses".to_string(),
            stage_name: "Loss identification".to_string(),
            headline: "no Loss has been identified".to_string(),
            detail: concat!(
                "A Loss is something of value that could be lost - what the analysis exists ",
                "to prevent. Hazards are anchored to losses, so with no Loss the safety ",
                "argument has nothing to be about."
            )
            .to_string(),
            basis: "computed over Loss nodes; the model declares none".to_string(),
        });
    }

    if hazards == 0 {
        gaps.push(MethodGap {
            stage: "hazards".to_string(),
            stage_name: "Hazard identification".to_string(),
            headline: "no Hazard has been identified".to_string(),
            detail: concat!(
                "A Hazard is a system state that, in a worst-case environment, leads to a ",
                "Loss. Naming the hazards is the first step of STPA, and every check below ",
                "is a relation between a Hazard and the rest of the model - so with no ",
                "Hazard those checks have nothing to relate. Returning no findings over no ",
                "hazards means the analysis has not started, not that it is complete."
            )
            .to_string(),
            basis: "computed over Hazard nodes; the model declares none".to_string(),
        });
    }

    if constraints == 0 {
        gaps.push(MethodGap {
            stage: "system-constraints".to_string(),
            stage_name: "System-constraint definition".to_string(),
            headline: "no SystemConstraint has been defined".to_string(),
            detail: concat!(
                "A SystemConstraint is the system-level behaviour that must hold to prevent ",
                "the hazards. Without one the hazards may be named, but nothing constrains ",
                "them."
            )
            .to_string(),
            basis: concat!(
                "computed over requirements carrying the SystemConstraint stereotype; the ",
                "model declares none"
            )
            .to_string(),
        });
    }

    if hazards > 0 && controllers == 0 && control_actions == 0 {
        gaps.push(MethodGap {
            stage: "control-structure".to_string(),
            stage_name: "Control-structure modelling".to_string(),
            headline: "no Controller or ControlAction has been modelled".to_string(),
            detail: concat!(
                "STPA analyses how the control of a system can become unsafe, so it needs ",
                "the control structure the hazards arise in. This model names its hazards ",
                "but carries no Controller and no ControlAction to control them with."
            )
            .to_string(),
            basis: format!(
                concat!(
                    "computed over Controller and ControlAction nodes; the model declares none, ",
                    "though it declares {} hazard(s)"
                ),
                hazards
            ),
        });
    }

    if (controllers > 0 || control_actions > 0) && ucas == 0 {
        gaps.push(MethodGap {
            stage: "unsafe-control-actions".to_string(),
            stage_name: "Unsafe-control-action identification".to_string(),
            headline: "no UnsafeControlAction has been identified".to_string(),
            detail: concat!(
                "The model carries a control structure but no UnsafeControlAction. ",
                "Enumerating how each control action can be unsafe - not provided, ",
                "provided, wrong timing or order, stopped too soon or applied too long - is ",
                "the step that turns a control structure into an analysis."
            )
            .to_string(),
            basis: concat!(
                "computed over UnsafeControlAction nodes; the model declares none, though it ",
                "declares a control structure"
            )
            .to_string(),
        });
    }

    gaps
}

/// Count the graph nodes carrying a stereotype.
fn count(graph: &[GraphNode], stereotype: &str) -> usize {
    graph
        .iter()
        .filter(|n| is_stereotype(n, stereotype))
        .count()
}

/// The one entry point: compute all four document-level checks and the completeness report.
/// Returns an empty report (no findings, the basis saying so) when the document has no graph.
pub fn stpa_report(root: &OkfRoot) -> StpaReport {
    let Some(graph) = root.graph.as_ref() else {
        return StpaReport::empty(
            "no graph section: there are no relationships to check".to_string(),
        );
    };

    let nodes: HashMap<&str, &GraphNode> = graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let name_of = |id: &str| -> String {
        nodes
            .get(id)
            .filter(|n| !n.name.is_empty())
            .map(|n| n.name.clone())
            .unwrap_or_else(|| id.to_string())
    };

    let unanalysed_actions = unanalysed_control_actions(graph, &nodes, &name_of);
    let feedback_gaps = feedback_gaps(graph, &nodes, &name_of);
    let (hazards_without_constraints, constraints_without_elements) =
        hazard_and_constraint_findings(root, graph, &nodes, &name_of);
    let ucas_without_scenarios = ucas_without_scenario(graph, &nodes, &name_of);

    let total_findings = unanalysed_actions.len()
        + feedback_gaps.len()
        + hazards_without_constraints.len()
        + constraints_without_elements.len()
        + ucas_without_scenarios.len();

    let basis = format!(
        "computed over {} graph nodes and {} graph edges; the platform holds the authored analysis and checks its completeness - it does not perform STPA",
        graph.nodes.len(),
        graph.edges.len()
    );

    // The elements every stage is measured over, read once so the counts the method-coverage
    // layer judges and the counts the report publishes can never disagree.
    let losses = count(&graph.nodes, STEREOTYPE_LOSS);
    let control_actions = count(&graph.nodes, STEREOTYPE_CONTROL_ACTION);
    let controllers = count(&graph.nodes, STEREOTYPE_CONTROLLER);
    let hazards = count(&graph.nodes, STEREOTYPE_HAZARD);
    let constraints = root
        .requirements
        .iter()
        .filter(|r| is_requirement_stereotype(r, STEREOTYPE_CONSTRAINT))
        .count();
    let ucas = count(&graph.nodes, STEREOTYPE_UCA);

    // The method-coverage layer: which stages were performed at all, judged before the
    // relational checks below judge how consistently.
    let method_gaps = method_gaps(
        losses,
        hazards,
        constraints,
        controllers,
        control_actions,
        ucas,
    );

    StpaReport {
        losses,
        control_actions,
        controllers,
        controlled_processes: count(&graph.nodes, STEREOTYPE_PROCESS),
        feedback: count(&graph.nodes, STEREOTYPE_FEEDBACK),
        hazards,
        constraints,
        ucas,
        loss_scenarios: count(&graph.nodes, STEREOTYPE_LOSS_SCENARIO),
        method_gaps,
        unanalysed_actions,
        feedback_gaps,
        hazards_without_constraints,
        constraints_without_elements,
        ucas_without_scenarios,
        total_findings,
        basis,
    }
}

/// A requirement declares a stereotype.
fn is_requirement_stereotype(requirement: &okf::types::Requirement, stereotype: &str) -> bool {
    requirement.stereotypes.iter().any(|s| s == stereotype)
}

/// Check 1: every ControlAction must carry a UCA for each of the four UCA types. A UCA covers a
/// type for an action when it references the action (a `reference` edge UCA -> ControlAction)
/// and carries that type token.
fn unanalysed_control_actions(
    graph: &okf::types::Graph,
    nodes: &HashMap<&str, &GraphNode>,
    name_of: &dyn Fn(&str) -> String,
) -> Vec<UnanalysedAction> {
    let control_actions: Vec<&GraphNode> = graph
        .nodes
        .iter()
        .filter(|n| is_stereotype(n, STEREOTYPE_CONTROL_ACTION))
        .collect();

    let mut covered: HashMap<&str, HashSet<&str>> = HashMap::new();
    for edge in &graph.edges {
        if edge.kind != "reference" {
            continue;
        }
        let Some(ty) = nodes.get(edge.source.as_str()).and_then(|n| uca_type(n)) else {
            continue;
        };
        let is_action = nodes
            .get(edge.target.as_str())
            .map(|n| is_stereotype(n, STEREOTYPE_CONTROL_ACTION))
            .unwrap_or(false);
        if is_action {
            covered.entry(edge.target.as_str()).or_default().insert(ty);
        }
    }

    let mut findings = Vec::new();
    for action in &control_actions {
        let covered_types = covered
            .get(action.id.as_str())
            .map(|set| {
                let mut v: Vec<String> = set.iter().map(|s| (*s).to_string()).collect();
                v.sort();
                v
            })
            .unwrap_or_default();
        let covered_set = covered.get(action.id.as_str());
        let missing_types: Vec<String> = UCA_TYPES
            .iter()
            .filter(|t| !covered_set.map(|set| set.contains(*t)).unwrap_or(false))
            .map(|t| (*t).to_string())
            .collect();
        if missing_types.is_empty() {
            continue;
        }
        let covered_desc = if covered_types.is_empty() {
            "none".to_string()
        } else {
            covered_types.join(", ")
        };
        let basis = format!(
            "computed over the UCA nodes that reference {}; the model carries {} of the four UCA types ({}) and does not carry ({})",
            action.id,
            covered_types.len(),
            covered_desc,
            missing_types.join(", ")
        );
        findings.push(UnanalysedAction {
            action_id: action.id.clone(),
            action_name: name_of(&action.id),
            missing_types,
            covered_types,
            basis,
        });
    }
    findings.sort_by(|a, b| a.action_id.cmp(&b.action_id));
    findings
}

/// Check 2: a Controller with a ControlAction and no Feedback path is the classic STPA defect.
/// A controller has a feedback path when a Feedback signal reports back to it (a `triggers`
/// edge Feedback -> Controller). This is the same directed-degree reading over the same node
/// index the orphan/isolation detector uses, applied to the control structure's edges.
fn feedback_gaps(
    graph: &okf::types::Graph,
    nodes: &HashMap<&str, &GraphNode>,
    name_of: &dyn Fn(&str) -> String,
) -> Vec<FeedbackGap> {
    let controllers: Vec<&GraphNode> = graph
        .nodes
        .iter()
        .filter(|n| is_stereotype(n, STEREOTYPE_CONTROLLER))
        .collect();

    let mut issued: HashMap<&str, Vec<String>> = HashMap::new();
    let mut has_feedback: HashSet<&str> = HashSet::new();
    for edge in &graph.edges {
        if edge.kind != "triggers" {
            continue;
        }
        let source_is_controller = nodes
            .get(edge.source.as_str())
            .map(|n| is_stereotype(n, STEREOTYPE_CONTROLLER))
            .unwrap_or(false);
        let target_is_action = nodes
            .get(edge.target.as_str())
            .map(|n| is_stereotype(n, STEREOTYPE_CONTROL_ACTION))
            .unwrap_or(false);
        if source_is_controller && target_is_action {
            issued
                .entry(edge.source.as_str())
                .or_default()
                .push(edge.target.clone());
        }
        let source_is_feedback = nodes
            .get(edge.source.as_str())
            .map(|n| is_stereotype(n, STEREOTYPE_FEEDBACK))
            .unwrap_or(false);
        let target_is_controller = nodes
            .get(edge.target.as_str())
            .map(|n| is_stereotype(n, STEREOTYPE_CONTROLLER))
            .unwrap_or(false);
        if source_is_feedback && target_is_controller {
            has_feedback.insert(edge.target.as_str());
        }
    }

    let mut findings = Vec::new();
    for controller in &controllers {
        let Some(actions) = issued.get(controller.id.as_str()) else {
            continue;
        };
        if has_feedback.contains(controller.id.as_str()) {
            continue;
        }
        let mut actions = actions.clone();
        actions.sort();
        actions.dedup();
        let basis = format!(
            "computed over the triggers edges of the control structure; {} issues control action(s) {} but no Feedback signal reports back to it",
            controller.id,
            actions.join(", ")
        );
        findings.push(FeedbackGap {
            controller_id: controller.id.clone(),
            controller_name: name_of(&controller.id),
            control_actions: actions,
            basis,
        });
    }
    findings.sort_by(|a, b| a.controller_id.cmp(&b.controller_id));
    findings
}

/// Check 3: every hazard must be constrained, and every constraint must reach the design.
/// The first half reads `dependency` edges labelled Mitigate from a Hazard to a SystemConstraint;
/// the second half reuses [crate::requirement_coverage], filtered to SystemConstraint requirements.
fn hazard_and_constraint_findings(
    root: &OkfRoot,
    graph: &okf::types::Graph,
    nodes: &HashMap<&str, &GraphNode>,
    name_of: &dyn Fn(&str) -> String,
) -> (Vec<HazardFinding>, Vec<ConstraintFinding>) {
    let hazards: Vec<&GraphNode> = graph
        .nodes
        .iter()
        .filter(|n| is_stereotype(n, STEREOTYPE_HAZARD))
        .collect();

    let constraint_ids: HashSet<&str> = root
        .requirements
        .iter()
        .filter(|r| is_requirement_stereotype(r, STEREOTYPE_CONSTRAINT))
        .map(|r| r.id.as_str())
        .collect();

    let mut mitigated: HashSet<&str> = HashSet::new();
    for edge in &graph.edges {
        if edge.kind != "dependency" || edge.label != "Mitigate" {
            continue;
        }
        let source_is_hazard = nodes
            .get(edge.source.as_str())
            .map(|n| is_stereotype(n, STEREOTYPE_HAZARD))
            .unwrap_or(false);
        if source_is_hazard && constraint_ids.contains(edge.target.as_str()) {
            mitigated.insert(edge.source.as_str());
        }
    }

    let mut hazards_without = Vec::new();
    for hazard in &hazards {
        if mitigated.contains(hazard.id.as_str()) {
            continue;
        }
        let basis = format!(
            "computed over dependency edges labelled Mitigate; {} has no Mitigate edge to a SystemConstraint",
            hazard.id
        );
        hazards_without.push(HazardFinding {
            hazard_id: hazard.id.clone(),
            hazard_name: name_of(&hazard.id),
            basis,
        });
    }
    hazards_without.sort_by(|a, b| a.hazard_id.cmp(&b.hazard_id));

    let coverage = crate::requirement_coverage(root);
    let uncovered: HashSet<&str> = coverage.uncovered.iter().map(|s| s.as_str()).collect();
    let mut constraints_without = Vec::new();
    for requirement in &root.requirements {
        if !is_requirement_stereotype(requirement, STEREOTYPE_CONSTRAINT) {
            continue;
        }
        if !uncovered.contains(requirement.id.as_str()) {
            continue;
        }
        let basis = format!(
            "computed over the platform's requirement coverage (Satisfy/Refine/Verify/Allocate); {} is a SystemConstraint no design element satisfies",
            requirement.id
        );
        constraints_without.push(ConstraintFinding {
            constraint_id: requirement.id.clone(),
            constraint_name: if requirement.name.is_empty() {
                requirement.id.clone()
            } else {
                requirement.name.clone()
            },
            basis,
        });
    }
    constraints_without.sort_by(|a, b| a.constraint_id.cmp(&b.constraint_id));

    (hazards_without, constraints_without)
}

/// Check 4: every UCA must be explained by a loss scenario. A UCA is explained when a
/// LossScenario node references it (a `reference` edge LossScenario -> UCA).
fn ucas_without_scenario(
    graph: &okf::types::Graph,
    nodes: &HashMap<&str, &GraphNode>,
    name_of: &dyn Fn(&str) -> String,
) -> Vec<UcaWithoutScenario> {
    let ucas: Vec<&GraphNode> = graph
        .nodes
        .iter()
        .filter(|n| is_stereotype(n, STEREOTYPE_UCA))
        .collect();

    let mut explained: HashSet<&str> = HashSet::new();
    for edge in &graph.edges {
        if edge.kind != "reference" {
            continue;
        }
        let source_is_scenario = nodes
            .get(edge.source.as_str())
            .map(|n| is_stereotype(n, STEREOTYPE_LOSS_SCENARIO))
            .unwrap_or(false);
        let target_is_uca = nodes
            .get(edge.target.as_str())
            .map(|n| is_stereotype(n, STEREOTYPE_UCA))
            .unwrap_or(false);
        if source_is_scenario && target_is_uca {
            explained.insert(edge.target.as_str());
        }
    }

    let mut findings = Vec::new();
    for uca in &ucas {
        if explained.contains(uca.id.as_str()) {
            continue;
        }
        let basis = format!(
            "computed over reference edges from LossScenario nodes; {} is a UCA not referenced by any LossScenario",
            uca.id
        );
        findings.push(UcaWithoutScenario {
            uca_id: uca.id.clone(),
            uca_name: name_of(&uca.id),
            basis,
        });
    }
    findings.sort_by(|a, b| a.uca_id.cmp(&b.uca_id));
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(name: &str) -> OkfRoot {
        let path = test_support::repo_root().join("sample/stpa").join(name);
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {}", path.display(), e));
        serde_json::from_str(&text).expect("fixture must parse")
    }

    /// The stage tokens of a report's method gaps, in the order the report names them.
    fn stages(report: &StpaReport) -> Vec<&str> {
        report
            .method_gaps
            .iter()
            .map(|gap| gap.stage.as_str())
            .collect()
    }

    /// A minimal model: graph nodes carrying the given stereotypes, graph edges naming their
    /// endpoints, and requirements carrying theirs. Every field the report reads is exercised.
    fn model(
        nodes: &[(&str, &[&str])],
        edges: &[(&str, &str, &str, &str)],
        requirements: &[(&str, &[&str])],
    ) -> OkfRoot {
        let graph_nodes: Vec<serde_json::Value> = nodes
            .iter()
            .map(|(id, stereotypes)| {
                serde_json::json!({ "id": id, "kind": "block", "name": id, "stereotypes": stereotypes })
            })
            .collect();
        let graph_edges: Vec<serde_json::Value> = edges
            .iter()
            .map(|(source, target, kind, label)| {
                serde_json::json!({ "source": source, "target": target, "kind": kind, "label": label })
            })
            .collect();
        let reqs: Vec<serde_json::Value> = requirements
            .iter()
            .map(|(id, stereotypes)| {
                serde_json::json!({ "id": id, "name": id, "kind": "requirement", "stereotypes": stereotypes })
            })
            .collect();
        serde_json::from_value(serde_json::json!({
            "project": "test",
            "structure": graph_nodes,
            "requirements": reqs,
            "graph": { "nodes": graph_nodes, "edges": graph_edges },
        }))
        .expect("test model must parse")
    }

    #[test]
    fn defective_model_fires_every_check() {
        let report = stpa_report(&load("fire-suppression-defective.json"));

        // Check 1: one control action, two of the four UCA types carried.
        assert_eq!(report.control_actions, 1);
        assert_eq!(report.ucas, 2);
        assert_eq!(report.unanalysed_actions.len(), 1);
        let finding = &report.unanalysed_actions[0];
        assert_eq!(finding.action_id, "ca-discharge");
        assert_eq!(finding.covered_types, vec!["not-provided", "provided"]);
        assert_eq!(
            finding.missing_types,
            vec![
                "wrong-timing-or-order",
                "stopped-too-soon-or-applied-too-long"
            ]
        );

        // Check 2: the controller issues a control action and has no feedback path.
        assert_eq!(report.feedback_gaps.len(), 1);
        assert_eq!(report.feedback_gaps[0].controller_id, "controller");
        assert_eq!(
            report.feedback_gaps[0].control_actions,
            vec!["ca-discharge"]
        );

        // Check 3: one hazard with no constraint, one constraint reaching no element.
        assert_eq!(report.hazards_without_constraints.len(), 1);
        assert_eq!(report.hazards_without_constraints[0].hazard_id, "haz-1");
        assert_eq!(report.constraints_without_elements.len(), 1);
        assert_eq!(report.constraints_without_elements[0].constraint_id, "sc-1");

        // Check 4: one UCA has no scenario (uca-p), the other is explained.
        assert_eq!(report.ucas_without_scenarios.len(), 1);
        assert_eq!(report.ucas_without_scenarios[0].uca_id, "uca-p");

        assert_eq!(report.total_findings, 5);
        assert!(report.basis.contains("does not perform STPA"));

        // Every stage was performed, so the gaps are real relational findings - the state where
        // a non-zero finding count is the honest thing to show.
        assert!(report.method_gaps.is_empty());
        assert_eq!(report.verdict(), StpaVerdict::GapsFound);
    }

    #[test]
    fn correct_model_is_silent() {
        let report = stpa_report(&load("fire-suppression-correct.json"));

        assert_eq!(report.control_actions, 1);
        assert_eq!(report.ucas, 4);
        assert_eq!(report.unanalysed_actions.len(), 0);
        assert_eq!(report.feedback_gaps.len(), 0);
        assert_eq!(report.hazards_without_constraints.len(), 0);
        assert_eq!(report.constraints_without_elements.len(), 0);
        assert_eq!(report.ucas_without_scenarios.len(), 0);
        assert_eq!(report.total_findings, 0);

        // The ONLY state in which a clean result may be shown: every stage was performed, and
        // the report carries the counts that prove the coverage was real.
        assert!(report.method_gaps.is_empty());
        assert_eq!(report.verdict(), StpaVerdict::Performed);
        assert!(report.is_clean());
        assert_eq!(report.losses, 1);
        assert!(report.hazards > 0 && report.constraints > 0 && report.control_actions > 0);
        let coverage = report.check_coverage();
        assert_eq!(coverage.unanalysed_actions, CheckState::Measured);
        assert_eq!(coverage.feedback_loops, CheckState::Measured);
        assert_eq!(coverage.hazards_without_constraints, CheckState::Measured);
        assert_eq!(coverage.constraints_without_elements, CheckState::Measured);
        assert_eq!(coverage.ucas_without_scenarios, CheckState::Measured);
    }

    #[test]
    fn a_model_without_a_graph_is_not_measured_rather_than_clean() {
        let root: OkfRoot = serde_json::from_str(r#"{"project":"empty"}"#).unwrap();
        let report = stpa_report(&root);
        assert_eq!(report.total_findings, 0);
        assert_eq!(report.control_actions, 0);
        assert!(report.basis.contains("no graph"));
        // Zero findings over a document with no graph at all is not a pass.
        assert_eq!(report.verdict(), StpaVerdict::NotStarted);
        assert!(!report.is_clean());
        assert!(!report.method_gaps.is_empty());
    }

    // ---------------------------------------------------------------------------------------
    // The method-coverage layer. Every check above is RELATIONAL: it tests a relationship
    // BETWEEN STPA elements. A model that carries no elements therefore returns zero findings,
    // and zero findings used to read as "complete". These tests pin the three states that make
    // that vacuous pass impossible.
    // ---------------------------------------------------------------------------------------

    #[test]
    fn a_model_with_no_hazards_is_not_started_not_clean() {
        // THE USER'S CASE. The model carries losses, a SystemConstraint, a control structure,
        // all four UCAs and all four loss scenarios - and no Hazard. Every relational check is
        // silent, so the old report said "complete". It must now say "not started".
        let report = stpa_report(&load("no-hazards.json"));

        assert_eq!(
            report.total_findings, 0,
            "the relational checks are all silent"
        );
        assert_eq!(report.hazards, 0);
        assert!(report.losses > 0 && report.control_actions > 0 && report.ucas > 0);

        assert_eq!(report.verdict(), StpaVerdict::NotStarted);
        assert!(
            !report.is_clean(),
            "zero findings over no hazards is not clean"
        );

        assert_eq!(stages(&report), vec!["hazards"]);
        let gap = &report.method_gaps[0];
        assert_eq!(gap.stage, "hazards");
        assert_eq!(gap.stage_name, "Hazard identification");
        assert!(
            gap.headline.contains("Hazard"),
            "the missing stage is named: {}",
            gap.headline
        );
        assert!(
            gap.basis.contains("declares none"),
            "the gap says what it was computed over: {}",
            gap.basis
        );
    }

    #[test]
    fn a_model_with_no_stpa_elements_names_every_foundational_stage() {
        let report = stpa_report(&load("no-stpa-elements.json"));
        assert_eq!(report.total_findings, 0);
        assert_eq!(report.verdict(), StpaVerdict::NotStarted);
        assert_eq!(
            stages(&report),
            vec!["losses", "hazards", "system-constraints"]
        );
    }

    #[test]
    fn each_method_stage_is_named_when_it_alone_is_missing() {
        // Stage 1: everything but a Loss.
        let no_losses = stpa_report(&model(
            &[
                ("haz-1", &["Hazard"]),
                ("controller", &["Controller"]),
                ("ca", &["ControlAction"]),
                ("uca", &["UnsafeControlAction", "not-provided"]),
            ],
            &[
                ("controller", "ca", "triggers", ""),
                ("uca", "ca", "reference", ""),
            ],
            &[("sc-1", &["SystemConstraint"])],
        ));
        assert_eq!(stages(&no_losses), vec!["losses"]);
        assert_eq!(no_losses.verdict(), StpaVerdict::NotStarted);

        // Stage 3: everything but a SystemConstraint.
        let no_constraints = stpa_report(&model(
            &[
                ("loss-1", &["Loss"]),
                ("haz-1", &["Hazard"]),
                ("controller", &["Controller"]),
                ("ca", &["ControlAction"]),
                ("uca", &["UnsafeControlAction", "not-provided"]),
            ],
            &[
                ("controller", "ca", "triggers", ""),
                ("uca", "ca", "reference", ""),
            ],
            &[],
        ));
        assert_eq!(stages(&no_constraints), vec!["system-constraints"]);

        // Stage 4: hazards, but nothing to control them with.
        let no_structure = stpa_report(&model(
            &[("loss-1", &["Loss"]), ("haz-1", &["Hazard"])],
            &[],
            &[("sc-1", &["SystemConstraint"])],
        ));
        assert_eq!(stages(&no_structure), vec!["control-structure"]);

        // Stage 5: a control structure, but the UCA enumeration was never done.
        let no_ucas = stpa_report(&model(
            &[
                ("loss-1", &["Loss"]),
                ("haz-1", &["Hazard"]),
                ("controller", &["Controller"]),
                ("ca", &["ControlAction"]),
            ],
            &[("controller", "ca", "triggers", "")],
            &[("sc-1", &["SystemConstraint"])],
        ));
        assert_eq!(stages(&no_ucas), vec!["unsafe-control-actions"]);
    }

    #[test]
    fn a_vacuous_report_can_never_read_as_a_pass() {
        // The vacuous case is structurally impossible, not handled by a template: the verdict is
        // a function of the method gaps, and the method gaps are a function of the elements the
        // model carries. Silence over nothing lands in NotStarted by construction.
        for name in ["no-hazards.json", "no-stpa-elements.json"] {
            let report = stpa_report(&load(name));
            assert_eq!(report.total_findings, 0, "{name}: the checks are silent");
            assert_ne!(
                report.verdict(),
                StpaVerdict::Performed,
                "{name}: zero findings AND zero elements measured must not be a pass"
            );
            assert_eq!(report.verdict(), StpaVerdict::NotStarted, "{name}");
            assert!(!report.is_clean(), "{name}");
            assert!(
                !report.method_gaps.is_empty(),
                "{name}: a missing stage must be named"
            );
        }

        // The implication that makes it structural, read over every fixture: Performed is only
        // reachable when every stage was measured over real elements.
        for name in [
            "no-hazards.json",
            "no-stpa-elements.json",
            "fire-suppression-defective.json",
            "fire-suppression-correct.json",
        ] {
            let report = stpa_report(&load(name));
            if report.verdict() == StpaVerdict::Performed {
                assert!(
                    report.losses > 0
                        && report.hazards > 0
                        && report.constraints > 0
                        && report.controllers > 0
                        && report.control_actions > 0
                        && report.ucas > 0,
                    "{name}: Performed over an unmeasured stage is impossible"
                );
            }
        }
    }

    #[test]
    fn a_check_with_no_subject_is_not_measurable_rather_than_clean() {
        // The hazard check has no subject in the no-hazards model: its empty result is
        // "nothing to measure", never "nothing wrong". Every other check does have a subject.
        let report = stpa_report(&load("no-hazards.json"));
        let coverage = report.check_coverage();
        assert_eq!(
            coverage.hazards_without_constraints,
            CheckState::NotMeasurable
        );
        assert_eq!(coverage.unanalysed_actions, CheckState::Measured);
        assert_eq!(coverage.feedback_loops, CheckState::Measured);
        assert_eq!(coverage.constraints_without_elements, CheckState::Measured);
        assert_eq!(coverage.ucas_without_scenarios, CheckState::Measured);

        // A model with nothing at all has no subject for any of the four document checks.
        let blank = stpa_report(&load("no-stpa-elements.json"));
        let coverage = blank.check_coverage();
        assert_eq!(coverage.unanalysed_actions, CheckState::NotMeasurable);
        assert_eq!(coverage.feedback_loops, CheckState::NotMeasurable);
        assert_eq!(
            coverage.hazards_without_constraints,
            CheckState::NotMeasurable
        );
        assert_eq!(
            coverage.constraints_without_elements,
            CheckState::NotMeasurable
        );
        assert_eq!(coverage.ucas_without_scenarios, CheckState::NotMeasurable);
    }
}
