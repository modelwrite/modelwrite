// SPDX-License-Identifier: AGPL-3.0-or-later
//! The "mw analytics" subcommand group: a read-only projection of a project's model and
//! measures into mw-analytics-schema@1's nine tables, delivered as JSON on stdout or as
//! CSV / NDJSON / Parquet files. Every metric travels with its basis as data (element count,
//! relationship count, constructs not carried, basis note, engine version, evidence hash):
//! a metric without a basis is a defect, and nothing here hands out a bare number.

use std::cmp::Ordering;
use std::path::Path;
use std::sync::Arc;

use arrow::array::{
    ArrayRef, BooleanArray, Float64Array, Int64Array, ListArray, StringArray, StructArray,
};
use arrow::buffer::OffsetBuffer;
use arrow::datatypes::{DataType, Field, Fields, Schema};
use arrow::record_batch::RecordBatch;
use serde_json::{json, Value};

use server::store::{Commit, CommitProvenance, Store};

/// The schema version every result and export carries.
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

pub fn table_columns(name: &str) -> &'static [&'static str] {
    TABLES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, c)| *c)
        .unwrap_or(&[])
}

/// One table of the projection: a name, its ordered columns, and its rows.
#[derive(Debug, Clone)]
pub struct Table {
    pub name: &'static str,
    pub columns: &'static [&'static str],
    pub rows: Vec<Value>,
}

fn table(name: &'static str, mut rows: Vec<Value>) -> Table {
    let columns = table_columns(name);
    let keys = sort_keys(name);
    if !keys.is_empty() {
        rows.sort_by(|a, b| compare_rows(a, b, keys));
    }
    Table {
        name,
        columns,
        rows,
    }
}

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

// ---------------------------------------------------------------------------
// Projection: the nine tables for one commit.
// ---------------------------------------------------------------------------

/// Build all nine tables for one commit of one project. The model is loaded once and shared
/// across the five model tables, so the projection cannot drift between them.
pub fn build_commit_tables(
    store: &dyn Store,
    project: &str,
    commit: &Commit,
) -> Result<Vec<Table>, String> {
    let root = load_model(store, project, &commit.hash)?;
    let constructs_not_carried = constructs_not_carried(store, project, commit)?;

    Ok(vec![
        projects_table(store, project)?,
        commits_table(commit),
        elements_table(project, commit, &root),
        relationships_table(project, commit, &root),
        requirements_table(project, commit, &root),
        trace_links_table(project, commit, &root),
        metrics_table(project, commit, &root, constructs_not_carried),
        metric_definitions_table(),
        import_losses_table(store, project, commit)?,
    ])
}

fn load_model(store: &dyn Store, project: &str, hash: &str) -> Result<okf::types::OkfRoot, String> {
    server::api::load_model(store, project, hash).map_err(|e| e.to_string())
}

fn projects_table(store: &dyn Store, project: &str) -> Result<Table, String> {
    let row = store
        .project(project)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("project {} not found", project))?;
    Ok(table(
        "projects",
        vec![json!({ "project": row.name, "created_at": row.created_at })],
    ))
}

fn commits_table(commit: &Commit) -> Table {
    let (kind, artifact_hash, binding_id, binding_version) = provenance_fields(&commit.provenance);
    table(
        "commits",
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
        })],
    )
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

