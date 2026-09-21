// SPDX-License-Identifier: AGPL-3.0-or-later
pub mod analytics_api;
pub mod api;
pub mod assist;
pub mod audit;
pub mod audit_api;
pub mod auth;
pub mod binding_api;
pub mod binding_registry;
pub mod composition;
pub mod error;
pub mod gate_api;
pub mod locks_api;
pub mod merge;
pub mod merge_api;
pub mod proposal_api;
pub mod registered;
pub mod store;
pub mod trial;
pub mod ui;

pub use error::{ApiError, BodyTooLarge};

use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

/// The largest request body the service will accept, unless `MW_MAX_BODY_BYTES` says
/// otherwise.
///
/// 512 MiB, deliberately: real MBSE models run to hundreds of megabytes (the Open-MBEE
/// Thirty Meter Telescope is a 36 MB XMI and is SMALL by production standards), so a
/// two-megabyte default turns a routine vendor export into a failure. 512 MiB accepts the
/// models a migration actually sees while staying under SQLite's 1 GB single-value ceiling
/// with headroom, and it bounds a single upload's on-disk staging so a run of concurrent
/// imports cannot exhaust the host. A larger model is a configuration decision, not a code
/// change: set `MW_MAX_BODY_BYTES`.
pub const DEFAULT_MAX_BODY_BYTES: u64 = 512 * 1024 * 1024;

/// Read `MW_MAX_BODY_BYTES` from the environment, falling back to
/// [`DEFAULT_MAX_BODY_BYTES`] when it is unset or does not parse to a positive number. A
/// value that is not a positive integer is a configuration error, but refusing to start on
/// it would make a typo in one optional knob take the service down; the honest fallback is
/// the documented default, and the operator can see the effective value on the import page.
pub fn max_body_bytes_from_env() -> u64 {
    std::env::var("MW_MAX_BODY_BYTES")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|bytes| *bytes > 0)
        .unwrap_or(DEFAULT_MAX_BODY_BYTES)
}

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn store::Store>,
    pub evidence_dir: std::path::PathBuf,
    pub auth: auth::AuthConfig,
}

/// The complete route table. `/health` and `/version` are public liveness/information
/// endpoints and take no identity. Every other route enforces the required permission —
/// and project reachability, where a project is named — before touching the store, so an
/// unauthenticated caller gets 401 and an under-privileged one gets 403 rather than
/// reaching project data.
///
/// The JSON routes take `Identity` as an extractor. The `/ui` routes call the same
/// `auth::identity` function directly, because they render their failures as pages rather
/// than as JSON, and then apply the SAME Read and scope decisions in the same order. Two
/// ways to RESOLVE an identity is a deliberate trade for rendering the 401 as HTML; two
/// ways to DECIDE what it may do would not be, which is why the decisions are shared.
pub fn app(state: AppState) -> Router {
    app_with_limit(state, max_body_bytes_from_env())
}

/// The complete route table at an explicit body limit. Tests drive the refusal through
/// this entry point with a small limit; `app` uses the configured (or default) limit. The
/// limit is applied as [`DefaultBodyLimit`] so EVERY body-consuming route - the JSON
/// extractors and the streaming multipart path alike - is capped, and it is carried on
/// [`api::ApiState`] so the import page can state it and the streaming handler can refuse
/// with the exact number.
pub fn app_with_limit(state: AppState, max_body_bytes: u64) -> Router {
    let api_state = api::ApiState {
        store: state.store,
        evidence_dir: state.evidence_dir,
        auth: state.auth,
        max_body_bytes,
        registered: None,
    };
    workbench_router(api_state, max_body_bytes)
        // A person given the trial's address types the BARE hostname, not /ui. Serving 404 there
        // made the deployment look broken when it was working. The root sends a visitor to the
        // workbench; every real route is unchanged.
        .route(
            "/",
            get(|| async { axum::response::Redirect::temporary("/ui") }),
        )
}

