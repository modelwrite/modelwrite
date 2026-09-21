// SPDX-License-Identifier: AGPL-3.0-or-later
//! The mw-analytics-schema@1 projection: a read-only view of one project and one commit as
//! the nine schema tables, plus the uniform metrics projection (every engine computation in
//! one row shape, each carrying its basis as data).
//!
//! This is the SERVER half of the projection. It mirrors `cli/src/analytics.rs` column for
//! column and note for note, because REST, CLI and MCP are transports for the SAME tables
//! and Package 7 asserts the rows are identical. Rows are built in the schema's snake_case
//! "at rest" naming; the JSON wire camelCase mapping lives in [`super::format`].

use std::cmp::Ordering;

use serde_json::{json, Value};

use crate::api::load_model;
use crate::store::{Commit, CommitProvenance, Store};

/// The schema version every response and export carries.
pub const SCHEMA_VERSION: &str = "mw-analytics-schema@1";

/// The nine tables, in stable order, with their columns in schema order (snake_case at rest).
pub const TABLES: &[(&str, &[&str])] = &[
    ("projects", &["project", "created_at"]),
    (
        "commits",
        &[
            "hash",
            "project",
            "branch",
            "parents",
            "author",
            "message",
            "committed_at",
            "okf_hash",
            "provenance_kind",
            "import_artifact_hash",
            "binding_id",
            "binding_version",
        ],
    ),
    (
        "elements",
        &[
            "project",
            "commit",
            "element_id",
            "section",
            "name",
            "kind",
            "stereotypes",
            "attributes",
            "documentation",
        ],
    ),
    (
        "relationships",
        &["project", "commit", "source", "target", "kind", "label"],
    ),
    (
        "requirements",
        &[
            "project",
            "commit",
            "element_id",
            "identifier",
            "text",
            "name",
            "kind",
            "covered",
            "covering_link_kinds",
        ],
    ),
    (
        "trace_links",
        &[
            "project",
            "commit",
            "requirement_id",
            "element_id",
            "link_kind",
            "source_id",
            "target_id",
        ],
    ),
    (
        "metrics",
        &[
            "project",
            "commit",
            "metric_id",
            "value",
            "value_state",
            "unit",
            "trust",
            "basis_element_count",
            "basis_relationship_count",
            "constructs_not_carried",
            "basis_note",
            "engine_version",
            "evidence_hash",
        ],
    ),
    (
        "metric_definitions",
        &["id", "name", "description", "depends_on", "status"],
    ),
    (
        "import_losses",
        &[
            "import_artifact_hash",
            "project",
            "binding_id",
            "binding_version",
            "construct",
            "subject",
            "severity",
            "note",
        ],
    ),
];

/// The columns of one table, or an empty slice for an unknown table name.
pub fn table_columns(name: &str) -> &'static [&'static str] {
    TABLES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, c)| *c)
        .unwrap_or(&[])
}

/// Whether a name is one of the nine schema tables.
pub fn is_table(name: &str) -> bool {
    TABLES.iter().any(|(n, _)| *n == name)
}

/// The key columns a table's rows are ordered by (deterministic output, design decision 5).
/// `import_losses` has a CONSTANT key for one commit (a single import), so its natural
/// mapping order IS the sorted order and it never needs materialising to sort.
fn sort_keys(name: &str) -> &'static [&'static str] {
    match name {
        "projects" => &["project"],
        "commits" => &["hash"],
        "elements" => &["commit", "element_id"],
        "relationships" => &["commit", "source", "target", "kind", "label"],
        "requirements" => &["commit", "element_id"],
        "trace_links" => &[
            "commit",
            "requirement_id",
            "link_kind",
            "source_id",
            "target_id",
        ],
        "metrics" => &["commit", "metric_id"],
        "metric_definitions" => &["id"],
        "import_losses" => &["import_artifact_hash", "project"],
        _ => &[],
    }
}

