// SPDX-License-Identifier: AGPL-3.0-or-later
//! The STPA / STAMP completeness checks.
//!
//! The platform does NOT perform STPA. It holds the authored analysis as a typed OKF graph
//! (the stereotype convention declared in `sample/stpa/stpa-vocabulary.json`) and CHECKS that
//! the analysis is complete and internally consistent. Every check below is a pure function of
//! the document and every finding names what was computed over and what the model did not carry
//! - the basis rule, applied to the safety argument.
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
    pub control_actions: usize,
    pub controllers: usize,
    pub controlled_processes: usize,
    pub feedback: usize,
    pub hazards: usize,
    pub constraints: usize,
    pub ucas: usize,
    pub loss_scenarios: usize,
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
            control_actions: 0,
            controllers: 0,
            controlled_processes: 0,
            feedback: 0,
            hazards: 0,
            constraints: 0,
            ucas: 0,
            loss_scenarios: 0,
            unanalysed_actions: Vec::new(),
            feedback_gaps: Vec::new(),
            hazards_without_constraints: Vec::new(),
            constraints_without_elements: Vec::new(),
            ucas_without_scenarios: Vec::new(),
            total_findings: 0,
            basis,
        }
    }
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

    StpaReport {
        control_actions: count(&graph.nodes, STEREOTYPE_CONTROL_ACTION),
        controllers: count(&graph.nodes, STEREOTYPE_CONTROLLER),
        controlled_processes: count(&graph.nodes, STEREOTYPE_PROCESS),
        feedback: count(&graph.nodes, STEREOTYPE_FEEDBACK),
        hazards: count(&graph.nodes, STEREOTYPE_HAZARD),
        constraints: root
            .requirements
            .iter()
            .filter(|r| is_requirement_stereotype(r, STEREOTYPE_CONSTRAINT))
            .count(),
        ucas: count(&graph.nodes, STEREOTYPE_UCA),
        loss_scenarios: count(&graph.nodes, STEREOTYPE_LOSS_SCENARIO),
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
    }

    #[test]
    fn a_model_without_a_graph_is_not_a_finding() {
        let root: OkfRoot = serde_json::from_str(r#"{"project":"empty"}"#).unwrap();
        let report = stpa_report(&root);
        assert_eq!(report.total_findings, 0);
        assert_eq!(report.control_actions, 0);
        assert!(report.basis.contains("no graph"));
    }
}
