// SPDX-License-Identifier: AGPL-3.0-or-later
//! A1 - the analyses frame, proved end to end.
//!
//! The proofs this file carries, in the order the design lists them:
//!
//! 1. both built-in analyses run over a REAL seeded model (the coffee-machine corpus, a real
//!    MagicDraw export), are STORED as records, and the library lists them;
//! 2. a DIFF between two runs after a real model change names what appeared, what was
//!    resolved and what was measured differently, with finding identity surviving across the
//!    two runs;
//! 3. determinism: the same definition, commit and engine version produce the same record;
//! 4. UNKNOWN is never blank: an analysis over a model that cannot be measured says so rather
//!    than rendering zero;
//! 5. an analysis never writes: the project's branch tip is unchanged after a run;
//! 6. every number in a finding is the engine's, scanned digit by digit exactly as the
//!    onboarding summary is scanned;
//! 7. a definition the vocabulary cannot express is refused, with a reason.

use std::collections::HashSet;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sha2::{Digest, Sha256};
use tower::ServiceExt;

use server::analyses::{self, AnalysisRun, Severity};
use server::auth::AuthConfig;
use server::store::sqlite::SqliteStore;
use server::AppState;

// ---------------------------------------------------------------------------
// Harness.

fn state(dir: &std::path::Path) -> AppState {
    AppState {
        store: Arc::new(SqliteStore::open(&dir.join("mw.db")).unwrap()),
        evidence_dir: dir.to_path_buf(),
        auth: AuthConfig::Open,
    }
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn post(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn post_form(uri: &str, params: &[(&str, &str)]) -> Request<Body> {
    let body = params
        .iter()
        .map(|(key, value)| format!("{}={}", key, percent_encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .unwrap()
}

fn percent_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{:02X}", byte)),
        }
    }
    out
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// Create the project and commit one document onto main, returning the commit hash.
async fn seed(router: &axum::Router, project: &str, doc: &serde_json::Value) -> String {
    let created = router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": project })))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    commit(router, project, "main", "seed", doc).await
}

async fn commit(
    router: &axum::Router,
    project: &str,
    branch: &str,
    message: &str,
    doc: &serde_json::Value,
) -> String {
    let response = router
        .clone()
        .oneshot(post(
            &format!("/projects/{project}/commits"),
            serde_json::json!({ "branch": branch, "author": "alex", "message": message, "okf": doc }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = body_text(response).await;
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    value["hash"].as_str().unwrap().to_string()
}

/// Run one analysis through the PAGE a person uses, and return the id of the record it stored
/// - read from the address the page redirects to, never from a re-run.
async fn run_analysis(
    router: &axum::Router,
    project: &str,
    definition: &str,
    branch: &str,
) -> String {
    let response = router
        .clone()
        .oneshot(post_form(
            &format!("/ui/projects/{project}/analyses"),
            &[("definition", definition), ("branch", branch)],
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::SEE_OTHER,
        "running an analysis stores a record and lands the caller on it"
    );
    let location = response
        .headers()
        .get("location")
        .expect("the run redirects to the record it stored")
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        location.starts_with(&format!("/ui/projects/{project}/analyses/")),
        "the record is addressed under the project, got {}",
        location
    );
    location.rsplit('/').next().unwrap().to_string()
}

async fn page(router: &axum::Router, uri: &str) -> String {
    let response = router.clone().oneshot(get(uri)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK, "GET {} failed", uri);
    body_text(response).await
}

fn corpus() -> serde_json::Value {
    serde_json::from_str(&test_support::load_okf_expected()).unwrap()
}

fn root_of(doc: &serde_json::Value) -> okf::types::OkfRoot {
    serde_json::from_value(doc.clone()).unwrap()
}

fn requirement_ids(doc: &serde_json::Value) -> HashSet<String> {
    doc["requirements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|requirement| requirement["id"].as_str().unwrap().to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// An independent reading of the ENGINE, computed here in the test rather than read back out
// of a page: this is the reference every finding is checked against. It is deliberately a
// plain read of the graph crate (and, for dangling links, of the document's own edge list),
// not a call into the frame under test.

fn engine_uncovered(doc: &serde_json::Value) -> HashSet<String> {
    graph::requirement_coverage(&root_of(doc))
        .uncovered
        .into_iter()
        .collect()
}

fn engine_covered(doc: &serde_json::Value) -> HashSet<String> {
    let report = graph::requirement_coverage(&root_of(doc));
    requirement_ids(doc)
        .into_iter()
        .filter(|id| !report.uncovered.contains(id))
        .collect()
}

fn engine_orphans(doc: &serde_json::Value) -> HashSet<String> {
    graph::graph_stats(&root_of(doc))
        .isolated
        .into_iter()
        .collect()
}

/// The isolated groups exactly as the health detector defines them: every component after the
/// largest, of two or more nodes.
fn engine_groups(doc: &serde_json::Value) -> Vec<Vec<String>> {
    graph::components(&root_of(doc))
        .groups
        .into_iter()
        .skip(1)
        .filter(|group| group.len() >= 2)
        .collect()
}

/// The dangling links as the DOCUMENT declares them: an edge naming an endpoint that is not a
/// node. The health view reads the same list, and the analysis must agree with it.
fn engine_dangling(doc: &serde_json::Value) -> HashSet<String> {
    let graph = doc.get("graph").unwrap();
    let nodes: HashSet<&str> = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["id"].as_str().unwrap())
        .collect();
    graph["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|edge| {
            !nodes.contains(edge["source"].as_str().unwrap())
                || !nodes.contains(edge["target"].as_str().unwrap())
        })
        .map(|edge| {
            format!(
                "{} -> {}",
                edge["source"].as_str().unwrap(),
                edge["target"].as_str().unwrap()
            )
        })
        .collect()
}

/// The subjects of one severity in a stored run.
fn subjects(run: &AnalysisRun, severity: Severity) -> HashSet<String> {
    run.findings
        .iter()
        .filter(|finding| finding.severity == severity)
        .map(|finding| finding.subject.clone())
        .collect()
}

/// The subjects of a diff list.
fn diff_subjects(findings: &[analyses::Finding]) -> HashSet<String> {
    findings
        .iter()
        .map(|finding| finding.subject.clone())
        .collect()
}

fn sha256(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

// ---------------------------------------------------------------------------
// 1 + 3 + 5. The two built-in analyses, stored, deterministic, and read-only.

#[tokio::test]
async fn both_built_in_analyses_run_over_the_real_model_and_are_stored_in_the_library() {
    let dir = tempfile::tempdir().unwrap();
    let app = state(dir.path());
    let store = app.store.clone();
    let router = server::app(app);
    let doc = corpus();
    let commit_hash = seed(&router, "coffee", &doc).await;

    let coverage_id = run_analysis(&router, "coffee", analyses::REQUIREMENT_COVERAGE, "main").await;
    let health_id = run_analysis(&router, "coffee", analyses::MODEL_HEALTH, "main").await;

    // The library lists what has been run: both records, each addressable.
    let library = page(&router, "/ui/projects/coffee/analyses").await;
    assert!(library.contains("Analyses"), "{}", library);
    assert!(library.contains("Requirement coverage"), "{}", library);
    assert!(library.contains("Model health (graph gaps)"), "{}", library);
    assert!(
        library.contains(&format!("/analyses/{}", coverage_id)),
        "the library must address the coverage run:\n{}",
        library
    );
    assert!(
        library.contains(&format!("/analyses/{}", health_id)),
        "the library must address the health run:\n{}",
        library
    );
    assert!(
        library.contains(analyses::ENGINE_VERSION),
        "a run carries the engine version it was computed at:\n{}",
        library
    );
    assert!(
        library.contains(&commit_hash[..8]),
        "a run carries the commit it examined:\n{}",
        library
    );

    // Both records are in the store, against the commit, with the engine version.
    let runs = store.analysis_runs("coffee").unwrap();
    assert_eq!(runs.len(), 2, "{:?}", runs);
    for run in &runs {
        assert_eq!(run.commit_hash, commit_hash);
        assert_eq!(run.engine_version, analyses::ENGINE_VERSION);
        assert!(run.measured);
    }

    // Proof (a): the coverage run's findings are the ENGINE's own coverage, named one by one.
    let coverage = runs
        .iter()
        .find(|run| run.definition_id() == analyses::REQUIREMENT_COVERAGE)
        .unwrap();
    assert_eq!(subjects(coverage, Severity::Gap), engine_uncovered(&doc));
    assert_eq!(subjects(coverage, Severity::Ok), engine_covered(&doc));
    let report = graph::requirement_coverage(&root_of(&doc));
    assert_eq!(coverage.count(Severity::Gap), report.uncovered.len());
    assert_eq!(coverage.count(Severity::Ok), report.covered);
    assert_eq!(coverage.count(Severity::Unknown), 0);

    // Proof (b): the health run's findings are the engine's graph gaps, exactly.
    let health = runs
        .iter()
        .find(|run| run.definition_id() == analyses::MODEL_HEALTH)
        .unwrap();
    let mut expected_gaps = engine_orphans(&doc);
    expected_gaps.extend(engine_dangling(&doc));
    for group in engine_groups(&doc) {
        expected_gaps.insert(group.join(", "));
    }
    assert_eq!(subjects(health, Severity::Gap), expected_gaps);
    assert_eq!(health.count(Severity::Unknown), 0);

    // The run page is the RECORD: the definition AS RUN is on it, naming the computations the
    // analysis used, and the headline is exactly what the findings compose.
    let run_page = page(
        &router,
        &format!("/ui/projects/coffee/analyses/{coverage_id}"),
    )
    .await;
    assert!(
        run_page.contains("graph.requirement_coverage"),
        "the record names the engine computation it used:\n{}",
        run_page
    );
    assert!(
        run_page.contains(&analyses::summary_sentence(
            &coverage.definition,
            &coverage.findings,
            coverage.measured
        )),
        "the page states exactly what the findings compose:\n{}",
        run_page
    );
    assert!(
        run_page.contains(&format!("Gaps ({})", coverage.count(Severity::Gap))),
        "the page names the gap count the record carries:\n{}",
        run_page
    );
    // Every finding carries its ADDRESS on the page, so identity is visible, not implied.
    for finding in &coverage.findings {
        assert!(run_page.contains(&finding.id), "{}", finding.id);
    }
}

#[tokio::test]
async fn the_same_definition_commit_and_engine_version_produce_the_same_record() {
    let dir = tempfile::tempdir().unwrap();
    let app = state(dir.path());
    let store = app.store.clone();
    let router = server::app(app);
    let doc = corpus();
    let commit_hash = seed(&router, "coffee", &doc).await;

    // Measure twice, directly against the frame, and compare the findings, the bytes and the
    // hash - the hash recomputed HERE, independently, over the bytes the record carries.
    let definition = analyses::resolve(analyses::REQUIREMENT_COVERAGE).unwrap();
    let root = root_of(&doc);
    let first = analyses::measure(&definition, &root).unwrap();
    let second = analyses::measure(&definition, &root).unwrap();
    assert_eq!(first.findings, second.findings);
    assert_eq!(first.findings_json, second.findings_json);
    assert_eq!(first.evidence_hash, second.evidence_hash);
    assert_eq!(sha256(&first.findings_json), first.evidence_hash);
    assert_eq!(
        analyses::record_id("coffee", &definition, &commit_hash, &first.evidence_hash),
        analyses::record_id("coffee", &definition, &commit_hash, &second.evidence_hash)
    );

    // Run the same analysis through the page twice: the same RECORD.
    let first_id = run_analysis(&router, "coffee", analyses::REQUIREMENT_COVERAGE, "main").await;
    let second_id = run_analysis(&router, "coffee", analyses::REQUIREMENT_COVERAGE, "main").await;
    assert_eq!(
        first_id, second_id,
        "the same definition over the same commit is the same record"
    );
    assert_eq!(
        store.analysis_runs("coffee").unwrap().len(),
        1,
        "re-running an unchanged analysis stores the same record, not a second one"
    );
    let stored = store.analysis_run("coffee", &first_id).unwrap().unwrap();
    assert_eq!(stored.id, first_id);
    assert_eq!(stored.engine_version, analyses::ENGINE_VERSION);
    assert_eq!(stored.findings, first.findings);
    assert_eq!(stored.findings_json, first.findings_json);
    assert_eq!(sha256(&stored.findings_json), stored.evidence_hash);
    assert_eq!(stored.commit_hash, commit_hash);
    // The stored bytes ARE the canonical bytes the hash was taken over, so a read-back is
    // byte-identical.
    let reparsed: Vec<analyses::Finding> = serde_json::from_str(&stored.findings_json).unwrap();
    assert_eq!(reparsed, stored.findings);
}

#[tokio::test]
async fn an_analysis_run_never_writes_to_the_model() {
    let dir = tempfile::tempdir().unwrap();
    let app = state(dir.path());
    let store = app.store.clone();
    let router = server::app(app);
    let doc = corpus();
    let commit_hash = seed(&router, "coffee", &doc).await;

    let tip_before = store.branch_tip("coffee", "main").unwrap();
    let commits_before = store.commits_on("coffee", "main").unwrap().len();
    let model_before = store.commit("coffee", &commit_hash).unwrap().unwrap();

    run_analysis(&router, "coffee", analyses::REQUIREMENT_COVERAGE, "main").await;
    run_analysis(&router, "coffee", analyses::MODEL_HEALTH, "main").await;

    // A run READS: the branch tip, the history and the model are exactly what they were.
    assert_eq!(store.branch_tip("coffee", "main").unwrap(), tip_before);
    assert_eq!(
        store.commits_on("coffee", "main").unwrap().len(),
        commits_before
    );
    let model_after = store.commit("coffee", &commit_hash).unwrap().unwrap();
    assert_eq!(model_after.okf_hash, model_before.okf_hash);
    assert_eq!(model_after.provenance, model_before.provenance);
    assert_eq!(model_after, model_before);
    assert_eq!(store.analysis_runs("coffee").unwrap().len(), 2);
    // The ACT is recorded - in the audit log, not in the model.
    let audit = store.audit("coffee", 50).unwrap();
    assert_eq!(
        audit
            .iter()
            .filter(|entry| entry.action == server::audit::ANALYSIS_RUN)
            .count(),
        2,
        "each run is recorded in the audit log: {:?}",
        audit
            .iter()
            .map(|entry| entry.action.clone())
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// 2. The diff: a real model change, and what appeared and was resolved.

/// The ids one model change touched, so the test can assert the diff against them by name
/// rather than only against a set difference.
struct ModelChange {
    doc: serde_json::Value,
    /// An uncovered requirement a new Satisfy link now covers: its finding goes clean.
    covered_now: String,
    /// A covered requirement whose ONLY covering link was removed: its finding becomes a gap.
    uncovered_now: String,
    /// An uncovered requirement that was deleted: its gap is resolved.
    removed: String,
    /// A requirement that arrived with no covering link: a gap appears.
    added: String,
}

/// Remove one real covering link and return the requirement it un-covers. The ENGINE decides -
/// candidate edges are removed one at a time and the graph crate's own coverage report says
/// whether exactly one requirement stopped being covered - so the test never reimplements the
/// coverage rule to build its own fixture.
fn uncover_one(doc: &serde_json::Value) -> (serde_json::Value, String) {
    let baseline = engine_uncovered(doc);
    let nodes: HashSet<String> = doc["graph"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["id"].as_str().unwrap().to_string())
        .collect();
    let edges = doc["graph"]["edges"].as_array().unwrap();
    for (index, edge) in edges.iter().enumerate() {
        let source = edge["source"].as_str().unwrap();
        let target = edge["target"].as_str().unwrap();
        // Only a link between two real nodes: the dangling ones are removed anyway, and they
        // are the health analysis's business, not this one's.
        if !nodes.contains(source) || !nodes.contains(target) {
            continue;
        }
        if edge["kind"] != "dependency" {
            continue;
        }
        if !matches!(
            edge["label"].as_str().unwrap(),
            "Satisfy" | "Refine" | "Verify" | "Allocate"
        ) {
            continue;
        }
        let mut candidate = doc.clone();
        candidate["graph"]["edges"]
            .as_array_mut()
            .unwrap()
            .remove(index);
        let after = engine_uncovered(&candidate);
        let mut flipped = after.difference(&baseline);
        if let Some(requirement) = flipped.next() {
            if flipped.next().is_none() {
                return (candidate, requirement.clone());
            }
        }
    }
    panic!("the corpus must carry a requirement covered by exactly one link");
}

/// The changed model: a real edit of the coffee corpus, not a random mutation.
///
/// * the two dangling links are removed (they named an endpoint that is not a node), so the two
///   dangling findings RESOLVE;
/// * one requirement's only covering link is removed, so that requirement's finding is
///   MEASURED DIFFERENTLY - it keeps its address and becomes a gap;
/// * one uncovered requirement is COVERED by a new Satisfy link, so its finding is measured
///   differently in the other direction;
/// * one uncovered requirement is DELETED, so its gap RESOLVES;
/// * a node with no edges is ADDED, so an orphan APPEARS;
/// * a requirement with no covering link is ADDED, so a gap APPEARS.
///
/// The two requirements that keep their address while their severity flips are the point: the
/// diff reports them as measured differently rather than as one resolution and one appearance,
/// which is the whole reason a finding has an address.
fn changed_model(original: &serde_json::Value) -> ModelChange {
    let mut gaps: Vec<String> = engine_uncovered(original).into_iter().collect();
    gaps.sort();
    let covered_now = gaps.pop().unwrap();
    let removed = gaps.pop().unwrap();

    let (mut doc, uncovered_now) = uncover_one(original);
    {
        let graph = doc.get_mut("graph").unwrap();
        let known: HashSet<String> = graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["id"].as_str().unwrap().to_string())
            .collect();
        let hub = {
            let mut ids: Vec<String> = known.iter().cloned().collect();
            ids.sort();
            ids[0].clone()
        };
        let edges = graph.get_mut("edges").unwrap().as_array_mut().unwrap();
        edges.retain(|edge| {
            known.contains(edge["source"].as_str().unwrap())
                && known.contains(edge["target"].as_str().unwrap())
        });
        edges.push(serde_json::json!({
            "source": hub,
            "target": covered_now,
            "kind": "dependency",
            "label": "Satisfy"
        }));
        graph
            .get_mut("nodes")
            .unwrap()
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "id": "a1-test-orphan",
                "kind": "block",
                "name": "Orphan added by the A1 proof"
            }));
    }
    {
        let requirements = doc.get_mut("requirements").unwrap().as_array_mut().unwrap();
        requirements.retain(|requirement| requirement["id"].as_str().unwrap() != removed);
        requirements.push(serde_json::json!({
            "id": "a1-test-requirement",
            "name": "A requirement added by the A1 proof",
            "reqText": "The A1 proof declares a requirement no link covers."
        }));
    }
    ModelChange {
        doc,
        covered_now,
        uncovered_now,
        removed,
        added: "a1-test-requirement".to_string(),
    }
}

#[tokio::test]
async fn the_diff_names_what_appeared_and_what_was_resolved_across_a_real_model_change() {
    let dir = tempfile::tempdir().unwrap();
    let app = state(dir.path());
    let store = app.store.clone();
    let router = server::app(app);
    let before_doc = corpus();
    seed(&router, "coffee", &before_doc).await;
    let change = changed_model(&before_doc);
    let after_doc = change.doc.clone();
    let before_requirements = requirement_ids(&before_doc);
    let after_requirements = requirement_ids(&after_doc);
    let uncovered_before = engine_uncovered(&before_doc);
    let uncovered_after = engine_uncovered(&after_doc);

    // Run both analyses over the model as it is, then change the model and run them again.
    let coverage_before =
        run_analysis(&router, "coffee", analyses::REQUIREMENT_COVERAGE, "main").await;
    let health_before = run_analysis(&router, "coffee", analyses::MODEL_HEALTH, "main").await;
    let after_commit = commit(
        &router,
        "coffee",
        "main",
        "cover one requirement, drop another",
        &after_doc,
    )
    .await;
    let coverage_after =
        run_analysis(&router, "coffee", analyses::REQUIREMENT_COVERAGE, "main").await;
    let health_after = run_analysis(&router, "coffee", analyses::MODEL_HEALTH, "main").await;
    assert_ne!(
        coverage_before, coverage_after,
        "a changed model is a new record"
    );

    // The library now lists four records, over two commits.
    let library = page(&router, "/ui/projects/coffee/analyses").await;
    assert!(library.contains("Library (4)"), "{}", library);
    // The compare form offers the two ENDS of the history by default, and both are runs of
    // the SAME definition, so pressing the button compares two readings of one analysis.
    assert!(
        library.contains(&format!("<option selected value=\"{}\"", coverage_before)),
        "the oldest run is preselected as the from side:\n{}",
        library
    );
    assert!(
        library.contains(&format!("<option selected value=\"{}\"", coverage_after)),
        "the newest run of the same definition is preselected as the to side:\n{}",
        library
    );

    // The diff, computed on the STORED runs.
    let load = |id: &str| store.analysis_run("coffee", id).unwrap().unwrap();
    let coverage_diff = analyses::diff(&load(&coverage_before), &load(&coverage_after));

    // What the ENGINE says the three lists must be, derived here from the two documents rather
    // than read back out of the diff.
    // Every requirement in a run has a finding, so a requirement that arrived is an appearance
    // and one that was deleted is a resolution, whatever its severity; a requirement that
    // stayed and changed severity is a change of READING, not two events.
    let expected_appeared: HashSet<String> = after_requirements
        .difference(&before_requirements)
        .cloned()
        .collect();
    let expected_resolved: HashSet<String> = before_requirements
        .difference(&after_requirements)
        .cloned()
        .collect();
    let expected_changed: HashSet<String> = before_requirements
        .intersection(&after_requirements)
        .filter(|id| uncovered_before.contains(*id) != uncovered_after.contains(*id))
        .cloned()
        .collect();
    assert!(uncovered_before.contains(&change.covered_now));
    assert!(!uncovered_after.contains(&change.covered_now));
    assert!(!uncovered_before.contains(&change.uncovered_now));
    assert!(uncovered_after.contains(&change.uncovered_now));

    assert_eq!(diff_subjects(&coverage_diff.appeared), expected_appeared);
    assert_eq!(diff_subjects(&coverage_diff.resolved), expected_resolved);
    assert_eq!(
        coverage_diff
            .changed
            .iter()
            .map(|change| change.to.subject.clone())
            .collect::<HashSet<_>>(),
        expected_changed
    );
    assert_eq!(expected_appeared, HashSet::from([change.added.clone()]));
    assert_eq!(expected_resolved, HashSet::from([change.removed.clone()]));
    // The two requirements whose severity flipped are measured DIFFERENTLY, not resolved and
    // re-added: their findings kept their addresses.
    assert_eq!(
        expected_changed,
        HashSet::from([change.covered_now.clone(), change.uncovered_now.clone()])
    );
    assert_eq!(coverage_diff.changed.len(), 2);
    for change in &coverage_diff.changed {
        assert_eq!(change.from.id, change.to.id, "a change keeps its identity");
        assert_ne!(change.from.severity, change.to.severity);
    }

    // IDENTITY SURVIVES: the findings the two runs share have the SAME address, so unchanged
    // findings are recognised as unchanged rather than as one resolution and one appearance.
    let mut unchanged = 0usize;
    for finding in &coverage_diff.from.findings {
        if let Some(other) = coverage_diff
            .to
            .findings
            .iter()
            .find(|other| other.id == finding.id)
        {
            assert_eq!(finding.subject, other.subject);
            assert_eq!(finding.check, other.check);
            if finding.same_reading(other) {
                unchanged += 1;
            }
        }
    }
    assert!(unchanged > 20, "the runs share most of their findings");
    assert_eq!(coverage_diff.unchanged, unchanged);
    assert!(!coverage_diff.identical());

    // The health diff: the orphan appears, the two dangling links resolve.
    let health_diff = analyses::diff(&load(&health_before), &load(&health_after));
    assert_eq!(
        diff_subjects(&health_diff.appeared),
        engine_orphans(&after_doc)
            .difference(&engine_orphans(&before_doc))
            .cloned()
            .collect::<HashSet<_>>()
    );
    assert!(diff_subjects(&health_diff.appeared).contains("a1-test-orphan"));
    assert_eq!(
        diff_subjects(&health_diff.resolved),
        engine_dangling(&before_doc)
            .difference(&engine_dangling(&after_doc))
            .cloned()
            .collect::<HashSet<_>>()
    );
    assert_eq!(diff_subjects(&health_diff.resolved).len(), 2);

    // And the DIFF PAGE the person reads says exactly these things.
    let diff_page = page(
        &router,
        &format!(
            "/ui/projects/coffee/analyses/diff?from={}&to={}",
            coverage_before, coverage_after
        ),
    )
    .await;
    assert!(diff_page.contains(&format!("Appeared ({})", coverage_diff.appeared.len())));
    assert!(diff_page.contains(&format!("Resolved ({})", coverage_diff.resolved.len())));
    assert!(diff_page.contains(&format!(
        "Measured differently ({})",
        coverage_diff.changed.len()
    )));
    assert!(diff_page.contains(&format!("{} unchanged", coverage_diff.unchanged)));
    for finding in &coverage_diff.appeared {
        assert!(
            diff_page.contains(&finding.id),
            "the diff names the finding's address: {}",
            finding.id
        );
    }
    for finding in &coverage_diff.resolved {
        assert!(diff_page.contains(&finding.id), "{}", finding.id);
    }

    // The run page carries the same diff when pointed at another run, so the comparison is one
    // click from a record.
    let run_page = page(
        &router,
        &format!(
            "/ui/projects/coffee/analyses/{}?compare={}",
            coverage_after, coverage_before
        ),
    )
    .await;
    assert!(run_page.contains(&format!("Appeared ({})", coverage_diff.appeared.len())));
    assert!(run_page.contains(&format!("Resolved ({})", coverage_diff.resolved.len())));
    assert!(run_page.contains(&after_commit[..8]));

    // Diffing a run with itself is a real answer, not an empty page: nothing changed.
    let same = page(
        &router,
        &format!(
            "/ui/projects/coffee/analyses/diff?from={}&to={}",
            coverage_after, coverage_after
        ),
    )
    .await;
    assert!(
        same.contains("The two runs measured the same thing"),
        "{}",
        same
    );
}

// ---------------------------------------------------------------------------
// 4. UNKNOWN is never blank.

/// The corpus with its graph removed: a document the graph analysis has no input for. It still
/// declares its requirements, so "coverage cannot be measured" is the honest answer and zero
/// would be a lie.
fn unmeasurable(doc: &serde_json::Value) -> serde_json::Value {
    let mut doc = doc.clone();
    doc.as_object_mut().unwrap().remove("graph");
    assert!(!doc["requirements"].as_array().unwrap().is_empty());
    doc
}

#[tokio::test]
async fn an_analysis_over_a_model_it_cannot_measure_says_so_rather_than_rendering_zero() {
    let dir = tempfile::tempdir().unwrap();
    let app = state(dir.path());
    let store = app.store.clone();
    let router = server::app(app);
    let doc = unmeasurable(&corpus());

    // The commit path VALIDATES a model (a document with no graph section is a validation
    // error), so the graphless commit is written the way any commit is written - through the
    // store's own commit path, which is what the CLI and the import path use. The analyses
    // themselves then run through the PAGE, exactly as a person runs them.
    let created = router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let bytes = serde_json::to_vec(&doc).unwrap();
    let okf_hash = store.put_blob(&bytes).unwrap();
    store
        .commit_model(
            "coffee",
            "main",
            &okf_hash,
            "alex",
            "a model with no graph section",
            None,
            None,
            None,
        )
        .unwrap();

    let coverage_id = run_analysis(&router, "coffee", analyses::REQUIREMENT_COVERAGE, "main").await;
    let health_id = run_analysis(&router, "coffee", analyses::MODEL_HEALTH, "main").await;

    for id in [coverage_id.as_str(), health_id.as_str()] {
        let run = store.analysis_run("coffee", id).unwrap().unwrap();
        assert!(
            !run.measured,
            "a document with no graph cannot be measured: {:?}",
            run.findings
        );
        assert!(!run.findings.is_empty(), "the row is NOT omitted");
        for finding in &run.findings {
            assert_eq!(
                finding.severity,
                Severity::Unknown,
                "an unmeasurable check is Unknown, never a zero: {:?}",
                finding
            );
            assert_eq!(finding.evidence["measured"], serde_json::json!(false));
            assert!(
                finding.evidence.get("coveredCount").is_none()
                    && finding.evidence.get("uncoveredCount").is_none()
                    && finding.evidence.get("isolatedNodes").is_none()
                    && finding.evidence.get("nodeCount").is_none()
                    && finding.evidence.get("total").is_none(),
                "an unmeasurable check publishes NO count at all: {:?}",
                finding.evidence
            );
            assert!(
                finding.statement.contains("cannot be measured"),
                "the finding says why: {}",
                finding.statement
            );
        }
        let page = page(&router, &format!("/ui/projects/coffee/analyses/{id}")).await;
        assert!(
            page.contains(&format!("Not measured ({})", run.findings.len())),
            "the not-measured rows are rendered, not omitted:\n{}",
            page
        );
        assert!(
            page.contains("Not measured:"),
            "the page says the run was not measured:\n{}",
            page
        );
        assert!(
            !page.contains("No gaps") && !page.contains("Healthy"),
            "an unmeasured run must never read as clean:\n{}",
            page
        );
        assert!(
            !page.contains("0 gap")
                && !page.contains("0 uncovered")
                && !page.contains("0 orphaned")
                && !page.contains("0 dangling"),
            "UNKNOWN is not blank and it is not zero:\n{}",
            page
        );
        assert!(
            page.contains("Gaps (0)"),
            "the gap row is present and says none: {}",
            page
        );
    }

    // The library, too, publishes no gap count for a run it could not measure.
    let library = page(&router, "/ui/projects/coffee/analyses").await;
    assert!(library.contains("not measured"), "{}", library);
    assert!(
        !library.contains("0 gap") && !library.contains("0 ok"),
        "a library row for an unmeasured run must not read as a zero:
{}",
        library
    );

    // The health run's rows name each check it could not run.
    let health = store.analysis_run("coffee", &health_id).unwrap().unwrap();
    let checks: HashSet<String> = health.findings.iter().map(|f| f.check.clone()).collect();
    assert_eq!(health.findings.len(), 3);
    assert!(checks.contains("graph.graph_stats"));
    assert!(checks.contains("graph.components"));
    assert!(checks.contains("graph.dangling_links"));

    let sentence =
        analyses::summary_sentence(&health.definition, &health.findings, health.measured);
    assert!(sentence.starts_with("Not measured"), "{}", sentence);
}

// ---------------------------------------------------------------------------
// 6. Every number is the engine's.

/// Every maximal run of ASCII digits in a piece of prose.
fn digit_runs(text: &str) -> Vec<String> {
    let mut runs = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else if !current.is_empty() {
            runs.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        runs.push(current);
    }
    runs
}

fn collect_strings(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::String(text) => {
            out.push_str(text);
            out.push(' ');
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_strings(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                collect_strings(item, out);
            }
        }
        _ => {}
    }
}

fn collect_numbers(value: &serde_json::Value, out: &mut Vec<u64>) {
    match value {
        serde_json::Value::Number(number) => {
            if let Some(value) = number.as_u64() {
                out.push(value);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_numbers(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                collect_numbers(item, out);
            }
        }
        _ => {}
    }
}

/// The strings the ENGINE supplied for one finding: its subject, its name, its address, and
/// every string inside the evidence. A digit run inside one of these is the engine's own text
/// (an opaque MagicDraw id, for example), not a number this frame invented.
fn engine_strings(finding: &analyses::Finding) -> String {
    let mut haystack = String::new();
    haystack.push_str(&finding.subject);
    haystack.push(' ');
    haystack.push_str(&finding.subject_name);
    haystack.push(' ');
    haystack.push_str(&finding.id);
    haystack.push(' ');
    collect_strings(&finding.evidence, &mut haystack);
    haystack
}

#[tokio::test]
async fn every_number_in_a_finding_comes_from_the_engine() {
    let dir = tempfile::tempdir().unwrap();
    let app = state(dir.path());
    let store = app.store.clone();
    let router = server::app(app);
    let doc = corpus();
    seed(&router, "coffee", &doc).await;

    let coverage_id = run_analysis(&router, "coffee", analyses::REQUIREMENT_COVERAGE, "main").await;
    let health_id = run_analysis(&router, "coffee", analyses::MODEL_HEALTH, "main").await;

    let mut whole_run_text = String::new();
    let mut engine_text = String::new();
    let mut allowed_counts: Vec<u64> = Vec::new();

    for id in [coverage_id.as_str(), health_id.as_str()] {
        let run = store.analysis_run("coffee", id).unwrap().unwrap();
        for finding in &run.findings {
            assert!(!finding.statement.is_empty());
            // EXHAUSTIVE: every run of digits in the statement is either one of the engine's
            // own numbers for this finding or a fragment of a string the engine supplied. A
            // number this frame invented cannot survive this check.
            let mut allowed: Vec<u64> = Vec::new();
            collect_numbers(&finding.evidence, &mut allowed);
            let haystack = engine_strings(finding);
            for run_of_digits in digit_runs(&finding.statement) {
                let as_number = run_of_digits.parse::<u64>().unwrap_or(u64::MAX);
                assert!(
                    allowed.contains(&as_number) || haystack.contains(&run_of_digits),
                    "the statement states {} which is neither an engine number {:?} nor part of an engine string; a finding may not contain a number the engine did not produce: {}",
                    run_of_digits,
                    allowed,
                    finding.statement
                );
            }
            collect_numbers(&finding.evidence, &mut allowed_counts);
            engine_text.push_str(&haystack);
            engine_text.push(' ');
            whole_run_text.push_str(&finding.statement);
            whole_run_text.push(' ');
        }

        // The headline is a template over the findings' own tallies.
        let sentence = analyses::summary_sentence(&run.definition, &run.findings, run.measured);
        whole_run_text.push_str(&sentence);
        whole_run_text.push(' ');
        allowed_counts.push(run.findings.len() as u64);
        allowed_counts.push(run.count(Severity::Gap) as u64);
        allowed_counts.push(run.count(Severity::Unknown) as u64);
        allowed_counts.push(run.count(Severity::Ok) as u64);
    }

    assert!(
        !digit_runs(&whole_run_text).is_empty(),
        "the check must have something to scan: {}",
        whole_run_text
    );
    for run_of_digits in digit_runs(&whole_run_text) {
        let as_number = run_of_digits.parse::<u64>().unwrap_or(u64::MAX);
        assert!(
            allowed_counts.contains(&as_number) || engine_text.contains(&run_of_digits),
            "the prose of a run states {} which is neither an engine count {:?} nor part of an engine string; an analysis may not state a number the engine did not produce:\n{}",
            run_of_digits,
            allowed_counts,
            whole_run_text
        );
    }
}

// ---------------------------------------------------------------------------
// 7. A definition the vocabulary cannot express is refused.

#[tokio::test]
async fn a_definition_the_vocabulary_cannot_express_is_refused_with_a_reason() {
    let dir = tempfile::tempdir().unwrap();
    let app = state(dir.path());
    let store = app.store.clone();
    let router = server::app(app);
    seed(&router, "coffee", &corpus()).await;

    let response = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/coffee/analyses",
            &[("definition", "make-the-coffee"), ("branch", "main")],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = body_text(response).await;
    assert!(
        body.contains("no analysis definition named 'make-the-coffee'"),
        "a refusal names the reason:\n{}",
        body
    );
    assert!(
        body.contains(analyses::REQUIREMENT_COVERAGE) && body.contains(analyses::MODEL_HEALTH),
        "the refusal names what this server CAN run:\n{}",
        body
    );
    assert!(
        store.analysis_runs("coffee").unwrap().is_empty(),
        "a refused analysis stores nothing"
    );

    let refusal = analyses::resolve("make-the-coffee").unwrap_err();
    assert!(refusal.reason.contains("make-the-coffee"));
}

// ---------------------------------------------------------------------------
// The edges: an empty library, and a measured model with no requirements.

#[tokio::test]
async fn the_library_over_a_project_with_no_runs_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    seed(&router, "coffee", &corpus()).await;

    let library = page(&router, "/ui/projects/coffee/analyses").await;
    assert!(library.contains("Library (0)"), "{}", library);
    assert!(
        library.contains("No analysis has been run over this project yet"),
        "an empty library is a state to act on, not a blank page:\n{}",
        library
    );
    assert!(
        !library.contains("Compare two runs"),
        "there is nothing to compare yet:\n{}",
        library
    );
}

#[tokio::test]
async fn a_model_with_no_requirements_is_measured_and_says_what_it_found() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    // A tiny document with a graph and no requirements: coverage is MEASURED, and its answer
    // is a sentence rather than an empty list.
    let mut doc = corpus();
    doc["requirements"] = serde_json::json!([]);
    seed(&router, "tiny", &doc).await;

    let id = run_analysis(&router, "tiny", analyses::REQUIREMENT_COVERAGE, "main").await;
    let page = page(&router, &format!("/ui/projects/tiny/analyses/{id}")).await;
    assert!(page.contains("the model declares no requirements, so there is nothing to cover"));
    assert!(page.contains("Measured clean (1)"));
    // The not-measured ROW is still present and says none - a measured model is not the same
    // statement as an unmeasured one, and the page says both explicitly.
    assert!(page.contains("Not measured (0)"));
    assert!(page.contains("None: every check this analysis names was measured."));
}