fn elements_table(project: &str, commit: &Commit, root: &okf::types::OkfRoot) -> Table {
    let mut rows = Vec::new();
    push_elements(&mut rows, project, commit, "structure", &root.structure);
    push_elements(&mut rows, project, commit, "interfaces", &root.interfaces);
    push_elements(&mut rows, project, commit, "signals", &root.signals);
    table("elements", rows)
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

fn relationships_table(project: &str, commit: &Commit, root: &okf::types::OkfRoot) -> Table {
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
    table("relationships", rows)
}

fn requirements_table(project: &str, commit: &Commit, root: &okf::types::OkfRoot) -> Table {
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
    table("requirements", rows)
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

fn trace_links_table(project: &str, commit: &Commit, root: &okf::types::OkfRoot) -> Table {
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
    table("trace_links", rows)
}

fn metric_definitions_table() -> Table {
    let rows = analytics::metrics::metric_definitions()
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
        .collect();
    table("metric_definitions", rows)
}

/// The number of constructs an import could not carry into the model the metric was computed
/// over: the import record's content-losses count for an imported commit, 0 otherwise.
fn constructs_not_carried(
    store: &dyn Store,
    project: &str,
    commit: &Commit,
) -> Result<i64, String> {
    let artifact_hash = match &commit.provenance {
        CommitProvenance::Imported { artifact_hash, .. } => artifact_hash.as_str(),
        _ => return Ok(0),
    };
    let record = store
        .import_report(project, artifact_hash)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("import {} for project {} not found", artifact_hash, project))?;
    let report: binding::LossReport = serde_json::from_str(&record.loss_report)
        .map_err(|e| format!("corrupt loss report: {}", e))?;
    Ok(report.content_losses().len() as i64)
}

fn import_losses_table(store: &dyn Store, project: &str, commit: &Commit) -> Result<Table, String> {
    let artifact_hash = match &commit.provenance {
        CommitProvenance::Imported { artifact_hash, .. } => artifact_hash.as_str(),
        _ => return Ok(table("import_losses", Vec::new())),
    };
    let record = store
        .import_report(project, artifact_hash)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("import {} for project {} not found", artifact_hash, project))?;
    let report: binding::LossReport = serde_json::from_str(&record.loss_report)
        .map_err(|e| format!("corrupt loss report: {}", e))?;
    let mut rows = Vec::new();
    for mapping in &report.mappings {
        rows.push(json!({
            "import_artifact_hash": record.artifact_hash,
            "project": project,
            "binding_id": record.binding_id,
            "binding_version": record.binding_version,
            "construct": construct_of(&mapping.subject),
            "subject": mapping.subject,
            "severity": severity_of(&mapping.verdict),
            "note": mapping.note,
        }));
    }
    Ok(table("import_losses", rows))
}

fn construct_of(subject: &str) -> &str {
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

fn metrics_table(
    project: &str,
    commit: &Commit,
    root: &okf::types::OkfRoot,
    constructs_not_carried: i64,
) -> Table {
    table(
        "metrics",
        metric_rows(project, commit, root, constructs_not_carried),
    )
}

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

// ---------------------------------------------------------------------------
// Command entry points (offline path). These print data to stdout and diagnostics to stderr.
// ---------------------------------------------------------------------------

fn print_json(value: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).expect("result serialises")
    );
}

pub fn offline_schema() -> Result<(), String> {
    let tables: Vec<Value> = TABLES
        .iter()
        .map(|(name, columns)| json!({ "name": name, "columns": columns }))
        .collect();
    let metric_definitions: Vec<Value> = analytics::metrics::metric_definitions()
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
        .collect();
    print_json(&json!({
        "schemaVersion": SCHEMA_VERSION,
        "tables": tables,
        "metricDefinitions": metric_definitions,
    }));
    Ok(())
}

pub fn offline_tables(
    store: &dyn Store,
    project: &str,
    commit: Option<&str>,
    branch: Option<&str>,
) -> Result<(), String> {
    let commit = resolve_commit(store, project, commit, branch)?;
    let tables = build_commit_tables(store, project, &commit)?;
    let rows: Vec<Value> = tables
        .iter()
        .map(|t| json!({ "name": t.name, "rowCount": t.rows.len() }))
        .collect();
    print_json(&json!({
        "schemaVersion": SCHEMA_VERSION,
        "project": project,
        "commit": commit.hash,
        "tables": rows,
    }));
    Ok(())
}

