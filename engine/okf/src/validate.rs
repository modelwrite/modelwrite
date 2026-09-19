// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::HashSet;

use serde::Serialize;

use crate::types::OkfRoot;

pub const NODE_KINDS: &[&str] = &[
    "activity",
    "actor",
    "block",
    "interface",
    "requirement",
    "signal",
    "state",
    "stateMachine",
    "usecase",
];

pub const EDGE_KINDS: &[&str] = &[
    "association",
    "behavior",
    "contains",
    "dependency",
    "effect",
    "generalization",
    "include",
    "part",
    "reference",
    "subject",
    "transition",
    "triggers",
    "uses",
];

/// Every declared element, paired with the section that declares it, for the cross-check
/// against the graph. The section name is carried so the warning can say where to look.
fn element_ids_by_section(root: &OkfRoot) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();
    for element in &root.structure {
        out.push(("structure element", element.id.clone()));
    }
    for element in &root.interfaces {
        out.push(("interface", element.id.clone()));
    }
    for element in &root.signals {
        out.push(("signal", element.id.clone()));
    }
    for requirement in &root.requirements {
        out.push(("requirement", requirement.id.clone()));
    }
    out
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidationReport {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn validate(root: &OkfRoot) -> ValidationReport {
    let mut report = ValidationReport {
        valid: true,
        errors: Vec::new(),
        warnings: Vec::new(),
    };

    if !root.okf.is_empty() && root.okf != "1.0" {
        report.errors.push(format!(
            "okf marker must be empty or 1.0, found {}",
            root.okf
        ));
    } else if root.okf.is_empty() {
        report
            .warnings
            .push("legacy OKF without version marker; exporters should emit okf 1.0".to_string());
    }
    if root.project.is_empty() {
        report.errors.push("project name is empty".to_string());
    }
    for reference in &root.references {
        let label = if reference.project.is_empty() {
            "(unnamed project)".to_string()
        } else {
            reference.project.clone()
        };
        if reference.project.is_empty() {
            report
                .errors
                .push("subsystem reference has an empty project".to_string());
        }
        if reference.revision.is_empty() {
            report.errors.push(format!(
                "subsystem reference to {} has an empty revision",
                label
            ));
        }
        if reference.role.is_empty() {
            report.errors.push(format!(
                "subsystem reference to {} has an empty role",
                label
            ));
        }
    }
    if root.graph.is_none() {
        report.errors.push("graph section is missing".to_string());
    }
    if root.state_machine.is_none() {
        report
            .errors
            .push("stateMachine section is missing".to_string());
    }

    let mut seen: HashSet<String> = HashSet::new();
    let mut add_id = |id: &str, section: &str, errors: &mut Vec<String>| {
        if id.is_empty() {
            errors.push(format!("{}: empty element id", section));
        } else if !seen.insert(id.to_string()) {
            errors.push(format!("duplicate element id {} in {}", id, section));
        }
    };
    for el in &root.structure {
        add_id(&el.id, "structure", &mut report.errors);
    }
    for el in &root.interfaces {
        add_id(&el.id, "interfaces", &mut report.errors);
    }
    for el in &root.signals {
        add_id(&el.id, "signals", &mut report.errors);
    }
    for r in &root.requirements {
        add_id(&r.id, "requirements", &mut report.errors);
        if r.req_id.trim().is_empty() {
            report
                .errors
                .push(format!("requirement {} has an empty reqId", r.id));
        }
    }
    if let Some(sm) = &root.state_machine {
        if sm.name.is_empty() {
            report
                .errors
                .push("stateMachine has an empty name".to_string());
        }
        for region in &sm.regions {
            for s in &region.states {
                add_id(&s.id, "stateMachine", &mut report.errors);
            }
        }
    }

    if let Some(graph) = &root.graph {
        if graph.nodes.is_empty() {
            report.errors.push("graph has no nodes".to_string());
        }
        let node_ids: HashSet<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
        for n in &graph.nodes {
            if !NODE_KINDS.contains(&n.kind.as_str()) {
                report
                    .errors
                    .push(format!("unknown node kind {} on node {}", n.kind, n.id));
            }
        }
        // Every element the document declares should have a node in the graph, because the
        // graph is how the platform reasons about relationships. When one is missing, the
        // element is INVISIBLE to every graph-based answer - coverage, traceability, impact -
        // while still appearing in the document, which is the quiet kind of inconsistency
        // this platform exists to prevent.
        //
        // The commonest cause is a RENAME: changing an element's id without updating the
        // graph leaves the node under the old name, and nothing else notices, because a
        // dangling EDGE endpoint is reported but an orphaned ELEMENT was not. A warning
        // rather than an error, because a document that predates this check must stay
        // usable - the same reasoning that governs dangling edge endpoints.
        for (section, id) in element_ids_by_section(root) {
            if !node_ids.contains(id.as_str()) {
                report.warnings.push(format!(
                    "{} {} has no node in the graph, so it is invisible to coverage and traceability",
                    section, id
                ));
            }
        }
        for e in &graph.edges {
            if !EDGE_KINDS.contains(&e.kind.as_str()) {
                report.errors.push(format!(
                    "unknown edge kind {} from {} to {}",
                    e.kind, e.source, e.target
                ));
            }
            // Unresolved endpoints are warnings, not errors: the reference corpus proves
            // that real exporter output can carry edges whose element was never emitted as
            // a node (two satisfy links in the coffee-machine export). The platform must
            // hold, gate and reason about such a model while reporting the defect loudly;
            // refusing the document would make the corpus unusable as the proof fixture.
            if !node_ids.contains(e.source.as_str()) {
                report.warnings.push(format!(
                    "dangling edge endpoint {} (source of a {} edge)",
                    e.source, e.kind
                ));
            }
            if !node_ids.contains(e.target.as_str()) {
                report.warnings.push(format!(
                    "dangling edge endpoint {} (target of a {} edge)",
                    e.target, e.kind
                ));
            }
        }
    }

    let sm = &root.summary;
    if let Some(graph) = &root.graph {
        if sm.graph_nodes as usize != graph.nodes.len()
            || sm.graph_edges as usize != graph.edges.len()
        {
            report.warnings.push(format!(
                "summary graph counts ({}/{}) do not match graph section ({}/{})",
                sm.graph_nodes,
                sm.graph_edges,
                graph.nodes.len(),
                graph.edges.len()
            ));
        }
    }
    if sm.blocks as usize != root.structure.len() {
        report.warnings.push(format!(
            "summary blocks {} do not match structure length {}",
            sm.blocks,
            root.structure.len()
        ));
    }
    if sm.requirements as usize != root.requirements.len() {
        report.warnings.push(format!(
            "summary requirements {} do not match requirements length {}",
            sm.requirements,
            root.requirements.len()
        ));
    }
    if sm.signals as usize != root.signals.len() {
        report.warnings.push(format!(
            "summary signals {} do not match signals length {}",
            sm.signals,
            root.signals.len()
        ));
    }
    if sm.activities as usize != root.activities.len() {
        report.warnings.push(format!(
            "summary activities {} do not match activities length {}",
            sm.activities,
            root.activities.len()
        ));
    }
    if sm.interfaces as usize != root.interfaces.len() {
        report.warnings.push(format!(
            "summary interfaces {} do not match interfaces length {}",
            sm.interfaces,
            root.interfaces.len()
        ));
    }

    report.valid = report.errors.is_empty();
    report
}