fn compare_rows(a: &Value, b: &Value, keys: &[&str]) -> Ordering {
    for k in keys {
        let av = a.get(k).and_then(Value::as_str).unwrap_or("");
        let bv = b.get(k).and_then(Value::as_str).unwrap_or("");
        match av.cmp(bv) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    Ordering::Equal
}

/// Sort rows into the table's deterministic key order.
pub fn sort_rows(name: &str, rows: &mut [Value]) {
    let keys = sort_keys(name);
    if !keys.is_empty() {
        rows.sort_by(|a, b| compare_rows(a, b, keys));
    }
}

// ---------------------------------------------------------------------------
// Projection: the model-backed tables for one commit.
// ---------------------------------------------------------------------------

/// Load the OKF document behind a commit, mapping the store error to a plain string.
pub fn load(store: &dyn Store, project: &str, hash: &str) -> Result<okf::types::OkfRoot, String> {
    load_model(store, project, hash).map_err(|e| e.to_string())
}

/// The `projects` table for one project (always one row).
pub fn projects_table(store: &dyn Store, project: &str) -> Result<Vec<Value>, String> {
    let row = store
        .project(project)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("project {} not found", project))?;
    Ok(vec![
        json!({ "project": row.name, "created_at": row.created_at }),
    ])
}

/// The `commits` table for one commit (always one row).
pub fn commits_table(commit: &Commit) -> Vec<Value> {
    let (kind, artifact_hash, binding_id, binding_version) = provenance_fields(&commit.provenance);
    vec![json!({
        "hash": commit.hash,
        "project": commit.project,
        "branch": commit.branch,
        "parents": commit.parents,
        "author": commit.author,
        "message": commit.message,
        "committed_at": commit.created_at,
        "okf_hash": commit.okf_hash,
        "provenance_kind": kind,
        "import_artifact_hash": artifact_hash,
        "binding_id": binding_id,
        "binding_version": binding_version,
    })]
}

fn provenance_fields(
    provenance: &CommitProvenance,
) -> (String, Option<String>, Option<String>, Option<String>) {
    match provenance {
        CommitProvenance::Authored => ("authored".to_string(), None, None, None),
        CommitProvenance::Imported {
            artifact_hash,
            binding_id,
            binding_version,
            ..
        } => (
            "imported".to_string(),
            Some(artifact_hash.clone()),
            Some(binding_id.clone()),
            Some(binding_version.clone()),
        ),
        CommitProvenance::Accepted { .. } => ("accepted".to_string(), None, None, None),
        CommitProvenance::Unknown => ("unknown".to_string(), None, None, None),
    }
}

/// The `elements` table: one row per element across structure/interfaces/signals.
pub fn elements_table(project: &str, commit: &Commit, root: &okf::types::OkfRoot) -> Vec<Value> {
    let mut rows = Vec::new();
    push_elements(&mut rows, project, commit, "structure", &root.structure);
    push_elements(&mut rows, project, commit, "interfaces", &root.interfaces);
    push_elements(&mut rows, project, commit, "signals", &root.signals);
    rows
}

fn push_elements(
    rows: &mut Vec<Value>,
    project: &str,
    commit: &Commit,
    section: &str,
    elements: &[okf::types::Element],
) {
    for e in elements {
        rows.push(json!({
            "project": project,
            "commit": commit.hash,
            "element_id": e.id,
            "section": section,
            "name": e.name,
            "kind": e.kind,
            "stereotypes": e.stereotypes,
            "attributes": e.attributes,
            "documentation": e.documentation,
        }));
    }
}

/// The `relationships` table: one row per graph edge.
pub fn relationships_table(
    project: &str,
    commit: &Commit,
    root: &okf::types::OkfRoot,
) -> Vec<Value> {
    let mut rows = Vec::new();
    if let Some(graph) = &root.graph {
        for edge in &graph.edges {
            rows.push(json!({
                "project": project,
                "commit": commit.hash,
                "source": edge.source,
                "target": edge.target,
                "kind": edge.kind,
                "label": edge.label,
            }));
        }
    }
    rows
}

