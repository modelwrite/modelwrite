// SPDX-License-Identifier: AGPL-3.0-or-later
//! The analyses frame (slice A1): a DEFINITION, a RUN stored as a record, its FINDINGS, and
//! the DIFF between two runs.
//!
//! ## What this module is for
//!
//! Running an analysis produces a **record**, not a page that recomputes on view. A run
//! carries the commit it examined, the definition version it ran, the engine version it was
//! computed at, and the findings it measured - so the same model, the same analysis and the
//! same engine version produce the same record, byte for byte ([Measured::evidence_hash]
//! covers exactly the finding bytes). That is ruling 1 of the approved design, and it is what
//! makes the library and the diff possible.
//!
//! ## The rule this module exists to keep
//!
//! **The LLM authors and interprets; the engine measures.** Every value in a [Finding] comes
//! from an engine computation named, as data, on the definition that produced it
//! ([AnalysisDefinition::computations]). Nothing here counts anything the engine did not
//! count. The one place this module reads the document itself instead of calling the graph
//! crate - the dangling-link read - is the SAME detector the model-health view renders
//! ([crate::ui::health]), reused rather than reimplemented, so an analysis can never disagree
//! with the page it is an analysis of.
//!
//! ## UNKNOWN is not blank
//!
//! Where the engine cannot measure - a document that carries no graph at all, so the graph
//! analysis has no input - the run says so. It does not render zero and it does not omit the
//! row: the finding is emitted with [Severity::Unknown] and a reason, and the run records
//! itself as not measured. "Nothing to measure" is not "nothing wrong".
//!
//! ## Findings are addressable and diffable
//!
//! A finding's identity ([Finding::id]) is a function of the MEASURED SUBJECT and the check
//! that measured it, never of the run: the same uncovered requirement has the same id in two
//! runs months apart, which is what lets [diff] say "this appeared" and "this was resolved"
//! instead of leaving a person to infer it.
//!
//! ## What A1 deliberately does NOT do
//!
//! No plugin system, no LLM-authored definition (a definition the built-in vocabulary cannot
//! express is REFUSED with a reason, never silently approximated), no model-level simulation.
//! A run READS the model; it never writes one.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use graph::requirement_coverage;
use okf::types::OkfRoot;

use crate::store::blob_hash;
use crate::ui::health::{health_report, name_index};

/// The engine version every run is stamped with: the version of the code that measured it.
///
/// The same value the analytics projection stamps its metrics with, so "computed at engine
/// version X" means the same thing on both surfaces.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The definition id of the built-in requirement-coverage analysis.
pub const REQUIREMENT_COVERAGE: &str = "requirement-coverage";
/// The definition id of the built-in model-health (graph gaps) analysis.
pub const MODEL_HEALTH: &str = "model-health";
/// The version of the built-in definitions. Bump when a definition's checks change: a run
/// names the version that produced it, so an old run stays readable after a definition moves.
pub const DEFINITION_VERSION: u32 = 1;

/// One engine computation an analysis names. The definition carries these as DATA, so what an
/// analysis measured is answerable from the record alone, a year later, without reading code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Computation {
    /// The check id, as the finding's own check field names it.
    pub id: String,
    /// What computes it - the engine crate, or the server's own detector where the read is a
    /// read of the document rather than an engine call.
    pub measured_by: String,
    /// What it provides to the analysis, in one line.
    pub provides: String,
}

fn computation(id: &str, measured_by: &str, provides: &str) -> Computation {
    Computation {
        id: id.to_string(),
        measured_by: measured_by.to_string(),
        provides: provides.to_string(),
    }
}

/// Who authored a definition. A1 ships built-ins only; the LLM-proposed-and-accepted case
/// (slice A3) is a variant here rather than a new field, so a run's provenance cannot be
/// forgotten when the vocabulary grows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Provenance {
    /// Shipped with the server, reviewed as code.
    BuiltIn,
}

impl Provenance {
    pub fn as_str(&self) -> &'static str {
        match self {
            Provenance::BuiltIn => "built-in",
        }
    }
}

