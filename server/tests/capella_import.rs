// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Capella/Arcadia binding wired into the server as a VIEWER (ImportOnly).
//!
//! This is the same three facts server/tests/sysmlv2_import.rs pins for the SysML v2
//! viewer, plus the two things that are Capella's own:
//!
//!   1. The registry lists capella-arcadia@1.0 with Direction::ImportOnly, so the
//!      workbench import page offers THREE bindings (sysml-v1-xmi@2.4,
//!      sysml-v2-textual@1.0, capella-arcadia@1.0) and marks the two viewers as
//!      viewers. The generated binding list on the site is therefore TRUE.
//!   2. Importing a real .capella model returns OKF plus its NAMED loss report: the
//!      constructs outside the reader's subset come back as Unmappable entries naming
//!      the element type, id and name, the packages/ports/edge ids that cannot be
//!      carried come back as named Lossy drops, and a viewer import records
//!      `fidelity: null` (no round trip was measured) rather than a fabricated diff.
//!   3. An export or round trip through the viewer is REFUSED with the direction
//!      named, never a panic or a 500.
//!
//! A Capella model is a FOLDER (the .capella semantic model, the .aird diagram layer,
//! the .afm metadata), but only the .capella file holds the semantic model and only it
//! is read. The file to upload or drop is therefore that ONE file: no zip of the folder
//! is required, and the drop zone detects it from the bytes (a .capella is XMI whose
//! root is a capellamodeller:Project) rather than from the extension.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use binding::Direction;
use http_body_util::BodyExt;
use server::store::Store;
use tower::ServiceExt;

fn state(dir: &std::path::Path) -> server::AppState {
    let store = server::store::sqlite::SqliteStore::open(&dir.join("mw.db")).unwrap();
    server::AppState {
        store: std::sync::Arc::new(store),
        evidence_dir: dir.to_path_buf(),
        auth: server::auth::AuthConfig::Open,
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

fn post_multipart(uri: &str, content_type: &str, body: Vec<u8>) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", content_type)
        .body(Body::from(body))
        .unwrap()
}

/// A multipart/form-data body carrying the dropped file, built by hand so the test drives
/// the same streaming intake a browser's form post does.
fn multipart_drop(file_name: &str, artifact: &[u8]) -> (String, Vec<u8>) {
    let boundary = "mw-capella-boundary";
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"artifact\"; filename=\"{}\"\r\n",
            file_name
        )
        .as_bytes(),
    );
    body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
    body.extend_from_slice(artifact);
    body.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());
    (format!("multipart/form-data; boundary={}", boundary), body)
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn decode_entities(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Every `<option value="...">label</option>` in a rendered page, in document order.
fn options(html: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find("<option value=\"") {
        let after = &rest[at + "<option value=\"".len()..];
        let Some(end) = after.find('"') else { break };
        let value = after[..end].to_string();
        let Some(gt) = after.find('>') else { break };
        let tail = &after[gt + 1..];
        let Some(close) = tail.find("</option>") else {
            break;
        };
        out.push((value, decode_entities(tail[..close].trim())));
        rest = &tail[close..];
    }
    out
}

async fn create_project(router: &axum::Router, name: &str) {
    let response = router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": name })))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
}

/// The committed HAND-WRITTEN fixture. A Capella model is a folder, but this is the one
/// file of it that holds the semantic model - the file a person uploads or drops.
fn fixture() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/aeb-control-structure.capella"
    ))
    .expect("fixture must exist")
}

/// The REAL vendor model, fetched OUTSIDE the repository (EPL-2.0 is not
/// AGPL-compatible, so it is never committed). Located exactly as
/// engine/binding-capella/tests/real_capella.rs locates it; None means SKIP, loudly.
fn real_model() -> Option<Vec<u8>> {
    if let Ok(p) = std::env::var("CAPELLA_IFE_CAPELLA") {
        return Some(std::fs::read(&p).unwrap_or_else(|e| panic!("cannot read {p}: {e}")));
    }
    let candidates = [
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../modelwrite-capella-corpus/ife-variant/In-Flight Entertainment System.capella"
        ),
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../sample/examples/arcadia-capella/dbinfrago-ife-variant/In-Flight Entertainment System.capella"
        ),
    ];
    for c in candidates {
        if let Ok(bytes) = std::fs::read(c) {
            return Some(bytes);
        }
    }
    None
}