/// The `requirements` table: one row per requirement, joined with the engine's coverage.
pub fn requirements_table(
    project: &str,
    commit: &Commit,
    root: &okf::types::OkfRoot,
) -> Vec<Value> {
    let mut covering: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    if let Some(graph) = &root.graph {
        let req_ids: std::collections::HashSet<&str> =
            root.requirements.iter().map(|r| r.id.as_str()).collect();
        for edge in &graph.edges {
            if edge.kind != "dependency" {
                continue;
            }
            let req_id = match edge.label.as_str() {
                "Satisfy" | "Refine" | "Verify" if req_ids.contains(edge.target.as_str()) => {
                    Some(edge.target.as_str())
                }
                "Allocate" if req_ids.contains(edge.target.as_str()) => Some(edge.target.as_str()),
                "Allocate" if req_ids.contains(edge.source.as_str()) => Some(edge.source.as_str()),
                _ => None,
            };
            if let Some(req_id) = req_id {
                covering
                    .entry(req_id.to_string())
                    .or_default()
                    .push(edge.label.clone());
            }
        }
    }

    let uncovered: std::collections::HashSet<String> = uncovered_ids(root);
    let mut rows = Vec::new();
    for r in &root.requirements {
        let mut kinds = covering.remove(&r.id).unwrap_or_default();
        kinds.sort();
        kinds.dedup();
        rows.push(json!({
            "project": project,
            "commit": commit.hash,
            "element_id": r.id,
            "identifier": r.req_id,
            "text": r.req_text,
            "name": r.name,
            "kind": r.kind,
            "covered": !uncovered.contains(r.id.as_str()),
            "covering_link_kinds": kinds,
        }));
    }
    rows
}

/// The requirement ids the engine's coverage reports as uncovered (empty when there is no
/// graph, where every requirement reads uncovered, mirroring the gate evidence).
fn uncovered_ids(root: &okf::types::OkfRoot) -> std::collections::HashSet<String> {
    if root.graph.is_some() {
        graph::requirement_coverage(root)
            .uncovered
            .into_iter()
            .collect()
    } else {
        root.requirements.iter().map(|r| r.id.clone()).collect()
    }
}

/// The `trace_links` table: one row per coverage (dependency Satisfy/Refine/Verify/Allocate)
/// edge, with the requirement endpoint resolved.
pub fn trace_links_table(project: &str, commit: &Commit, root: &okf::types::OkfRoot) -> Vec<Value> {
    let mut rows = Vec::new();
    if let Some(graph) = &root.graph {
        let req_ids: std::collections::HashSet<&str> =
            root.requirements.iter().map(|r| r.id.as_str()).collect();
        for edge in &graph.edges {
            if edge.kind != "dependency" {
                continue;
            }
            let link_kind = edge.label.as_str();
            let (requirement_id, element_id) = match link_kind {
                "Satisfy" | "Refine" | "Verify" if req_ids.contains(edge.target.as_str()) => {
                    (edge.target.as_str(), edge.source.as_str())
                }
                "Allocate" if req_ids.contains(edge.target.as_str()) => {
                    (edge.target.as_str(), edge.source.as_str())
                }
                "Allocate" if req_ids.contains(edge.source.as_str()) => {
                    (edge.source.as_str(), edge.target.as_str())
                }
                _ => continue,
            };
            rows.push(json!({
                "project": project,
                "commit": commit.hash,
                "requirement_id": requirement_id,
                "element_id": element_id,
                "link_kind": link_kind,
                "source_id": edge.source,
                "target_id": edge.target,
            }));
        }
    }
    rows
}

/// The `metric_definitions` table: generated from the engine, never hand-written.
pub fn metric_definitions_table() -> Vec<Value> {
    analytics::metrics::metric_definitions()
        .into_iter()
        .map(|d| {
            json!({
                "id": d.id,
                "name": d.name,
                "description": d.description,
                "depends_on": d.depends_on,
                "status": d.status,
            })
        })
        .collect()
}

/// The number of constructs an import could not carry into the model the metric was computed
/// over: the import record's content-losses count for an imported commit, 0 otherwise.
pub fn constructs_not_carried(
    store: &dyn Store,
    project: &str,
    commit: &Commit,
) -> Result<i64, String> {
    let artifact_hash = match &commit.provenance {
        CommitProvenance::Imported { artifact_hash, .. } => artifact_hash.as_str(),
        _ => return Ok(0),
    };
    let report = load_loss_report(store, project, artifact_hash)?;
    Ok(report.content_losses().len() as i64)
}

/// Load and parse the loss report an import committed with, keyed by (project, artifact_hash).
fn load_loss_report(
    store: &dyn Store,
    project: &str,
    artifact_hash: &str,
) -> Result<binding::LossReport, String> {
    let record = store
        .import_report(project, artifact_hash)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("import {} for project {} not found", artifact_hash, project))?;
    serde_json::from_str(&record.loss_report).map_err(|e| format!("corrupt loss report: {}", e))
}