/// A named, declared analysis: what it measures, with which engine computations, at which
/// version. A run names the definition version that produced it (ruling 3 of the design).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisDefinition {
    pub id: String,
    pub name: String,
    pub version: u32,
    /// One line saying what the analysis answers.
    pub summary: String,
    pub provenance: Provenance,
    /// The engine computations this analysis uses, as data.
    pub computations: Vec<Computation>,
}

/// Requirement coverage: every requirement, named, with whether a Satisfy, Refine, Verify or
/// Allocate link covers it. The computation is the graph crate's own coverage report - the
/// same one the gate and the requirements page use, so this analysis can never disagree with
/// them.
pub fn requirement_coverage_definition() -> AnalysisDefinition {
    AnalysisDefinition {
        id: REQUIREMENT_COVERAGE.to_string(),
        name: "Requirement coverage".to_string(),
        version: DEFINITION_VERSION,
        summary: "Every requirement the model declares, named, with whether a Satisfy, Refine, Verify or Allocate link covers it.".to_string(),
        provenance: Provenance::BuiltIn,
        computations: vec![computation(
            "graph.requirement_coverage",
            "mw-graph",
            "the covered and uncovered requirement sets, and the count of each link kind that covers them",
        )],
    }
}

/// Model health: the graph gaps - orphaned nodes, isolated groups and dangling links - from
/// the SAME engine calls and the SAME detector the health view renders.
pub fn model_health_definition() -> AnalysisDefinition {
    AnalysisDefinition {
        id: MODEL_HEALTH.to_string(),
        name: "Model health (graph gaps)".to_string(),
        version: DEFINITION_VERSION,
        summary: "Orphaned nodes, isolated groups and dangling links: what is disconnected or unresolved in the model's graph.".to_string(),
        provenance: Provenance::BuiltIn,
        computations: vec![
            computation(
                "graph.graph_stats",
                "mw-graph",
                "node and edge counts, the isolated node set and the component sizes",
            ),
            computation(
                "graph.components",
                "mw-graph",
                "connected-component membership, so an isolated group is named member by member rather than only counted",
            ),
            computation(
                "graph.dangling_links",
                "mw-server (the health detector, the single source of truth for these counts)",
                "every edge whose endpoint is not a node, read from the document's own edge list",
            ),
        ],
    }
}

/// Every built-in definition, in the order the page offers them.
pub fn built_ins() -> Vec<AnalysisDefinition> {
    vec![requirement_coverage_definition(), model_health_definition()]
}

/// Resolve a definition by id, or refuse with a reason. A definition the bounded vocabulary
/// cannot express is REFUSED - never silently approximated (design, "the honest scoping").
pub fn resolve(id: &str) -> Result<AnalysisDefinition, Refusal> {
    match id {
        REQUIREMENT_COVERAGE => Ok(requirement_coverage_definition()),
        MODEL_HEALTH => Ok(model_health_definition()),
        other => Err(refusal(other)),
    }
}

/// The refusal for a definition this server cannot run, naming what it CAN run.
pub fn refusal(id: &str) -> Refusal {
    Refusal {
        reason: format!(
            "this server has no analysis definition named '{}'; the definitions it can run are {}",
            id,
            built_ins()
                .iter()
                .map(|definition| definition.id.clone())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// A refusal: an analysis the vocabulary cannot express, with the reason, as a value rather
/// than an approximation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub reason: String,
}

/// How a finding reads. Gap is a measured problem, Ok is a measurement that found none, and
/// Unknown is a thing that could not be measured - which is never rendered as a zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    Gap,
    Ok,
    Unknown,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Gap => "gap",
            Severity::Ok => "ok",
            Severity::Unknown => "unknown",
        }
    }

    /// Every severity, in the order a run renders them: what is broken, what could not be
    /// measured, then what measured clean.
    pub const ALL: [Severity; 3] = [Severity::Gap, Severity::Unknown, Severity::Ok];
}

