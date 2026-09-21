// SPDX-License-Identifier: AGPL-3.0-or-later
//! E1 - the drop-zone front door, end to end.
//!
//! These tests drive the real front door with a REAL model: the 193 KB MagicDraw export in
//! real-world/mdzip/model.xmi, the coffee-machine Cameo model the project's own binding tests
//! use. They assert the three things E1 cannot ship without:
//!
//! * a drop completes the whole flow - detect, create the project from the file name, retain
//!   the artifact, import, and render a plain-language result;
//! * every number in that plain-language summary is a number the ENGINE produced, checked
//!   against the binding's own loss report, the imported document and the engine's own
//!   round-trip diff - the summary may not carry a count the engine did not measure;
//! * the same facts render with no reasoner configured, which is the public showcase's mode,
//!   and the flow completes from an ordinary form post with no script at all.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use server::binding_registry;
use server::store::Store;
use server::ui::dropzone;

// ---------------------------------------------------------------------------
// Harness.

fn state(dir: &std::path::Path) -> server::AppState {
    let store = server::store::sqlite::SqliteStore::open(&dir.join("mw.db")).unwrap();
    server::AppState {
        store: std::sync::Arc::new(store),
        evidence_dir: dir.to_path_buf(),
        auth: server::auth::AuthConfig::Open,
    }
}

/// The real MagicDraw export: the fastest real case, and the one the task names.
fn real_model() -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../real-world/mdzip/model.xmi");
    std::fs::read(path).unwrap_or_else(|e| panic!("cannot read {}: {}", path, e))
}