fn skip_real_model() {
    eprintln!("SKIPPED: the Capella corpus is not fetched (EPL-2.0, never bundled).");
    eprintln!("Fetch it and set CAPELLA_IFE_CAPELLA to the .capella file, or run the");
    eprintln!("sample/examples fetch script, then run this test again.");
}

// ---------------------------------------------------------------------------
// 1. The registry list, and the page that renders it.

#[test]
fn the_capella_viewer_is_in_the_registry_list() {
    let bindings = server::binding_registry::bindings();
    let viewer = bindings
        .iter()
        .find(|b| b.id == "capella-arcadia")
        .expect("capella-arcadia must be registered");
    assert_eq!(viewer.version, "1.0");
    assert_eq!(viewer.direction, Direction::ImportOnly);
    assert!(
        viewer.description.contains(".capella"),
        "the description names the notation, got: {}",
        viewer.description
    );
    // The registry name matches the crate's own binding id, so the workbench, the
    // gate and the generated docs all name the same binding.
    assert_eq!(viewer.id, binding_capella::BINDING_ID);
    assert_eq!(viewer.version, binding_capella::BINDING_VERSION);
}

#[tokio::test]
async fn the_import_page_offers_three_bindings_and_marks_the_viewers() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "aeb").await;

    let response = router
        .clone()
        .oneshot(get("/ui/projects/aeb/import"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    // The page renders the registry list twice (the upload form and the paste form), so
    // dedupe the options: what matters is WHICH bindings are offered and how they read.
    let mut seen: Vec<(String, String)> = Vec::new();
    for option in options(&html) {
        if !seen.iter().any(|(value, _)| *value == option.0) {
            seen.push(option);
        }
    }
    println!("=== THE THREE BINDINGS AS THE IMPORT PAGE LISTS THEM ===");
    for (value, label) in &seen {
        println!("{} -> {}", value, label);
    }
    let ids: Vec<&str> = seen.iter().map(|(value, _)| value.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "sysml-v1-xmi@2.4",
            "sysml-v2-textual@1.0",
            "capella-arcadia@1.0"
        ],
        "the import page must offer exactly the three registered bindings, in registry order"
    );
    let capella = seen
        .iter()
        .find(|(value, _)| value == "capella-arcadia@1.0")
        .unwrap();
    assert!(
        capella.1.ends_with("(viewer: reads, does not write back)"),
        "capella must be marked a viewer, got: {}",
        capella.1
    );
    assert!(capella.1.contains("capella-arcadia@1.0"));
    // The read/write binding is NOT marked a viewer; the SysML v2 viewer IS.
    let xmi = seen
        .iter()
        .find(|(value, _)| value == "sysml-v1-xmi@2.4")
        .unwrap();
    assert!(!xmi.1.contains("viewer"), "got: {}", xmi.1);
    let sysmlv2 = seen
        .iter()
        .find(|(value, _)| value == "sysml-v2-textual@1.0")
        .unwrap();
    assert!(sysmlv2.1.contains("(viewer: reads, does not write back)"));
}

// ---------------------------------------------------------------------------
// 2. A real import through the server: OKF plus named losses, fidelity null.