/// One finding: what the engine measured about one subject, with the evidence behind it.
///
/// The id is the finding's ADDRESS. It is a pure function of the definition, the check and the
/// measured subject, so it survives across runs; the evidence is the measurement itself, so
/// nothing is asserted that was not measured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub id: String,
    /// The engine computation that measured this, named as the definition names it.
    pub check: String,
    pub severity: Severity,
    /// The element or requirement id the finding is about.
    pub subject: String,
    /// The subject's display name, empty when the document names none.
    pub subject_name: String,
    /// One line, in words. It states the measurement; it never states a number the engine did
    /// not produce (the test suite scans every digit in it, exactly as the onboarding summary
    /// is scanned).
    pub statement: String,
    /// The measurement itself, as the engine's own values.
    pub evidence: Value,
}

impl Finding {
    /// Whether two findings are the SAME READING of the same subject: the same address (so the
    /// same definition, check and subject) and the same severity.
    ///
    /// This - not byte equality of the whole finding - is what a diff compares. A finding's
    /// evidence carries the engine's own report for the WHOLE run (the coverage totals, the
    /// node and edge counts), so one requirement losing its coverage changes the evidence of
    /// every coverage finding in the run; comparing bytes would report the whole run as
    /// "measured differently" and say nothing about which requirement moved. The judgement is
    /// what changed or did not.
    pub fn same_reading(&self, other: &Finding) -> bool {
        self.id == other.id && self.severity == other.severity
    }

    /// Build a finding, addressing it by definition, check and measured subject.
    pub fn new(
        definition_id: &str,
        check: &str,
        severity: Severity,
        subject: &str,
        subject_name: &str,
        statement: &str,
        evidence: Value,
    ) -> Finding {
        Finding {
            id: format!("{}:{}:{}", definition_id, check, subject),
            check: check.to_string(),
            severity,
            subject: subject.to_string(),
            subject_name: subject_name.to_string(),
            statement: statement.to_string(),
            evidence,
        }
    }
}

/// The deterministic measurement of one definition over one document: the findings, whether
/// the engine could measure at all, and the canonical bytes and hash an evidence record is
/// cited by.
///
/// The findings_json field is the exact canonical serialisation the evidence hash covers, so a
/// stored run carries the bytes its own hash was taken over - the same discipline the gate's
/// evidence record keeps.
#[derive(Debug, Clone, PartialEq)]
pub struct Measured {
    pub findings: Vec<Finding>,
    /// False when the engine could not measure this document at all (no graph): the findings
    /// then say why, one per check, rather than reporting zero.
    pub measured: bool,
    pub findings_json: String,
    pub evidence_hash: String,
}

/// Measure a definition over a document. This is the ONLY place a finding is produced, and
/// every value in one comes from an engine call named on the definition.
pub fn measure(definition: &AnalysisDefinition, root: &OkfRoot) -> Result<Measured, Refusal> {
    match definition.id.as_str() {
        REQUIREMENT_COVERAGE => Ok(measure_requirement_coverage(definition, root)),
        MODEL_HEALTH => Ok(measure_model_health(definition, root)),
        other => Err(refusal(other)),
    }
}

/// Sort, serialise and hash a measured finding set. Findings are ordered by id, so the bytes
/// are a function of the measured SET and not of the order the checks happened to run in -
/// which is what makes "the same definition over the same revision produces the same evidence
/// bytes" true rather than hopeful.
fn finish(findings: Vec<Finding>, measured: bool) -> Measured {
    let mut findings = findings;
    findings.sort_by(|a, b| a.id.cmp(&b.id));
    let findings_json = serde_json::to_string(&findings).expect("findings serialise");
    let evidence_hash = blob_hash(findings_json.as_bytes());
    Measured {
        findings,
        measured,
        findings_json,
        evidence_hash,
    }
}

