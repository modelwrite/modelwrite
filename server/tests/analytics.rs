// SPDX-License-Identifier: AGPL-3.0-or-later
//! Contract tests for the Package 3 analytics REST surface: the schema document, every
//! table, the metrics and trend endpoints with the basis rule, determinism (byte-identical
//! CSV/NDJSON), streaming of the loss table, content negotiation, the OpenMetrics gate, and
//! the permission behaviour - a project outside the caller's scope is REFUSED, never silently
//! empty.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use server::auth::{AuthConfig, Identity};
use server::store::sqlite::SqliteStore;
use server::store::Store;
use server::{app, AppState};

fn corpus() -> Value {
    serde_json::from_str(&test_support::load_okf_expected()).expect("corpus fixture parses")
}

fn post(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn get_accept(uri: &str, accept: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header("accept", accept)
        .body(Body::empty())
        .unwrap()
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn text_body(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// Create the project and commit the real corpus to 'main', returning the router, the store,
/// the temp dir and the tip commit hash.
async fn setup() -> (axum::Router, Arc<SqliteStore>, tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    let router = app(AppState {
        store: store.clone(),
        evidence_dir: dir.path().to_path_buf(),
        auth: AuthConfig::Open,
    });
    assert_eq!(
        router
            .clone()
            .oneshot(post("/projects", json!({ "name": "coffee" })))
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            json!({ "branch": "main", "author": "alex", "message": "import the corpus", "okf": corpus() }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);
    let body = json_body(committed).await;
    let tip = body["hash"].as_str().unwrap().to_string();
    (router, store, dir, tip)
}

#[tokio::test]
async fn the_schema_document_lists_nine_tables_and_generated_definitions() {
    let (router, _store, _dir, _tip) = setup().await;
    let response = router.oneshot(get("/analytics/schema")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;

    assert_eq!(body["schemaVersion"], "mw-analytics-schema@1");
    let tables = body["tables"].as_array().unwrap();
    assert_eq!(tables.len(), 9);
    let names: Vec<&str> = tables.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        vec![
            "projects",
            "commits",
            "elements",
            "relationships",
            "requirements",
            "trace_links",
            "metrics",
            "metric_definitions",
            "import_losses",
        ]
    );
    // Column names are the at-rest snake_case logical names.
    let elements = tables.iter().find(|t| t["name"] == "elements").unwrap();
    assert_eq!(elements["columns"][1], "commit");
    assert_eq!(elements["columns"][2], "element_id");

    // 25 generated metric definitions, never hand-written.
    let definitions = body["metricDefinitions"].as_array().unwrap();
    assert_eq!(definitions.len(), 25);
    let first = &definitions[0];
    assert!(first["id"].as_str().unwrap().starts_with("graph."));
    assert_eq!(first["status"], "active");
    assert!(first["depends_on"].is_array());
}

#[tokio::test]
async fn every_table_reads_for_the_corpus_with_total_and_camelcase_rows() {
    let (router, _store, _dir, tip) = setup().await;

    // (table, expected total, a column that the camelCase JSON wire must carry)
    let cases: Vec<(&str, usize, &str)> = vec![
        ("projects", 1, "createdAt"),
        ("commits", 1, "okfHash"),
        ("elements", 58, "elementId"),
        ("relationships", 165, "source"),
        ("requirements", 25, "elementId"),
        ("metrics", 25, "metricId"),
        ("metric_definitions", 25, "dependsOn"),
        ("import_losses", 0, "importArtifactHash"),
    ];

    for (table, total, camel) in cases {
        let uri = format!(
            "/analytics/coffee/tables/{}?commit={}&limit=1000",
            table, tip
        );
        let response = router.clone().oneshot(get(&uri)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{} must read", table);
        let body = json_body(response).await;
        assert_eq!(body["schemaVersion"], "mw-analytics-schema@1", "{}", table);
        assert_eq!(body["table"], table);
        assert_eq!(body["commit"], tip);
        assert_eq!(body["total"], total, "{} row count", table);
        let rows = body["rows"].as_array().unwrap();
        assert_eq!(rows.len(), total, "{} page length", table);
        if total > 0 {
            assert!(
                rows[0].get(camel).is_some(),
                "{} must carry {} on the wire",
                table,
                camel
            );
        }
    }

    // trace_links is the coverage-edge subset of relationships; its count is the number of
    // Satisfy/Refine/Verify/Allocate dependency edges, which is non-zero for the corpus.
    let uri = format!(
        "/analytics/coffee/tables/trace_links?commit={}&limit=1000",
        tip
    );
    let response = router.oneshot(get(&uri)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert!(
        body["total"].as_u64().unwrap() > 0,
        "corpus has trace links"
    );
    let first = &body["rows"][0];
    for key in [
        "requirementId",
        "elementId",
        "linkKind",
        "sourceId",
        "targetId",
    ] {
        assert!(first.get(key).is_some(), "trace_links must carry {}", key);
    }
}

#[tokio::test]
async fn metrics_returns_25_rows_each_with_its_basis() {
    let (router, _store, _dir, tip) = setup().await;
    let uri = format!("/analytics/coffee/metrics?commit={}", tip);
    let response = router.oneshot(get(&uri)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["schemaVersion"], "mw-analytics-schema@1");
    assert_eq!(body["project"], "coffee");
    assert_eq!(body["commit"], tip);

    let metrics = body["metrics"].as_array().unwrap();
    assert_eq!(metrics.len(), 25);

    // The basis rule: every metric row carries its basis as data, never a bare number.
    for row in metrics {
        for key in [
            "value",
            "valueState",
            "unit",
            "trust",
            "basisElementCount",
            "basisRelationshipCount",
            "constructsNotCarried",
            "basisNote",
            "engineVersion",
            "evidenceHash",
        ] {
            assert!(
                row.get(key).is_some(),
                "metric {} must carry {}, got {}",
                row["metricId"],
                key,
                row
            );
        }
        assert_eq!(row["trust"], "measured");
        assert_eq!(row["valueState"], "present");
    }

    // The known corpus values: 25 requirements, 15 covered, 10 uncovered.
    let get = |id: &str| {
        metrics.iter().find(|r| r["metricId"] == id).unwrap()["value"]
            .as_i64()
            .unwrap()
    };
    assert_eq!(get("coverage.total"), 25);
    assert_eq!(get("coverage.covered"), 15);
    assert_eq!(get("coverage.uncovered"), 10);
    assert_eq!(get("graph.node_count"), 99);
    assert_eq!(get("graph.edge_count"), 165);
}

#[tokio::test]
async fn trend_returns_the_metric_across_commits_in_order_with_basis() {
    let (router, _store, _dir, first) = setup().await;
    // A second commit of the same corpus gives a second baseline on the branch.
    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            json!({ "branch": "main", "author": "alex", "message": "second baseline", "okf": corpus() }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);
    let second = json_body(committed).await["hash"]
        .as_str()
        .unwrap()
        .to_string();

    let response = router
        .clone()
        .oneshot(get(
            "/analytics/coffee/trend?metric=coverage.covered&branch=main",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["schemaVersion"], "mw-analytics-schema@1");
    assert_eq!(body["metric"], "coverage.covered");
    assert_eq!(body["branch"], "main");

    let rows = body["metrics"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["commit"], first.as_str());
    assert_eq!(rows[1]["commit"], second.as_str());
    for row in rows {
        assert_eq!(row["metricId"], "coverage.covered");
        assert!(row["basisElementCount"].is_number());
        assert!(row["evidenceHash"].is_string());
    }

    // An unknown metric is a bad request, named.
    let bad = router
        .oneshot(get("/analytics/coffee/trend?metric=nope&branch=main"))
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn csv_and_ndjson_are_byte_identical_across_two_runs() {
    let (router, _store, _dir, tip) = setup().await;

    for format in ["csv", "ndjson"] {
        let uri = format!(
            "/analytics/coffee/tables/elements?commit={}&format={}&limit=1000",
            tip, format
        );
        let first = router.clone().oneshot(get(&uri)).await.unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        let a = text_body(first).await;
        let second = router.clone().oneshot(get(&uri)).await.unwrap();
        let b = text_body(second).await;
        assert_eq!(a, b, "{} must be byte-identical across runs", format);
        assert!(!a.is_empty(), "{} must not be empty", format);
    }

    // CSV keeps the at-rest snake_case header and is deterministic.
    let csv_uri = format!(
        "/analytics/coffee/tables/elements?commit={}&format=csv&limit=1000",
        tip
    );
    let csv = text_body(router.clone().oneshot(get(&csv_uri)).await.unwrap()).await;
    assert!(
        csv.starts_with(
            "project,commit,element_id,section,name,kind,stereotypes,attributes,documentation\n"
        ),
        "CSV header must be snake_case at rest, got: {}",
        csv.lines().next().unwrap_or("")
    );
}

#[tokio::test]
async fn a_project_outside_the_callers_scope_is_refused_not_empty() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    store.create_project("coffee", None).unwrap();
    let bytes = test_support::load_okf_expected().into_bytes();
    let okf_hash = store.put_blob(&bytes).unwrap();
    store
        .commit_model(
            "coffee", "main", &okf_hash, "alex", "import", None, None, None,
        )
        .unwrap();

    // A viewer who may read, but only the "tea" project - "coffee" is out of scope.
    let scoped = Identity {
        subject: "viewer".to_string(),
        roles: vec!["viewer".to_string()],
        projects: vec!["tea".to_string()],
        trial_id: None,
    };
    let router = app(AppState {
        store: store.clone(),
        evidence_dir: dir.path().to_path_buf(),
        auth: AuthConfig::fixed(scoped),
    });

    for uri in [
        "/analytics/coffee/tables/elements".to_string(),
        "/analytics/coffee/metrics".to_string(),
        "/analytics/coffee/trend?metric=coverage.covered&branch=main".to_string(),
    ] {
        let response = router.clone().oneshot(get(&uri)).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "{} must refuse",
            uri
        );
        let body = json_body(response).await;
        assert_eq!(body["error"], "project not in scope");
    }

    // The schema endpoint needs only Read (no project), so the scoped viewer may read it.
    let schema = router.oneshot(get("/analytics/schema")).await.unwrap();
    assert_eq!(schema.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_caller_without_read_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    store.create_project("coffee", None).unwrap();
    let bytes = test_support::load_okf_expected().into_bytes();
    let okf_hash = store.put_blob(&bytes).unwrap();
    store
        .commit_model(
            "coffee", "main", &okf_hash, "alex", "import", None, None, None,
        )
        .unwrap();

    let nobody = Identity {
        subject: "nobody".to_string(),
        roles: Vec::new(),
        projects: vec!["*".to_string()],
        trial_id: None,
    };
    let router = app(AppState {
        store: store.clone(),
        evidence_dir: dir.path().to_path_buf(),
        auth: AuthConfig::fixed(nobody),
    });

    for uri in [
        "/analytics/schema".to_string(),
        "/analytics/coffee/tables/elements".to_string(),
        "/analytics/coffee/metrics".to_string(),
    ] {
        let response = router.clone().oneshot(get(&uri)).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "{} must refuse",
            uri
        );
    }
}

#[tokio::test]
async fn equality_filters_and_pagination_work() {
    let (router, _store, _dir, tip) = setup().await;

    // Filter requirements to the covered ones: covered=true does not match (boolean column),
    // but filtering by kind or an identifier works. Filter by section on elements is a key
    // column equality filter.
    let filtered = router
        .clone()
        .oneshot(get(&format!(
            "/analytics/coffee/tables/elements?commit={}&section=signals&limit=1000",
            tip
        )))
        .await
        .unwrap();
    assert_eq!(filtered.status(), StatusCode::OK);
    let body = json_body(filtered).await;
    assert_eq!(body["total"], 9, "9 signal elements");
    assert!(body["rows"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["section"] == "signals"));

    // Pagination: limit 10 over 58 elements leaves a nextCursor, and pages do not overlap.
    let page1 = router
        .clone()
        .oneshot(get(&format!(
            "/analytics/coffee/tables/elements?commit={}&limit=10",
            tip
        )))
        .await
        .unwrap();
    let p1 = json_body(page1).await;
    assert_eq!(p1["rows"].as_array().unwrap().len(), 10);
    assert_eq!(p1["total"], 58);
    let cursor = p1["nextCursor"].as_str().unwrap().to_string();
    assert_eq!(cursor, "10");

    let page2 = router
        .clone()
        .oneshot(get(&format!(
            "/analytics/coffee/tables/elements?commit={}&limit=10&cursor={}",
            tip, cursor
        )))
        .await
        .unwrap();
    let p2 = json_body(page2).await;
    assert_eq!(p2["rows"].as_array().unwrap().len(), 10);
    let first_id = p1["rows"][0]["elementId"].as_str().unwrap();
    let second_first_id = p2["rows"][0]["elementId"].as_str().unwrap();
    assert_ne!(first_id, second_first_id, "pages must not overlap");

    // An unknown filter column is a bad request, not a silent empty page.
    let bad = router
        .oneshot(get(&format!(
            "/analytics/coffee/tables/elements?commit={}&nope=x",
            tip
        )))
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_accept_header_negotiates_format() {
    let (router, _store, _dir, tip) = setup().await;
    let uri = format!("/analytics/coffee/tables/elements?commit={}&limit=5", tip);

    let csv = router
        .clone()
        .oneshot(get_accept(&uri, "text/csv"))
        .await
        .unwrap();
    assert_eq!(csv.status(), StatusCode::OK);
    assert_eq!(csv.headers().get("content-type").unwrap(), "text/csv");
    let csv_text = text_body(csv).await;
    assert!(
        csv_text.starts_with("project,commit,element_id,"),
        "got: {}",
        csv_text.lines().next().unwrap_or("")
    );

    let ndjson = router
        .clone()
        .oneshot(get_accept(&uri, "application/x-ndjson"))
        .await
        .unwrap();
    let nd_text = text_body(ndjson).await;
    assert!(nd_text.lines().next().unwrap().contains("element_id"));

    let json = router
        .clone()
        .oneshot(get_accept(&uri, "application/json"))
        .await
        .unwrap();
    let json_body_v = json_body(json).await;
    assert_eq!(json_body_v["table"], "elements");
    assert!(json_body_v["rows"][0].get("elementId").is_some());
}

#[tokio::test]
async fn openmetrics_is_off_by_default_and_on_when_enabled() {
    let (router, _store, _dir, _tip) = setup().await;
    std::env::remove_var("MW_ENABLE_OPENMETRICS");

    let off = router.clone().oneshot(get("/metrics")).await.unwrap();
    assert_eq!(off.status(), StatusCode::SERVICE_UNAVAILABLE);

    std::env::set_var("MW_ENABLE_OPENMETRICS", "1");
    let on = router.clone().oneshot(get("/metrics")).await.unwrap();
    assert_eq!(on.status(), StatusCode::OK);
    let text = text_body(on).await;
    for gauge in [
        "mw_requirements_total",
        "mw_requirements_uncovered",
        "mw_requirements_coverage_ratio",
        "mw_graph_orphans",
        "mw_import_blocking_losses",
        "mw_elements",
    ] {
        assert!(text.contains(gauge), "must expose {}: {}", gauge, text);
    }
    assert!(text.contains("project=\"coffee\",branch=\"main\""));
    assert!(text.contains("mw_requirements_total{project=\"coffee\",branch=\"main\"} 25"));
    std::env::remove_var("MW_ENABLE_OPENMETRICS");
}

#[tokio::test]
async fn import_losses_streams_for_an_imported_commit() {
    // Import the small SysML v2 drone fixture (a real viewer import with named losses), accept
    // every blocking loss, and read the loss table back through the streaming path.
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("mw.db")).unwrap());
    let router = app(AppState {
        store: store.clone(),
        evidence_dir: dir.path().to_path_buf(),
        auth: AuthConfig::Open,
    });
    assert_eq!(
        router
            .clone()
            .oneshot(post("/projects", json!({ "name": "drone" })))
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );

    let artifact = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/Drone_BaseArchitecture.sysml"
    ))
    .unwrap();

    let refused = router
        .clone()
        .oneshot(post(
            "/projects/drone/import",
            json!({ "binding": "sysml-v2-textual@1.0", "branch": "main", "author": "alex", "message": "import drone", "artifact": artifact }),
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let blocking = json_body(refused).await["blocking"]
        .as_array()
        .unwrap()
        .clone();
    let accept: Vec<String> = blocking
        .iter()
        .map(|m| {
            format!(
                "{} [{}]",
                m["subject"].as_str().unwrap(),
                m["verdict"].as_str().unwrap().to_lowercase()
            )
        })
        .collect();

    let committed = router
        .clone()
        .oneshot(post(
            "/projects/drone/import",
            json!({ "binding": "sysml-v2-textual@1.0", "branch": "main", "author": "alex", "message": "import drone", "artifact": artifact, "acceptLosses": accept }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);

    // The loss table streams: one row per mapping, with the blocking losses named.
    let response = router
        .clone()
        .oneshot(get("/analytics/drone/tables/import_losses?limit=1000"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["table"], "import_losses");
    assert_eq!(body["schemaVersion"], "mw-analytics-schema@1");
    let total = body["total"].as_u64().unwrap();
    assert!(total > 0, "the drone import has loss mappings");
    let rows = body["rows"].as_array().unwrap();
    assert_eq!(rows.len() as u64, total);
    for row in rows {
        assert!(row["importArtifactHash"].is_string());
        assert!(row["severity"].is_string());
        assert!(row["subject"].is_string());
    }

    // The metrics for that imported commit carry constructsNotCarried > 0 (content losses).
    let metrics_resp = router
        .oneshot(get("/analytics/drone/metrics"))
        .await
        .unwrap();
    let metrics_body = json_body(metrics_resp).await;
    let row = metrics_body["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["metricId"] == "coverage.total")
        .unwrap();
    assert!(
        row["constructsNotCarried"].as_i64().unwrap() > 0,
        "an imported commit with content losses must carry constructsNotCarried > 0"
    );
}