#[tokio::test]
async fn importing_a_capella_model_returns_okf_and_named_losses() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "aeb").await;

    let artifact = String::from_utf8(fixture()).expect("the fixture is UTF-8");

    // First import without acceptance: the viewer's blocking losses come back BY NAME.
    let refused = router
        .clone()
        .oneshot(post(
            "/projects/aeb/import",
            serde_json::json!({
                "binding": "capella-arcadia@1.0",
                "branch": "main",
                "author": "alex",
                "message": "import the AEB control structure",
                "artifact": artifact
            }),
        ))
        .await
        .unwrap();
    assert_eq!(
        refused.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:?}",
        refused
    );
    let refused_body = json_body(refused).await;
    let blocking = refused_body["blocking"].as_array().expect("blocking array");
    let subjects: Vec<&str> = blocking
        .iter()
        .map(|m| m["subject"].as_str().unwrap())
        .collect();

    // The constructs outside the subset are named, with their Capella type and id.
    for expected in [
        "Part part-wheel",
        "StateMachine sm-brake",
        "State st-standby",
        "TransfoLink tl-1",
    ] {
        assert!(
            subjects.contains(&expected),
            "the named content loss {expected:?} is missing from {subjects:?}"
        );
    }
    // The package flattening, the port ids and the edge/allocation ids are named drops.
    assert!(subjects.iter().any(|s| s.starts_with("SystemFunctionPkg")));
    assert!(subjects
        .iter()
        .any(|s| s.starts_with("FunctionOutputPort p-detect-out")));
    assert!(subjects
        .iter()
        .any(|s| s.starts_with("FunctionalExchange fx-brake")));
    assert!(subjects
        .iter()
        .any(|s| s.starts_with("ComponentFunctionalAllocation alloc-detect")));

    let unmappable = blocking
        .iter()
        .filter(|m| m["verdict"] == "Unmappable")
        .count();
    let lossy = blocking.iter().filter(|m| m["verdict"] == "Lossy").count();
    assert_eq!(unmappable, 4, "four named content losses, got {subjects:?}");
    assert_eq!(lossy, 8, "eight named drops, got {subjects:?}");

    // Accept every blocking loss by its entry identity, exactly as the workbench form does.
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
            "/projects/aeb/import",
            serde_json::json!({
                "binding": "capella-arcadia@1.0",
                "branch": "main",
                "author": "alex",
                "message": "import the AEB control structure",
                "artifact": artifact,
                "acceptLosses": accept
            }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED, "{:?}", committed);
    let body = json_body(committed).await;

    // The named loss report travels with the commit and states the binding's direction.
    let report = &body["lossReport"];
    assert_eq!(report["binding"]["id"], "capella-arcadia");
    assert_eq!(report["binding"]["version"], "1.0");
    assert_eq!(report["binding"]["direction"], "ImportOnly");
    let mappings = report["mappings"].as_array().expect("loss mappings");
    // The carried components and functions are NOT losses: no Unmappable entry may name
    // a SystemComponent or a SystemFunction, because those are exactly what the reader
    // carries.
    assert!(
        mappings.iter().all(|m| {
            let subject = m["subject"].as_str().unwrap();
            !(subject.starts_with("SystemComponent") || subject.starts_with("SystemFunction"))
                || m["verdict"] != "Unmappable"
        }),
        "a carried component/function must never be reported Unmappable: {mappings:?}"
    );

    // A viewer has no round trip to measure, so fidelity is null - never a fake diff.
    assert_eq!(body["fidelity"], serde_json::Value::Null);

    // The import provenance names the binding it migrated through.
    let provenance = &body["commit"]["provenance"];
    assert_eq!(provenance["kind"], "imported");
    assert_eq!(provenance["bindingId"], "capella-arcadia");
    assert_eq!(provenance["bindingVersion"], "1.0");

    // The imported OKF is committed: five blocks (two components, three functions),
    // one system constraint, six graph nodes and five relationships.
    let commit_hash = body["commit"]["hash"].as_str().expect("commit hash");
    let store = server::store::sqlite::SqliteStore::open(&dir.path().join("mw.db")).unwrap();
    let commit = store.commit("aeb", commit_hash).unwrap().expect("commit");
    let okf_bytes = store.blob(&commit.okf_hash).unwrap().expect("okf blob");
    let okf: okf::types::OkfRoot = serde_json::from_slice(&okf_bytes).unwrap();

    let graph = okf
        .graph
        .as_ref()
        .expect("the imported model carries a graph");
    let contains = graph.edges.iter().filter(|e| e.kind == "contains").count();
    let parts = graph.edges.iter().filter(|e| e.kind == "part").count();
    let deps = graph
        .edges
        .iter()
        .filter(|e| e.kind == "dependency")
        .count();
    println!("=== THE IMPORT THROUGH THE SERVER, VERBATIM ===");
    println!("project            : {}", okf.project);
    println!("structure blocks   : {}", okf.structure.len());
    println!("requirements       : {}", okf.requirements.len());
    println!("graph nodes        : {}", graph.nodes.len());
    println!("graph edges        : {}", graph.edges.len());
    println!("  contains         : {contains}");
    println!("  part (allocated) : {parts}");
    println!("  dependency (flow): {deps}");
    println!("named content losses (Unmappable): {unmappable}");
    println!("named drops (Lossy)              : {lossy}");
    println!("blocking losses total            : {}", blocking.len());
    println!("fidelity                         : {}", body["fidelity"]);

    assert_eq!(okf.project, "AEB Control Structure");
    assert_eq!(okf.structure.len(), 5, "two components and three functions");
    assert_eq!(okf.requirements.len(), 1, "system constraint SC-1");
    assert_eq!(graph.nodes.len(), 6, "five blocks plus the constraint");
    assert_eq!(graph.edges.len(), 5);
    assert_eq!(contains, 1, "the nested function is a contains edge");
    assert_eq!(parts, 2, "two ComponentFunctionalAllocations");
    assert_eq!(deps, 2, "two FunctionalExchanges");
    assert_eq!(blocking.len(), 12, "four Unmappable plus eight Lossy");

    // The constraint's OpaqueExpression body is carried as the requirement text.
    let req = &okf.requirements[0];
    assert_eq!(req.name, "SC-1");
    assert_eq!(req.stereotypes, vec!["constraint".to_string()]);
    assert!(req.req_text.contains("never command a brake application"));
}