/// Requirement coverage, from the graph crate's own coverage report.
fn measure_requirement_coverage(definition: &AnalysisDefinition, root: &OkfRoot) -> Measured {
    const CHECK: &str = "graph.requirement_coverage";
    let names = name_index(root);

    // No graph is no input for the graph analysis. "Nothing to measure" is not "nothing
    // wrong", so this is Unknown with the reason, never a zero.
    if root.graph.is_none() {
        let finding = Finding::new(
            &definition.id,
            CHECK,
            Severity::Unknown,
            "graph",
            "",
            "the model carries no graph, so requirement coverage cannot be measured from it",
            json!({
                "measured": false,
                "reason": "the document has no graph section",
                "requirements": root.requirements.len(),
            }),
        );
        return finish(vec![finding], false);
    }

    let coverage = requirement_coverage(root);
    let mut findings = Vec::new();
    for requirement in &root.requirements {
        let covered = !coverage.uncovered.iter().any(|id| id == &requirement.id);
        let name = names
            .get(&requirement.id)
            .cloned()
            .unwrap_or_else(|| requirement.name.clone());
        let statement = if covered {
            "covered by a Satisfy, Refine, Verify or Allocate link"
        } else {
            "no Satisfy, Refine, Verify or Allocate link covers this requirement"
        };
        findings.push(Finding::new(
            &definition.id,
            CHECK,
            if covered { Severity::Ok } else { Severity::Gap },
            &requirement.id,
            &name,
            statement,
            coverage_evidence(covered, &coverage),
        ));
    }

    // A model that declares no requirements is MEASURED, and its answer is not an empty list:
    // coverage over nothing is said in words rather than left blank.
    if root.requirements.is_empty() {
        findings.push(Finding::new(
            &definition.id,
            CHECK,
            Severity::Ok,
            "requirements",
            "",
            "the model declares no requirements, so there is nothing to cover",
            json!({
                "measured": true,
                "total": 0,
                "coveredCount": 0,
                "uncoveredCount": 0,
            }),
        ));
    }

    finish(findings, true)
}

/// The engine's own coverage counts, as the evidence behind one requirement's finding.
fn coverage_evidence(covered: bool, coverage: &graph::CoverageReport) -> Value {
    json!({
        "measured": true,
        "covered": covered,
        "total": coverage.total,
        "coveredCount": coverage.covered,
        "uncoveredCount": coverage.uncovered.len(),
        "satisfied": coverage.satisfied,
        "refined": coverage.refined,
        "verified": coverage.verified,
        "allocated": coverage.allocated,
    })
}

/// The graph gaps: orphaned nodes, isolated groups and dangling links, from the SAME detector
/// the model-health view renders.
fn measure_model_health(definition: &AnalysisDefinition, root: &OkfRoot) -> Measured {
    let names = name_index(root);

    // health_report calls the graph crate, which requires a graph. Without one there is
    // nothing to measure, and every check the definition names says so in its own finding.
    if root.graph.is_none() {
        let findings = [
            (
                "graph.graph_stats",
                "the model carries no graph, so orphaned nodes cannot be measured from it",
                "orphaned nodes",
            ),
            (
                "graph.components",
                "the model carries no graph, so isolated groups cannot be measured from it",
                "isolated groups",
            ),
            (
                "graph.dangling_links",
                "the model carries no graph, so dangling links cannot be measured from it",
                "dangling links",
            ),
        ]
        .into_iter()
        .map(|(check, statement, subject)| {
            Finding::new(
                &definition.id,
                check,
                Severity::Unknown,
                subject,
                "",
                statement,
                json!({ "measured": false, "reason": "the document has no graph section" }),
            )
        })
        .collect();
        return finish(findings, false);
    }

    let report = health_report(root);
    let stats = graph::graph_stats(root);
    let mut findings = Vec::new();

    for id in &report.orphaned {
        findings.push(Finding::new(
            &definition.id,
            "graph.graph_stats",
            Severity::Gap,
            id,
            names.get(id).map(String::as_str).unwrap_or(""),
            "has no edges, so it is invisible to coverage and traceability",
            json!({
                "measured": true,
                "nodeCount": stats.node_count,
                "edgeCount": stats.edge_count,
                "componentCount": stats.component_count,
                "isolatedNodes": stats.isolated.len(),
            }),
        ));
    }

    for group in &report.isolated_groups {
        // The subject of a group is its membership: a group that gained or lost a member is a
        // measurement of a different subject, and the diff says so rather than hiding it
        // behind a count that happens to match.
        let subject = group.join(", ");
        findings.push(Finding::new(
            &definition.id,
            "graph.components",
            Severity::Gap,
            &subject,
            "",
            "sits in a disconnected group cut off from the main body; the members are named in the evidence",
            json!({
                "measured": true,
                "members": group,
                "groupSize": group.len(),
                "componentCount": stats.component_count,
                "nodeCount": stats.node_count,
            }),
        ));
    }

    for link in &report.dangling {
        let subject = format!("{} -> {}", link.source, link.target);
        findings.push(Finding::new(
            &definition.id,
            "graph.dangling_links",
            Severity::Gap,
            &subject,
            "",
            "names an endpoint that is not a node in the graph, so the link resolves to nothing",
            json!({
                "measured": true,
                "source": link.source,
                "target": link.target,
                "kind": link.kind,
                "label": link.label,
                "missingSource": link.missing_source,
                "missingTarget": link.missing_target,
            }),
        ));
    }

    finish(findings, true)
}