/// One `import_losses` row for a loss-report mapping. One row per mapping - nothing dropped.
pub fn loss_row(
    artifact_hash: &str,
    project: &str,
    binding_id: &str,
    binding_version: &str,
    mapping: &binding::Mapping,
) -> Value {
    json!({
        "import_artifact_hash": artifact_hash,
        "project": project,
        "binding_id": binding_id,
        "binding_version": binding_version,
        "construct": construct_of(&mapping.subject),
        "subject": mapping.subject,
        "severity": severity_of(&mapping.verdict),
        "note": mapping.note,
    })
}

/// The import record + loss report a commit's import carried, when the commit is an import.
pub struct ImportLosses {
    pub artifact_hash: String,
    pub binding_id: String,
    pub binding_version: String,
    pub report: binding::LossReport,
}

/// Resolve the loss report for an imported commit. `None` for every other commit kind.
pub fn import_losses_for(
    store: &dyn Store,
    project: &str,
    commit: &Commit,
) -> Result<Option<ImportLosses>, String> {
    let artifact_hash = match &commit.provenance {
        CommitProvenance::Imported { artifact_hash, .. } => artifact_hash.clone(),
        _ => return Ok(None),
    };
    let record = store
        .import_report(project, &artifact_hash)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("import {} for project {} not found", artifact_hash, project))?;
    let report: binding::LossReport = serde_json::from_str(&record.loss_report)
        .map_err(|e| format!("corrupt loss report: {}", e))?;
    Ok(Some(ImportLosses {
        artifact_hash,
        binding_id: record.binding_id,
        binding_version: record.binding_version,
        report,
    }))
}

pub fn construct_of(subject: &str) -> &str {
    subject.split_whitespace().next().unwrap_or(subject)
}

fn severity_of(verdict: &binding::MappingVerdict) -> &'static str {
    match verdict {
        binding::MappingVerdict::Exact => "exact",
        binding::MappingVerdict::Lossy => "lossy",
        binding::MappingVerdict::Unmappable => "unmappable",
    }
}

// ---------------------------------------------------------------------------
// Metrics: the engine's five computations projected into one row shape, each with basis.
// ---------------------------------------------------------------------------