// ---------------------------------------------------------------------------
// 2b. The REAL vendor model, through the server, when the corpus is fetched.

#[tokio::test]
async fn the_real_capella_model_imports_through_the_server() {
    let Some(bytes) = real_model() else {
        skip_real_model();
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    create_project(&router, "ife").await;

    let artifact = String::from_utf8(bytes.clone()).expect("the .capella file is UTF-8");
    let refused = router
        .clone()
        .oneshot(post(
            "/projects/ife/import",
            serde_json::json!({
                "binding": "capella-arcadia@1.0",
                "branch": "main",
                "author": "alex",
                "message": "import the In-Flight Entertainment System",
                "artifact": artifact
            }),
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let refused_body = json_body(refused).await;
    let blocking = refused_body["blocking"].as_array().expect("blocking array");
    let unmappable = blocking
        .iter()
        .filter(|m| m["verdict"] == "Unmappable")
        .count();
    let lossy = blocking.iter().filter(|m| m["verdict"] == "Lossy").count();

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
            "/projects/ife/import",
            serde_json::json!({
                "binding": "capella-arcadia@1.0",
                "branch": "main",
                "author": "alex",
                "message": "import the In-Flight Entertainment System",
                "artifact": artifact,
                "acceptLosses": accept
            }),
        ))
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED, "{:?}", committed);
    let body = json_body(committed).await;
    assert_eq!(body["fidelity"], serde_json::Value::Null);

    let commit_hash = body["commit"]["hash"].as_str().expect("commit hash");
    let store = server::store::sqlite::SqliteStore::open(&dir.path().join("mw.db")).unwrap();
    let commit = store.commit("ife", commit_hash).unwrap().expect("commit");
    let okf_bytes = store.blob(&commit.okf_hash).unwrap().expect("okf blob");
    let okf: okf::types::OkfRoot = serde_json::from_slice(&okf_bytes).unwrap();
    let graph = okf.graph.as_ref().expect("graph");
    println!(
        "=== THE REAL IFE MODEL (dbinfrago/Capella-IFE-sample, EPL-2.0), THROUGH THE SERVER ==="
    );
    println!("project             : {}", okf.project);
    println!("structure blocks    : {}", okf.structure.len());
    println!("requirements        : {}", okf.requirements.len());
    println!("graph nodes         : {}", graph.nodes.len());
    println!("graph edges         : {}", graph.edges.len());
    println!("content losses      : {unmappable}");
    println!("lossy drops         : {lossy}");
    println!("fidelity            : {}", body["fidelity"]);
    assert!(okf.structure.len() > 100, "the real model carries content");
    assert!(graph.edges.len() > 100, "the real model carries edges");
}