/// The plain-language line that heads a run: a template over values measured from the findings
/// themselves, so it cannot state a number the engine did not produce. Read it as a contract -
/// the test suite scans every digit in it against the findings, exactly as the onboarding
/// summary's digits are scanned against the engine.
pub fn summary_sentence(
    definition: &AnalysisDefinition,
    findings: &[Finding],
    measured: bool,
) -> String {
    let gaps = findings
        .iter()
        .filter(|finding| finding.severity == Severity::Gap)
        .count();
    if !measured {
        return format!(
            "Not measured: {} could not measure this model - the reason is on each finding below.",
            definition.name
        );
    }
    if findings.is_empty() {
        return format!(
            "{} measured this model and found nothing to report.",
            definition.name
        );
    }
    if gaps == 0 {
        return format!(
            "No gaps: {} measured this model and found {} findings, none of them a gap.",
            definition.name,
            findings.len()
        );
    }
    format!(
        "{}: {} measured this model and found {} among {} findings.",
        gaps,
        definition.name,
        if gaps == 1 { "a gap" } else { "gaps" },
        findings.len()
    )
}

/// One stored analysis run: the DEFINITION it ran, the commit it examined, the engine version
/// it was computed at, who ran it and when, and the findings it measured.
///
/// The id is deterministic: the same project, definition version, commit, engine version and
/// evidence hash always produce the same id, so re-running an unchanged analysis over an
/// unchanged model stores the same record rather than a second one (ruling 1: an analysis is a
/// record, not a side effect).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisRun {
    pub id: String,
    pub project: String,
    pub branch: String,
    pub commit_hash: String,
    pub engine_version: String,
    pub created_at: String,
    /// The verified subject that ran it, never a name from a request body.
    pub run_by: String,
    /// The definition AS RUN, naming its engine computations.
    pub definition: AnalysisDefinition,
    pub findings: Vec<Finding>,
    /// The canonical finding bytes the evidence hash covers.
    pub findings_json: String,
    pub measured: bool,
    pub evidence_hash: String,
}

impl AnalysisRun {
    /// The definition id, read from the definition the run carries.
    pub fn definition_id(&self) -> &str {
        &self.definition.id
    }

    pub fn definition_version(&self) -> u32 {
        self.definition.version
    }

    /// The findings of one severity, in the order the run stored them.
    pub fn of_severity(&self, severity: Severity) -> Vec<&Finding> {
        self.findings
            .iter()
            .filter(|finding| finding.severity == severity)
            .collect()
    }

    pub fn count(&self, severity: Severity) -> usize {
        self.of_severity(severity).len()
    }
}