pub fn offline_metrics(
    store: &dyn Store,
    project: &str,
    commit: Option<&str>,
    branch: Option<&str>,
    format: &str,
) -> Result<(), String> {
    let commit = resolve_commit(store, project, commit, branch)?;
    let root = load_model(store, project, &commit.hash)?;
    let cnc = constructs_not_carried(store, project, &commit)?;
    let rows = metric_rows(project, &commit, &root, cnc);
    emit(
        &json!({ "schemaVersion": SCHEMA_VERSION, "project": project, "commit": commit.hash }),
        table("metrics", rows),
        format,
    )
}

pub fn offline_trend(
    store: &dyn Store,
    project: &str,
    metric: &str,
    branch: &str,
    from: Option<&str>,
    to: Option<&str>,
    format: &str,
) -> Result<(), String> {
    let commits = store
        .commits_on(project, branch)
        .map_err(|e| e.to_string())?;
    let commits = slice_range(&commits, from, to)?;
    let mut rows = Vec::new();
    for commit in &commits {
        let root = load_model(store, project, &commit.hash)?;
        let cnc = constructs_not_carried(store, project, commit)?;
        if let Some(row) = metric_rows(project, commit, &root, cnc)
            .into_iter()
            .find(|r| r.get("metric_id").and_then(Value::as_str) == Some(metric))
        {
            rows.push(row);
        }
    }
    if rows.is_empty() {
        return Err(format!(
            "metric {} produced no rows across branch {}",
            metric, branch
        ));
    }
    emit(
        &json!({ "schemaVersion": SCHEMA_VERSION, "project": project, "branch": branch, "metric": metric }),
        table("metrics", rows),
        format,
    )
}

/// Slice a commit list to the [from, to] range, both inclusive, by hash in commit order.
fn slice_range(
    commits: &[Commit],
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Vec<Commit>, String> {
    let from_idx = match from {
        Some(h) => commits
            .iter()
            .position(|c| c.hash == h)
            .ok_or_else(|| format!("commit {} not found on this branch", h))?,
        None => 0,
    };
    let to_idx = match to {
        Some(h) => commits
            .iter()
            .position(|c| c.hash == h)
            .ok_or_else(|| format!("commit {} not found on this branch", h))?,
        None => commits.len() - 1,
    };
    if from_idx > to_idx {
        return Err("the from commit is after the to commit".to_string());
    }
    Ok(commits[from_idx..=to_idx].to_vec())
}

pub fn offline_export(
    store: &dyn Store,
    project: &str,
    commit: Option<&str>,
    branch: Option<&str>,
    all_commits: bool,
    format: &str,
    out: &Path,
) -> Result<(), String> {
    if !matches!(format, "csv" | "ndjson" | "parquet") {
        return Err("analytics export --format must be csv, ndjson or parquet".to_string());
    }
    let commits = resolve_commits(store, project, commit, branch, all_commits)?;
    if commits.is_empty() {
        return Err(format!("no commits to export for project {}", project));
    }
    std::fs::create_dir_all(out).map_err(|e| format!("cannot create {}: {}", out.display(), e))?;

    let mut per_commit: Vec<(String, Vec<Table>)> = Vec::new();
    for commit in &commits {
        let tables = build_commit_tables(store, project, commit)?;
        per_commit.push((commit.hash.clone(), tables));
    }

    let files = write_export(format, out, project, &per_commit)?;
    let files: Vec<Value> = files
        .iter()
        .map(|(name, rows)| json!({ "name": name, "rowCount": rows }))
        .collect();
    print_json(&json!({
        "schemaVersion": SCHEMA_VERSION,
        "project": project,
        "format": format,
        "out": out,
        "tables": files,
    }));
    Ok(())
}

/// Write one export tree from the per-commit tables: CSV and NDJSON merge every commit's rows
/// per table in commit order, Parquet is partitioned by table/project/commit. Both transports
/// call this, so a --db export and a --server export of the same commits are byte-identical
/// files. Returns each table's merged row count, in schema order.
pub fn write_export(
    format: &str,
    out: &Path,
    project: &str,
    per_commit: &[(String, Vec<Table>)],
) -> Result<Vec<(&'static str, usize)>, String> {
    std::fs::create_dir_all(out).map_err(|e| format!("cannot create {}: {}", out.display(), e))?;

    let names: Vec<&'static str> = TABLES.iter().map(|(n, _)| *n).collect();
    let mut merged: Vec<Table> = Vec::new();
    for &name in &names {
        let mut rows = Vec::new();
        for (_, tables) in per_commit {
            if let Some(t) = tables.iter().find(|t| t.name == name) {
                rows.extend(t.rows.clone());
            }
        }
        merged.push(Table {
            name,
            columns: table_columns(name),
            rows,
        });
    }

    match format {
        "csv" => {
            for t in &merged {
                write_csv(&out.join(format!("{}.csv", t.name)), t)?;
            }
        }
        "ndjson" => {
            for t in &merged {
                write_ndjson(&out.join(format!("{}.ndjson", t.name)), t)?;
            }
        }
        "parquet" => {
            for (commit, tables) in per_commit {
                for t in tables {
                    let dir = out
                        .join(t.name)
                        .join(format!("project={}", project))
                        .join(format!("commit={}", commit));
                    std::fs::create_dir_all(&dir)
                        .map_err(|e| format!("cannot create {}: {}", dir.display(), e))?;
                    write_parquet(&dir.join("part-0.parquet"), t)?;
                }
            }
        }
        _ => unreachable!(),
    }

    Ok(merged.iter().map(|t| (t.name, t.rows.len())).collect())
}

fn resolve_commit(
    store: &dyn Store,
    project: &str,
    commit: Option<&str>,
    branch: Option<&str>,
) -> Result<Commit, String> {
    let hash = match commit {
        Some(h) => h.to_string(),
        None => {
            let branch = branch.unwrap_or("main");
            store
                .branch_tip(project, branch)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("branch {} not found", branch))?
        }
    };
    store
        .commit(project, &hash)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("commit {} not found", hash))
}