// ---------------------------------------------------------------------------
// 3. An export or round trip through the viewer is refused, with the direction named.

#[test]
fn export_and_round_trip_through_the_capella_viewer_are_refused_with_the_direction_named() {
    let binding =
        server::binding_registry::resolve("capella-arcadia", "1.0").expect("the viewer resolves");

    let root: okf::types::OkfRoot = serde_json::from_value(serde_json::json!({
        "project": "Demo",
        "summary": {},
        "stateMachine": { "name": "sm", "regions": [] },
        "graph": { "nodes": [], "edges": [] }
    }))
    .unwrap();

    // Export is refused with a message naming the viewer and its direction. The
    // SURFACED text is the BindingError's Display - the string every caller that renders a
    // binding failure hands to a person - so print and pin THAT, not the inner field.
    match binding.export(&root) {
        Err(binding::BindingError::Export(message)) => {
            let surfaced = binding::BindingError::Export(message.clone()).to_string();
            println!("=== THE REFUSAL WORDING (as Display surfaces it) ===");
            println!("{}", surfaced);
            assert!(message.contains("ImportOnly"), "message: {}", message);
            assert!(message.contains("viewer"), "message: {}", message);
            assert!(
                surfaced.contains("ImportOnly") && surfaced.contains("viewer"),
                "the surfaced text names the direction: {surfaced}"
            );
        }
        other => panic!("expected an Export refusal, got {:?}", other),
    }

    // The round-trip harness refuses a viewer outright, naming its id.
    let source = serde_json::to_vec(&root).unwrap();
    match binding::round_trip(binding.as_ref(), &source) {
        Err(binding::BindingError::Viewer { id }) => {
            println!("=== THE ROUND-TRIP REFUSAL WORDING ===");
            println!("{}", binding::BindingError::Viewer { id: id.clone() });
            assert_eq!(id, "capella-arcadia");
        }
        other => panic!("expected a Viewer refusal, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// 4. The drop-zone front door detects a .capella from its BYTES.

#[tokio::test]
async fn a_dropped_capella_model_is_detected_by_its_bytes_and_read_by_the_viewer() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let bytes = fixture();

    // The detection itself, on the bytes (not the name): a .capella is XMI, and the
    // Capella root has to win over the XMI markers or an Arcadia model would be read
    // as UML.
    assert_eq!(
        server::ui::dropzone::detect(&bytes, "AEB.capella"),
        server::ui::dropzone::Format::Capella
    );
    assert_eq!(
        server::ui::dropzone::detect(&bytes, "renamed.txt"),
        server::ui::dropzone::Format::Capella
    );

    let (content_type, body) = multipart_drop("aeb-control-structure.capella", &bytes);
    let response = router
        .clone()
        .oneshot(post_multipart("/onboard", &content_type, body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let html = body_text(response).await;
    assert!(
        html.contains("detected a Capella/Arcadia .capella semantic model"),
        "the result page must name the detected format, got a page without it"
    );
    assert!(html.contains("capella-arcadia@1.0"));
    assert!(
        html.contains("viewer"),
        "the refusal page must state the viewer direction"
    );
    // The project is named after the dropped file, with no project box filled in.
    assert!(html.contains("/ui/projects/aeb-control-structure/health"));
}