/// The content address of a run: project, definition id and version, commit, engine version,
/// and the evidence hash. Deterministic by construction - the run time and the runner are
/// deliberately NOT part of it, because a run that measured the same thing twice is the same
/// record however long the second one waited to be run.
pub fn record_id(
    project: &str,
    definition: &AnalysisDefinition,
    commit_hash: &str,
    evidence_hash: &str,
) -> String {
    blob_hash(
        format!(
            "{}|{}|{}|{}|{}|{}",
            project, definition.id, definition.version, commit_hash, ENGINE_VERSION, evidence_hash
        )
        .as_bytes(),
    )
}

/// Assemble the record a store writes: the measured findings, addressed and stamped.
#[allow(clippy::too_many_arguments)]
pub fn record(
    project: &str,
    branch: &str,
    commit_hash: &str,
    run_by: &str,
    created_at: &str,
    definition: &AnalysisDefinition,
    measured: &Measured,
) -> AnalysisRun {
    AnalysisRun {
        id: record_id(project, definition, commit_hash, &measured.evidence_hash),
        project: project.to_string(),
        branch: branch.to_string(),
        commit_hash: commit_hash.to_string(),
        engine_version: ENGINE_VERSION.to_string(),
        created_at: created_at.to_string(),
        run_by: run_by.to_string(),
        definition: definition.clone(),
        findings: measured.findings.clone(),
        findings_json: measured.findings_json.clone(),
        measured: measured.measured,
        evidence_hash: measured.evidence_hash.clone(),
    }
}

/// One finding whose measurement changed between two runs: the same address, read differently.
#[derive(Debug, Clone, PartialEq)]
pub struct FindingChange {
    pub from: Finding,
    pub to: Finding,
}

/// What changed between two runs. The three lists are disjoint by construction: a finding
/// appeared, was resolved, or is the same address measured differently, and everything else is
/// unchanged. Nothing here is inferred - every entry is two stored findings compared by
/// identity.
#[derive(Debug, Clone, PartialEq)]
pub struct FindingsDiff {
    pub from: AnalysisRun,
    pub to: AnalysisRun,
    pub appeared: Vec<Finding>,
    pub resolved: Vec<Finding>,
    pub changed: Vec<FindingChange>,
    pub unchanged: usize,
}

impl FindingsDiff {
    /// Whether the two runs measured the same thing. A run that could not measure and a run
    /// that measured nothing wrong are NOT the same: the measured flag is part of it, so a
    /// diff can never turn "unmeasured" into "clean".
    pub fn identical(&self) -> bool {
        self.is_empty() && self.from.measured == self.to.measured
    }

    pub fn is_empty(&self) -> bool {
        self.appeared.is_empty() && self.resolved.is_empty() && self.changed.is_empty()
    }
}

/// Compare two stored runs, finding by finding, by ADDRESS ([Finding::id]). Two runs of the
/// same definition over two commits therefore say what appeared, what was resolved and what
/// was measured differently - a change is visible, not inferred.
///
/// "Measured differently" is [Finding::same_reading] false: the same address, read with a
/// different severity. The evidence of BOTH readings rides the change, so a person can see
/// what the engine said each time.
pub fn diff(from: &AnalysisRun, to: &AnalysisRun) -> FindingsDiff {
    let mut appeared = Vec::new();
    let mut changed = Vec::new();
    let mut unchanged = 0usize;
    for finding in &to.findings {
        match from.findings.iter().find(|other| other.id == finding.id) {
            None => appeared.push(finding.clone()),
            Some(previous) if previous.same_reading(finding) => unchanged += 1,
            Some(previous) => changed.push(FindingChange {
                from: previous.clone(),
                to: finding.clone(),
            }),
        }
    }
    let resolved: Vec<Finding> = from
        .findings
        .iter()
        .filter(|finding| !to.findings.iter().any(|other| other.id == finding.id))
        .cloned()
        .collect();
    FindingsDiff {
        from: from.clone(),
        to: to.clone(),
        appeared,
        resolved,
        changed,
        unchanged,
    }
}