/// The workbench + JSON API route table, shared by the showcase and the registered tier. The
/// registered tier builds the same table with a different [api::ApiState] (session auth plus a
/// per-trial store resolver), so every route keeps its permission and scope checks unchanged.
pub(crate) fn workbench_router(api_state: api::ApiState, max_body_bytes: u64) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/version", get(version))
        .route(
            "/ui",
            get(ui::pages::project_list).post(ui::pages::create_project),
        )
        .route("/ui/projects/:project", get(ui::pages::project_page))
        .route(
            "/ui/projects/:project/changes",
            get(ui::pages::project_page),
        )
        .route(
            "/ui/projects/:project/branch",
            post(ui::pages::create_branch),
        )
        .route("/ui/projects/:project/model", get(ui::model::model_page))
        .route(
            "/ui/projects/:project/overview",
            get(ui::model::overview_page),
        )
        .route("/ui/projects/:project/health", get(ui::health::health_page))
        .route("/ui/projects/:project/stpa", get(ui::stpa::stpa_page))
        .route("/ui/projects/:project/search", get(ui::search::search_page))
        .route(
            "/ui/projects/:project/structure",
            get(ui::model::structure_page),
        )
        .route(
            "/ui/projects/:project/composition",
            get(ui::composition::composition_page),
        )
        .route(
            "/ui/projects/:project/requirements",
            get(ui::model::requirements_page),
        )
        .route(
            "/ui/projects/:project/traceability",
            get(ui::model::traceability_page),
        )
        .route(
            "/ui/projects/:project/assist",
            get(ui::assist::assist_page).post(ui::assist::assist_form),
        )
        .route(
            "/ui/projects/:project/proposals",
            get(ui::proposals::proposals_page),
        )
        .route(
            "/ui/projects/:project/proposals/:id/accept",
            post(ui::assist::accept_proposal_form),
        )
        .route("/ui/app.js", get(ui::app_js))
        .route(
            "/ui/projects/:project/model/new",
            post(ui::create::create_model),
        )
        .route(
            "/ui/projects/:project/diagram",
            get(ui::diagram::diagram_page),
        )
        .route(
            "/ui/projects/:project/element/new",
            get(ui::create::element_form).post(ui::create::create_element),
        )
        .route(
            "/ui/projects/:project/edit/:element",
            get(ui::edit::edit_form).post(ui::edit::submit_edit),
        )
        .route(
            "/ui/projects/:project/import",
            get(ui::import::import_page).post(ui::import::submit_import),
        )
        .route(
            "/ui/projects/:project/import/upload",
            post(ui::import::submit_import_upload),
        )
        .route(
            "/ui/projects/:project/import/accept",
            post(ui::import::accept_import_form),
        )
        .route("/ui/projects/:project/gate", get(ui::gate::gate_list))
        .route("/ui/projects/:project/checks", get(ui::gate::gate_list))
        .route(
            "/ui/projects/:project/gate/:reference/:candidate",
            get(ui::gate::gate_detail),
        )
        .route(
            "/ui/projects/:project/checks/:reference/:candidate",
            get(ui::gate::gate_detail),
        )
        .route(
            "/ui/projects/:project/compare",
            get(ui::review::compare_page),
        )
        .route("/ui/projects/:project/merge", post(ui::review::merge_form))
        .route(
            "/ui/projects/:project/version/new",
            get(ui::version::new_version_page),
        )
        .route(
            "/ui/projects/:project/version/make-current",
            get(ui::version::make_current_page),
        )
        .route(
            "/projects",
            post(api::create_project).get(api::list_projects),
        )
        .route(
            "/projects/:project/commits",
            post(api::create_commit).get(api::list_commits),
        )
        .route("/projects/:project/commits/:hash", get(api::get_commit))
        .route(
            "/projects/:project/commits/:hash/record",
            get(api::get_commit_record),
        )
        .route(
            "/projects/:project/commits/:hash/references",
            get(api::list_references),
        )
        .route(
            "/projects/:project/commits/:hash/references/resolve",
            get(api::resolve_references),
        )
        .route(
            "/projects/:project/commits/:hash/checks",
            get(gate_api::commit_checks),
        )
        .route(
            "/projects/:project/branches",
            post(api::create_branch).get(api::list_branches),
        )
        .route(
            "/projects/:project/branches/:name",
            axum::routing::delete(api::delete_branch),
        )
        .route(
            "/projects/:project/branches/:name/reset",
            post(api::reset_branch),
        )
        .route("/projects/:project/gate", post(gate_api::run_gate))
        .route(
            "/projects/:project/import",
            post(binding_api::import_artifact),
        )
        .route(
            "/projects/:project/import/stream",
            post(binding_api::import_artifact_stream),
        )
        .route(
            "/projects/:project/import/:artifactHash/accept",
            post(binding_api::accept_import_artifact),
        )
        .route(
            "/projects/:project/import/:artifactHash/report",
            get(binding_api::import_report),
        )
        .route(
            "/projects/:project/import/:artifactHash/artifact",
            get(binding_api::get_artifact),
        )
        .route("/projects/:project/assist", post(assist::assist))
        .route(
            "/projects/:project/proposals",
            post(proposal_api::record_proposal).get(proposal_api::list_proposals),
        )
        .route(
            "/projects/:project/proposals/:id",
            get(proposal_api::get_proposal),
        )
        .route(
            "/projects/:project/proposals/:id/accept",
            post(proposal_api::accept_proposal),
        )
        .route(
            "/projects/:project/proposals/:id/refuse",
            post(proposal_api::refuse_proposal),
        )
        .route("/projects/:project/merge", post(merge_api::merge_branches))
        .route(
            "/projects/:project/gate-runs",
            get(gate_api::list_gate_runs),
        )
        .route("/projects/:project/audit", get(audit_api::list_audit))
        .route(
            "/projects/:project/analytics",
            get(analytics_api::project_analytics),
        )
        .route(
            "/projects/:project/locks",
            post(locks_api::acquire_locks)
                .get(locks_api::list_locks)
                .delete(locks_api::release_locks),
        )
        .route(
            "/projects/:project/locks/release",
            post(locks_api::release_locks),
        )
        .layer(DefaultBodyLimit::max(max_body_bytes as usize))
        .with_state(api_state)
}