/// A multipart/form-data body carrying the dropped file, built by hand so the test drives the
/// same streaming intake a browser's form post does.
fn multipart_drop(file_name: &str, artifact: &[u8]) -> (String, Vec<u8>) {
    let boundary = "mw-dropzone-boundary";
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

fn post_multipart(uri: &str, content_type: &str, body: Vec<u8>) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", content_type)
        .body(Body::from(body))
        .unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

/// A urlencoded form post, exactly what the browser sends when the one button is clicked with
/// no script anywhere in sight.
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

/// The value a browser ACTUALLY submits for a newline-joined hidden field. The HTML
/// form-encoding algorithm rewrites every LF as CRLF, so the server must accept that spelling:
/// this is the difference that made the one-button acceptance refuse itself in a real browser
/// while a hand-built test body (LF only) passed.
fn browser_all_losses(all_losses: &str) -> String {
    all_losses.replace('\n', "\r\n")
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

fn decode_entities(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// The p-element bodies inside the element carrying the marker, which is how the summary the
/// person reads is lifted back out of the rendered page.
fn section_paragraphs(html: &str, marker: &str) -> Vec<String> {
    let Some(start) = html.find(marker) else {
        return Vec::new();
    };
    let rest = &html[start..];
    let end = rest.find("</section>").unwrap_or(rest.len());
    let body = &rest[..end];
    let mut out = Vec::new();
    let mut cursor = 0;
    while let Some(open) = body[cursor..].find("<p>") {
        let abs = cursor + open + 3;
        let Some(close) = body[abs..].find("</p>") else {
            break;
        };
        out.push(decode_entities(&body[abs..abs + close]));
        cursor = abs + close;
    }
    out
}

/// The value of a hidden input, as it travels in the form the browser would submit.
fn hidden_value(html: &str, name: &str) -> Option<String> {
    let needle = format!("name=\"{}\" value=\"", name);
    let start = html.find(&needle)? + needle.len();
    let end = html[start..].find('"')? + start;
    Some(decode_entities(&html[start..end]))
}

/// The commit hash out of the first href that contains the given fragment.
fn link_value(html: &str, fragment: &str) -> Option<String> {
    let start = html.find(fragment)? + fragment.len();
    let rest = &html[start..];
    let end = rest.find('"')?;
    Some(decode_entities(&rest[..end]))
}

/// Every maximal run of ASCII digits in a piece of prose. Proof 2 uses this to ask, of each one,
/// whether the engine produced it.
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

/// The engine's own reading of the real model, computed here in the test rather than read back
/// out of the page: this is the reference every number in the summary is checked against.
struct EngineReading {
    root: okf::types::OkfRoot,
    blocking_total: usize,
    unmappable: usize,
    lossy: usize,
    round_trip_equal: bool,
}

fn read_with_the_engine(bytes: &[u8]) -> EngineReading {
    let binding = binding_registry::resolve(binding_xmi::BINDING_ID, binding_xmi::BINDING_VERSION)
        .expect("the XMI binding must be registered");
    let (root, report) = binding
        .import(bytes)
        .expect("the real MagicDraw export must import");
    let source = serde_json::to_vec(&root).expect("the imported document must serialise");
    let outcome = binding::round_trip(binding.as_ref(), &source)
        .expect("the engine must be able to measure the binding's own round trip");
    EngineReading {
        blocking_total: report.blocking().len(),
        unmappable: report.content_losses().len(),
        lossy: report.lossy().len(),
        round_trip_equal: outcome.diff.equal,
        root,
    }
}

/// The facts the page must be speaking, derived from the engine by the test itself.
fn expected_facts(bytes: &[u8], reading: &EngineReading) -> dropzone::OnboardFacts {
    let binding =
        binding_registry::resolve(binding_xmi::BINDING_ID, binding_xmi::BINDING_VERSION).unwrap();
    let (_, report) = binding.import(bytes).unwrap();
    let format = dropzone::detect(bytes, "model.xmi");
    dropzone::OnboardFacts::from_engine(dropzone::FactsInput {
        format_label: format.label(),
        binding: "sysml-v1-xmi@2.4",
        direction: binding.info().direction,
        root: &reading.root,
        loss_report: &report,
        round_trip: Some(reading.round_trip_equal),
        artifact_bytes: bytes.len() as u64,
        health: None,
    })
}

// ---------------------------------------------------------------------------
// 1. The real drop, end to end.

#[tokio::test]
async fn a_dropped_real_model_is_reported_in_plain_language() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let bytes = real_model();

    let (content_type, body) = multipart_drop("model.xmi", &bytes);
    let response = router
        .clone()
        .oneshot(post_multipart("/onboard", &content_type, body))
        .await
        .unwrap();
    // Blocking losses: the import is REFUSED until they are accepted by name, and the refusal
    // is a result page, not an error page.
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let html = body_text(response).await;

    // The project was created from the file name: no project-name box was filled in.
    assert!(
        html.contains("/ui/projects/model/health"),
        "the result must be scoped to the project named after the file"
    );

    let sentences = section_paragraphs(&html, "class=\"onboard-summary\"");
    assert!(
        !sentences.is_empty(),
        "the result page must carry the plain-language summary"
    );

    // Proof 1, verbatim: the plain-language summary the person is shown.
    println!("=== THE PLAIN-LANGUAGE SUMMARY THE USER IS SHOWN ===");
    for sentence in &sentences {
        println!("{}", sentence);
    }
    println!("=== ONE BUTTON ===");
    // The primary button is the one inside the onboarding accept form; the stylesheet also
    // mentions the name, so the form's own class is the marker to search from.
    let button = html
        .split("class=\"onboard-accept\"")
        .nth(1)
        .and_then(|rest| rest.split("accept_all\" value=\"1\">").nth(1))
        .and_then(|rest| rest.split('<').next())
        .unwrap_or("");
    println!("{}", decode_entities(button));

    assert!(
        sentences[0].contains("a SysML v1 XMI export"),
        "{:?}",
        sentences
    );
    assert!(sentences[0].contains("sysml-v1-xmi@2.4"), "{:?}", sentences);
}

// ---------------------------------------------------------------------------
// 2. Every number comes from the engine.

#[tokio::test]
async fn every_number_in_the_summary_matches_the_engine() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let bytes = real_model();
    let reading = read_with_the_engine(&bytes);
    let facts = expected_facts(&bytes, &reading);

    let (content_type, body) = multipart_drop("model.xmi", &bytes);
    let response = router
        .clone()
        .oneshot(post_multipart("/onboard", &content_type, body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let html = body_text(response).await;
    let sentences = section_paragraphs(&html, "class=\"onboard-summary\"");

    // (a) EXACT: the page says precisely what the engine's own numbers compose, sentence for
    // sentence. Nothing is added, nothing is reworded, nothing is dropped.
    let expected = dropzone::summary_sentences(&facts);
    assert_eq!(
        sentences, expected,
        "the rendered summary must equal the template over the engine's own numbers"
    );

    // (b) The numbers are the engine's, named one by one.
    let counts = format!(
        "It carried {} blocks, {} relationships and {} requirements.",
        reading.root.structure.len(),
        reading.root.graph.as_ref().unwrap().edges.len(),
        reading.root.requirements.len(),
    );
    assert!(
        sentences.contains(&counts),
        "expected {:?} among {:?}",
        counts,
        sentences
    );
    let losses = format!(
        "{} things are outside what this reader carries: {} it cannot map at all and {} it carries with a named drop.",
        reading.blocking_total, reading.unmappable, reading.lossy,
    );
    assert!(
        sentences.contains(&losses),
        "expected {:?} among {:?}",
        losses,
        sentences
    );
    let binding =
        binding_registry::resolve(binding_xmi::BINDING_ID, binding_xmi::BINDING_VERSION).unwrap();
    let (_, report) = binding.import(&bytes).unwrap();
    for class in dropzone::loss_classes(&report, 5) {
        let named = format!("{} × {}", class.count, class.name);
        assert!(
            sentences.iter().any(|s| s.contains(&named)),
            "the summary must name the class {} among {:?}",
            named,
            sentences
        );
    }

    // (c) EXHAUSTIVE: every run of digits in the prosaic summary is either one of the engine's
    // own counters or a fragment of a string the engine supplied (the opaque XMI ids inside the
    // example subjects). A count this module invented cannot survive this check.
    let mut allowed: Vec<u64> = vec![
        facts.blocks,
        facts.relationships,
        facts.requirements,
        facts.graph_nodes,
        facts.blocking,
        facts.unmappable,
        facts.lossy,
        facts.declarations,
    ];
    allowed.extend(facts.classes.iter().map(|class| class.count));
    let engine_subjects: String = report
        .blocking()
        .iter()
        .map(|mapping| mapping.subject.clone())
        .collect::<Vec<_>>()
        .join(" ");
    let prose = sentences.join("\n");
    for run in digit_runs(&prose) {
        let as_number = run.parse::<u64>().unwrap_or(u64::MAX);
        let is_engine_count = allowed.contains(&as_number);
        let is_engine_string = engine_subjects.contains(&run);
        assert!(
            is_engine_count || is_engine_string,
            "the summary states {} which is neither an engine count {:?} nor part of an engine string; the summary may not contain a number the engine did not produce:\n{}",
            run,
            allowed,
            prose
        );
    }

    // (d) The classes shown are the FIVE BIGGEST, checked against a full grouping of the
    // engine's own report: no class with more entries than a shown one may be left out, and the
    // counts shown are the engine's own tallies.
    let all = dropzone::loss_classes(&report, usize::MAX);
    let shown_total: u64 = facts.classes.iter().map(|class| class.count).sum();
    let top_total: u64 = all.iter().take(5).map(|class| class.count).sum();
    assert_eq!(shown_total, top_total);
    assert_eq!(facts.classes.len(), 5.min(all.len()));
    let smallest_shown = facts.classes.last().unwrap().count;
    for class in all.iter().skip(facts.classes.len()) {
        assert!(
            class.count <= smallest_shown,
            "a bigger class ({}) was left out of the summary",
            class.name
        );
    }
    assert!(
        all.iter().map(|class| class.count).sum::<u64>() == facts.blocking,
        "the full grouping must account for every blocking entry"
    );

    // (e) The engine's own table renders beside the prose, so the measurement stays visible.
    assert!(
        html.contains("class=\"onboard-facts\""),
        "the facts table must render"
    );
}

// ---------------------------------------------------------------------------
// 3. The no-reasoner path.

#[tokio::test]
async fn the_summary_carries_the_same_facts_with_no_reasoner_configured() {
    // The public showcase runs in open mode with no assist. This test pins that mode: if the
    // process has a reasoner configured the assertion would be about a different deployment, so
    // say so loudly rather than pretend.
    assert!(
        matches!(
            server::assist::reasoner_status(),
            server::assist::ReasonerStatus::NotConfigured
        ),
        "this test pins the NO-REASONER path; run it with no MW_ASSIST_ prefix or MW_ANTHROPIC_ key set"
    );

    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let bytes = real_model();
    let reading = read_with_the_engine(&bytes);
    let facts = expected_facts(&bytes, &reading);

    let (content_type, body) = multipart_drop("model.xmi", &bytes);
    let response = router
        .oneshot(post_multipart("/onboard", &content_type, body))
        .await
        .unwrap();
    let html = body_text(response).await;
    let sentences = section_paragraphs(&html, "class=\"onboard-summary\"");

    // Identical facts, template-composed: no reasoner was consulted and none was needed.
    assert_eq!(sentences, dropzone::summary_sentences(&facts));
    assert!(html.contains("class=\"onboard-facts\""));
    // Every sentence is the engine speaking; the page is server-rendered end to end.
    assert!(html.contains("Refused until you accept: model.xmi"));
}

// ---------------------------------------------------------------------------
// 4. No JavaScript: an ordinary form post completes the flow.

#[tokio::test]
async fn the_flow_completes_from_a_plain_form_post_with_no_script() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));

    // The front door itself: one form, one file input, one submit. A browser with JavaScript
    // disabled posts exactly this.
    let page = body_text(router.clone().oneshot(get("/ui")).await.unwrap()).await;
    assert!(page.contains("Drop your SysML model here, or choose a file"));
    assert!(page.contains("method=\"post\""));
    assert!(page.contains("enctype=\"multipart/form-data\""));
    assert!(page.contains("type=\"file\""));
    assert!(page.contains("name=\"artifact\""));
    let drop = page.find("Drop your SysML model here").unwrap();
    let list = page.find("class=\"projects\"").unwrap_or(usize::MAX);
    assert!(
        drop < list,
        "the drop zone must be the FIRST element on the projects page"
    );

    // The drop, as the form posts it.
    let bytes = real_model();
    let (content_type, body) = multipart_drop("model.xmi", &bytes);
    let response = router
        .clone()
        .oneshot(post_multipart("/onboard", &content_type, body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let html = body_text(response).await;
    assert!(
        html.contains("id=\"summary\""),
        "the result must be server-rendered"
    );

    // The one button, as the form posts it: no script, no fetch, one urlencoded submit.
    let artifact_hash = hidden_value(&html, "artifactHash").unwrap();
    let message = hidden_value(&html, "message").unwrap();
    let all_losses =
        hidden_value(&html, "all_losses").expect("the one button must carry the losses");
    assert_eq!(
        all_losses.split('\n').count(),
        read_with_the_engine(&bytes).blocking_total
    );
    // Exactly what a browser submits: LF rewritten to CRLF by the urlencoded serializer.
    let browser_losses = browser_all_losses(&all_losses);
    let response = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/model/onboard/accept",
            &[
                ("artifactHash", artifact_hash.as_str()),
                ("branch", "main"),
                ("message", message.as_str()),
                ("all_losses", browser_losses.as_str()),
                ("accept_all", "1"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let committed = body_text(response).await;
    assert!(committed.contains("Onboarded: model.xmi"));
}

// ---------------------------------------------------------------------------
// 5. The health view is one click from the result.

#[tokio::test]
async fn accepting_the_named_losses_imports_the_model_and_the_health_view_is_one_click_away() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let bytes = real_model();
    let reading = read_with_the_engine(&bytes);

    let (content_type, body) = multipart_drop("model.xmi", &bytes);
    let refused = router
        .clone()
        .oneshot(post_multipart("/onboard", &content_type, body))
        .await
        .unwrap();
    let refused_html = body_text(refused).await;

    // The ONE button: its label names the engine's counts.
    let label = format!(
        "Import {} blocks and {} relationships, accepting these {} named losses",
        reading.root.structure.len(),
        reading.root.graph.as_ref().unwrap().edges.len(),
        reading.blocking_total,
    );
    assert!(
        refused_html.contains(&label),
        "the one button must read exactly {:?}",
        label
    );
    // The per-entry list and the existing accept-all control stay available behind it.
    assert!(refused_html.contains("class=\"onboard-losses\""));
    assert!(refused_html.contains("loss-accept"));

    let artifact_hash = hidden_value(&refused_html, "artifactHash").unwrap();
    let message = hidden_value(&refused_html, "message").unwrap();
    let all_losses = hidden_value(&refused_html, "all_losses").unwrap();
    let browser_losses = browser_all_losses(&all_losses);
    let accepted = router
        .clone()
        .oneshot(post_form(
            "/ui/projects/model/onboard/accept",
            &[
                ("artifactHash", artifact_hash.as_str()),
                ("branch", "main"),
                ("message", message.as_str()),
                ("all_losses", browser_losses.as_str()),
                ("accept_all", "1"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::CREATED);
    let html = body_text(accepted).await;

    assert!(html.contains("Onboarded: model.xmi"));
    assert!(
        html.contains("It round-trips exactly"),
        "the engine measured the round trip; the summary must say so"
    );
    // The acceptance page states the same facts as the drop page: the format is re-detected
    // from the retained bytes, so a placeholder never reaches the person.
    assert!(
        html.contains("This is a SysML v1 XMI export"),
        "the accepted page must name the format it detected, not a placeholder"
    );
    // The health view is ONE click away, and it is the model that was just committed.
    let commit = link_value(&html, "/health?commit=").expect("a health link must be rendered");
    let health_href = format!("/ui/projects/model/health?commit={}", commit);
    assert!(html.contains(&health_href));

    let health = router.clone().oneshot(get(&health_href)).await.unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    let health_html = body_text(health).await;
    assert!(
        health_html.contains("Model health"),
        "the linked page must be the health view"
    );
    // And the health view's own numbers are the engine's, over the model that was imported.
    let stats = dropzone::health_facts(&reading.root).unwrap();
    assert!(
        health_html.contains(&stats.nodes.to_string()),
        "the health view must carry the engine's node count {}",
        stats.nodes
    );

    // The committed model is the one the drop produced: the same blocks, in the repository.
    let store = server::store::sqlite::SqliteStore::open(&dir.path().join("mw.db")).unwrap();
    let tip = store.branch_tip("model", "main").unwrap().unwrap();
    let model = server::api::load_model(&store, "model", &tip).unwrap();
    assert_eq!(model.structure.len(), reading.root.structure.len());
    assert_eq!(
        model.graph.as_ref().unwrap().edges.len(),
        reading.root.graph.as_ref().unwrap().edges.len()
    );
}