fn resolve_commits(
    store: &dyn Store,
    project: &str,
    commit: Option<&str>,
    branch: Option<&str>,
    all_commits: bool,
) -> Result<Vec<Commit>, String> {
    if let Some(h) = commit {
        let c = store
            .commit(project, h)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("commit {} not found", h))?;
        return Ok(vec![c]);
    }
    if all_commits {
        let mut seen = std::collections::BTreeSet::new();
        let mut out = Vec::new();
        for (name, _) in store.list_branches(project).map_err(|e| e.to_string())? {
            for c in store
                .commits_on(project, &name)
                .map_err(|e| e.to_string())?
            {
                if seen.insert(c.hash.clone()) {
                    out.push(c);
                }
            }
        }
        out.sort_by(|a, b| a.hash.cmp(&b.hash));
        return Ok(out);
    }
    let branch = branch.unwrap_or("main");
    store.commits_on(project, branch).map_err(|e| e.to_string())
}

/// Emit a table as JSON (pretty, wrapped in the envelope with the rows under "metrics") or as
/// CSV. Data goes to stdout; diagnostics never share it.
fn emit(envelope: &Value, t: Table, format: &str) -> Result<(), String> {
    match format {
        "json" => {
            let mut obj = envelope.as_object().cloned().unwrap_or_default();
            obj.insert("metrics".to_string(), Value::Array(t.rows));
            print_json(&Value::Object(obj));
            Ok(())
        }
        "csv" => {
            print!("{}", to_csv(&t));
            Ok(())
        }
        other => Err(format!(
            "analytics --format must be json or csv, got {}",
            other
        )),
    }
}

// ---------------------------------------------------------------------------
// Serialization.
// ---------------------------------------------------------------------------

pub fn to_csv(t: &Table) -> String {
    let mut out = String::new();
    out.push_str(&t.columns.join(","));
    out.push('\n');
    for row in &t.rows {
        let cells: Vec<String> = t.columns.iter().map(|c| csv_cell(&row[c])).collect();
        out.push_str(&cells.join(","));
        out.push('\n');
    }
    out
}