/// All 25 engine metrics for one commit, each carrying its basis. graph/coverage come from the
/// graph engine; compliance is the model's own specification (the portfolio of one); cost reads
/// no dataset (every requirement uncosted, never zero); the gate is the model gated against
/// itself (no external reference), so round-trip counts are 0.
pub fn metric_rows(
    project: &str,
    commit: &Commit,
    root: &okf::types::OkfRoot,
    constructs_not_carried: i64,
) -> Vec<Value> {
    let evidence_hash = commit.okf_hash.clone();
    let engine_version = env!("CARGO_PKG_VERSION").to_string();

    let has_graph = root.graph.is_some();
    let (node_count, edge_count, isolated_count, component_count) = if has_graph {
        let s = graph::graph_stats(root);
        (
            s.node_count as i64,
            s.edge_count as i64,
            s.isolated.len() as i64,
            s.component_count as i64,
        )
    } else {
        (0, 0, 0, 0)
    };

    let (total, satisfied, refined, verified, allocated, covered, uncovered_count) = if has_graph {
        let c = graph::requirement_coverage(root);
        (
            c.total as i64,
            c.satisfied as i64,
            c.refined as i64,
            c.verified as i64,
            c.allocated as i64,
            c.covered as i64,
            c.uncovered.len() as i64,
        )
    } else {
        let n = root.requirements.len() as i64;
        (n, 0, 0, 0, 0, 0, n)
    };
    let coverage_edges = satisfied + refined + verified + allocated;

    // The self-specification reading: the model's own requirements are the specification.
    let spec: Vec<&str> = root.requirements.iter().map(|r| r.id.as_str()).collect();
    let compliance = analytics::portfolio_report(&spec, &[root]);
    let model = &compliance.models[0];
    let specification_size = compliance.specification_size as i64;
    let covered_count = model.covered_count() as i64;
    let uncovered_req_count = model.uncovered_count() as i64;
    let unknown_count = model.unknown_count() as i64;

    // No cost dataset: every requirement reads UNCOSTED, never zero.
    let registry = analytics::Registry::new();
    let costs = analytics::cost_by_requirement(&[root], &registry, &[]).unwrap_or_default();
    let costed_count = costs.iter().filter(|c| !c.cost.is_uncosted()).count() as i64;
    let uncosted_count = costs.len() as i64 - costed_count;
    let requirement_count = root.requirements.len() as i64;

    // The self-gate: no external reference, so the round trip is trivially equal and the
    // verdict is the model's own validity plus graph connectivity.
    let gate = gate::run(root, root, false);
    let evidence = &gate.evidence;
    let validation_errors = evidence["validationErrors"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0) as i64;
    let validation_warnings = evidence["validationWarnings"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0) as i64;
    let gate_passed = if gate.passed { 1 } else { 0 };

    let element_count = (root.structure.len() + root.interfaces.len() + root.signals.len()) as i64;

    let mut rows = Vec::with_capacity(25);
    for (id, value) in [
        ("graph.node_count", node_count),
        ("graph.edge_count", edge_count),
        ("graph.isolated_count", isolated_count),
        ("graph.component_count", component_count),
    ] {
        rows.push(metric(
            project,
            commit,
            id,
            value,
            "count",
            node_count,
            edge_count,
            constructs_not_carried,
            &format!(
                "graph stats over {} nodes and {} edges",
                node_count, edge_count
            ),
            &engine_version,
            &evidence_hash,
        ));
    }
    for (id, value) in [
        ("coverage.total", total),
        ("coverage.satisfied", satisfied),
        ("coverage.refined", refined),
        ("coverage.verified", verified),
        ("coverage.allocated", allocated),
        ("coverage.covered", covered),
        ("coverage.uncovered", uncovered_count),
    ] {
        rows.push(metric(
            project,
            commit,
            id,
            value,
            "count",
            total,
            coverage_edges,
            constructs_not_carried,
            &format!(
                "requirement coverage over {} requirements and {} coverage edges",
                total, coverage_edges
            ),
            &engine_version,
            &evidence_hash,
        ));
    }
    for (id, value) in [
        ("compliance.specification_size", specification_size),
        ("compliance.covered_count", covered_count),
        ("compliance.uncovered_count", uncovered_req_count),
        ("compliance.unknown_count", unknown_count),
    ] {
        rows.push(metric(
            project,
            commit,
            id,
            value,
            "count",
            specification_size,
            0,
            constructs_not_carried,
            "self-specification: the model's own requirements",
            &engine_version,
            &evidence_hash,
        ));
    }
    for (id, value) in [
        ("cost.costed_count", costed_count),
        ("cost.uncosted_count", uncosted_count),
    ] {
        rows.push(metric(
            project,
            commit,
            id,
            value,
            "count",
            requirement_count,
            0,
            constructs_not_carried,
            "no cost dataset supplied; every requirement is uncosted",
            &engine_version,
            &evidence_hash,
        ));
    }
    for (id, value, unit) in [
        ("gate.passed", gate_passed, "bool"),
        ("gate.validation_errors", validation_errors, "count"),
        ("gate.validation_warnings", validation_warnings, "count"),
        ("gate.roundtrip.missing_elements", 0, "count"),
        ("gate.roundtrip.extra_elements", 0, "count"),
        ("gate.roundtrip.missing_edges", 0, "count"),
        ("gate.roundtrip.extra_edges", 0, "count"),
        ("gate.roundtrip.changed_attributes", 0, "count"),
    ] {
        rows.push(metric(
            project,
            commit,
            id,
            value,
            unit,
            element_count,
            edge_count,
            constructs_not_carried,
            "self-gate (no reference): the model gated against itself",
            &engine_version,
            &evidence_hash,
        ));
    }
    rows
}

#[allow(clippy::too_many_arguments)]
fn metric(
    project: &str,
    commit: &Commit,
    id: &str,
    value: i64,
    unit: &str,
    basis_element_count: i64,
    basis_relationship_count: i64,
    constructs_not_carried: i64,
    basis_note: &str,
    engine_version: &str,
    evidence_hash: &str,
) -> Value {
    json!({
        "project": project,
        "commit": commit.hash,
        "metric_id": id,
        "value": value,
        "value_state": "present",
        "unit": unit,
        "trust": "measured",
        "basis_element_count": basis_element_count,
        "basis_relationship_count": basis_relationship_count,
        "constructs_not_carried": constructs_not_carried,
        "basis_note": basis_note,
        "engine_version": engine_version,
        "evidence_hash": evidence_hash,
    })
}
