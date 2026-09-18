// SPDX-License-Identifier: AGPL-3.0-or-later
pub mod api;
pub mod audit_api;
pub mod auth;
pub mod error;
pub mod gate_api;
pub mod locks_api;
pub mod merge;
pub mod merge_api;
pub mod store;

pub use error::ApiError;

use std::sync::Arc;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn store::Store>,
    pub evidence_dir: std::path::PathBuf,
    pub auth: auth::AuthConfig,
}

impl AppState {
    fn api(&self) -> api::ApiState {
        api::ApiState {
            store: self.store.clone(),
            evidence_dir: self.evidence_dir.clone(),
            auth: self.auth.clone(),
        }
    }
}

/// The complete route table. `/health` and `/version` are public liveness/information
/// endpoints and take no identity. Every other route takes `Identity` as its first
/// extractor and enforces the required permission — and project reachability, where a
/// project is named — before touching the store, so an unauthenticated caller gets 401 and
/// an under-privileged one gets 403 rather than reaching project data.
pub fn app(state: AppState) -> Router {
    let api_state = state.api();
    Router::new()
        .route("/health", get(health))
        .route("/version", get(version))
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
        .route("/projects/:project/merge", post(merge_api::merge_branches))
        .route(
            "/projects/:project/gate-runs",
            get(gate_api::list_gate_runs),
        )
        .route("/projects/:project/audit", get(audit_api::list_audit))
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
        .with_state(api_state)
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