fn csv_cell(value: &Value) -> String {
    let s = match value {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

pub fn to_ndjson(t: &Table) -> String {
    let mut out = String::new();
    for row in &t.rows {
        out.push_str(&serde_json::to_string(row).expect("row serialises"));
        out.push('\n');
    }
    out
}

fn write_csv(path: &Path, t: &Table) -> Result<(), String> {
    std::fs::write(path, to_csv(t)).map_err(|e| format!("cannot write {}: {}", path.display(), e))
}

fn write_ndjson(path: &Path, t: &Table) -> Result<(), String> {
    std::fs::write(path, to_ndjson(t))
        .map_err(|e| format!("cannot write {}: {}", path.display(), e))
}

fn write_parquet(path: &Path, t: &Table) -> Result<(), String> {
    let mut fields = Vec::new();
    let mut columns = Vec::new();
    for col in t.columns {
        if *col == "project" || *col == "commit" {
            continue;
        }
        let (dtype, nullable) = arrow_type(t.name, col);
        let field = Field::new(*col, dtype.clone(), nullable);
        let array = build_column(&dtype, &t.rows, col)?;
        fields.push(field);
        columns.push(array);
    }
    let schema = Arc::new(Schema::new(fields));
    let batch = RecordBatch::try_new(schema.clone(), columns)
        .map_err(|e| format!("cannot build {} record batch: {}", t.name, e))?;
    let file = std::fs::File::create(path)
        .map_err(|e| format!("cannot create {}: {}", path.display(), e))?;
    let props = parquet::file::properties::WriterProperties::builder()
        .set_compression(parquet::basic::Compression::UNCOMPRESSED)
        .build();
    let mut writer = parquet::arrow::ArrowWriter::try_new(file, schema, Some(props))
        .map_err(|e| format!("cannot start {} parquet writer: {}", t.name, e))?;
    writer
        .write(&batch)
        .map_err(|e| format!("cannot write {} parquet: {}", t.name, e))?;
    writer
        .close()
        .map_err(|e| format!("cannot close {} parquet writer: {}", t.name, e))?;
    Ok(())
}

fn attributes_struct_type() -> DataType {
    DataType::Struct(Fields::from(vec![
        Field::new("name", DataType::Utf8, false),
        Field::new("type", DataType::Utf8, false),
        Field::new("aggregation", DataType::Utf8, false),
        Field::new("default", DataType::Utf8, false),
    ]))
}

/// The Arrow type of one column, matching spec/analytics/arrow/<table>.arrow.json.
fn arrow_type(table: &str, column: &str) -> (DataType, bool) {
    let utf8 = || (DataType::Utf8, false);
    let utf8_nullable = || (DataType::Utf8, true);
    let list_utf8 = || {
        (
            DataType::List(Arc::new(Field::new("item", DataType::Utf8, false))),
            false,
        )
    };
    match (table, column) {
        ("commits", "parents") => list_utf8(),
        ("commits", "import_artifact_hash" | "binding_id" | "binding_version") => utf8_nullable(),
        ("elements", "stereotypes") => list_utf8(),
        ("elements", "attributes") => (
            DataType::List(Arc::new(Field::new("item", attributes_struct_type(), true))),
            false,
        ),
        ("requirements", "covered") => (DataType::Boolean, false),
        ("requirements", "covering_link_kinds") => list_utf8(),
        ("metrics", "value") => (DataType::Float64, true),
        (
            "metrics",
            "basis_element_count" | "basis_relationship_count" | "constructs_not_carried",
        ) => (DataType::Int64, false),
        ("metric_definitions", "depends_on") => list_utf8(),
        _ => utf8(),
    }
}

fn build_column(dtype: &DataType, rows: &[Value], col: &str) -> Result<ArrayRef, String> {
    match dtype {
        DataType::Utf8 => {
            let v: Vec<Option<String>> = rows.iter().map(|r| json_str(r, col)).collect();
            Ok(Arc::new(StringArray::from(v)))
        }
        DataType::Boolean => {
            let v: Vec<Option<bool>> = rows.iter().map(|r| json_bool(r, col)).collect();
            Ok(Arc::new(BooleanArray::from(v)))
        }
        DataType::Int64 => {
            let v: Vec<Option<i64>> = rows.iter().map(|r| json_i64(r, col)).collect();
            Ok(Arc::new(Int64Array::from(v)))
        }
        DataType::Float64 => {
            let v: Vec<Option<f64>> = rows.iter().map(|r| json_f64(r, col)).collect();
            Ok(Arc::new(Float64Array::from(v)))
        }
        DataType::List(field) if field.data_type() == &DataType::Utf8 => {
            Ok(string_list_array(rows, col))
        }
        DataType::List(field) if matches!(field.data_type(), DataType::Struct(_)) => {
            Ok(attributes_list_array(rows, col))
        }
        other => Err(format!("unsupported Arrow type {:?} for {}", other, col)),
    }
}

fn json_str(row: &Value, col: &str) -> Option<String> {
    row.get(col).and_then(Value::as_str).map(String::from)
}

fn json_bool(row: &Value, col: &str) -> Option<bool> {
    row.get(col).and_then(Value::as_bool)
}

fn json_i64(row: &Value, col: &str) -> Option<i64> {
    row.get(col).and_then(Value::as_i64)
}

fn json_f64(row: &Value, col: &str) -> Option<f64> {
    row.get(col).and_then(Value::as_f64)
}

fn string_list_array(rows: &[Value], col: &str) -> ArrayRef {
    let mut offsets = vec![0i32];
    let mut flat: Vec<Option<String>> = Vec::new();
    for row in rows {
        let items = json_str_list(row, col);
        flat.extend(items);
        offsets.push(flat.len() as i32);
    }
    let values = Arc::new(StringArray::from(flat));
    let offsets = OffsetBuffer::new(offsets.into());
    Arc::new(ListArray::new(
        Arc::new(Field::new("item", DataType::Utf8, false)),
        offsets,
        values,
        None,
    ))
}

fn attributes_list_array(rows: &[Value], col: &str) -> ArrayRef {
    let mut offsets = vec![0i32];
    let mut names: Vec<Option<String>> = Vec::new();
    let mut types: Vec<Option<String>> = Vec::new();
    let mut aggregations: Vec<Option<String>> = Vec::new();
    let mut defaults: Vec<Option<String>> = Vec::new();
    for row in rows {
        let attrs = json_attributes(row, col);
        for (name, ty, agg, def) in attrs {
            names.push(Some(name));
            types.push(Some(ty));
            aggregations.push(Some(agg));
            defaults.push(Some(def));
        }
        offsets.push(names.len() as i32);
    }
    let fields = Fields::from(vec![
        Field::new("name", DataType::Utf8, false),
        Field::new("type", DataType::Utf8, false),
        Field::new("aggregation", DataType::Utf8, false),
        Field::new("default", DataType::Utf8, false),
    ]);
    let struct_arr = StructArray::new(
        fields,
        vec![
            Arc::new(StringArray::from(names)) as ArrayRef,
            Arc::new(StringArray::from(types)) as ArrayRef,
            Arc::new(StringArray::from(aggregations)) as ArrayRef,
            Arc::new(StringArray::from(defaults)) as ArrayRef,
        ],
        None,
    );
    let dtype = attributes_struct_type();
    Arc::new(ListArray::new(
        Arc::new(Field::new("item", dtype, true)),
        OffsetBuffer::new(offsets.into()),
        Arc::new(struct_arr),
        None,
    ))
}

fn json_str_list(row: &Value, col: &str) -> Vec<Option<String>> {
    row.get(col)
        .and_then(Value::as_array)
        .map(|arr| arr.iter().map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

fn json_attributes(row: &Value, col: &str) -> Vec<(String, String, String, String)> {
    row.get(col)
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .map(|v| {
                    (
                        v.get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        v.get("type")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        v.get("aggregation")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        v.get("default")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}