/// Resolve the address the service binds to, and refuse the combination that leaks a
/// system of record: listening beyond loopback while authentication is not configured, so
/// every request would be an anonymous admin.
///
/// The default stays loopback on purpose. A container must say MW_BIND=0.0.0.0 to be
/// reachable, which makes exposure a deliberate line in a deployment file rather than
/// something that happens the first time the service starts.
pub fn resolve_bind(
    host: &str,
    auth_is_open: bool,
    allow_open: bool,
) -> anyhow::Result<std::net::IpAddr> {
    let ip: std::net::IpAddr = host
        .parse()
        .map_err(|_| anyhow::anyhow!("MW_BIND must be an IP address, got {}", host))?;
    if auth_is_open && !ip.is_loopback() && !allow_open {
        anyhow::bail!(
            "refusing to bind {} while authentication is not configured: every request \
             would be an anonymous admin. Configure MW_AUTH_TOKEN or MW_AUTH_JWKS, bind \
             loopback, or set MW_ALLOW_OPEN=yes to accept the risk deliberately.",
            ip
        );
    }
    Ok(ip)
}

/// Liveness. Public on purpose: it never reaches the store, and it needs no token - a
/// health check that requires a credential fails exactly during the incident it exists to
/// detect. It reports the AUTH MODE so a deployment that forgot to configure authentication
/// is visible to monitoring rather than only in a stderr line; it reports the mode ONLY,
/// never a token, a key or a path.
async fn health(State(state): State<api::ApiState>) -> Json<serde_json::Value> {
    Json(json!({ "status": "ok", "authMode": state.auth.mechanism() }))
}

/// Build information. Public and stateless for the same reason as health.
async fn version() -> Json<serde_json::Value> {
    Json(json!({
        "server": env!("CARGO_PKG_VERSION"),
        "service": "mw-server"
    }))
}
