// SPDX-License-Identifier: AGPL-3.0-or-later
pub mod api;
pub mod error;
pub mod store;

pub use error::ApiError;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn store::Store>,
    pub evidence_dir: std::path::PathBuf,
}

impl AppState {
    fn api(&self) -> api::ApiState {
        api::ApiState {
            store: self.store.clone(),
            evidence_dir: self.evidence_dir.clone(),
        }
    }
}

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
        .route("/projects/:project/branches", post(api::create_branch))
        .with_state(api_state)
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
}

async fn version() -> Json<serde_json::Value> {
    Json(json!({
        "server": env!("CARGO_PKG_VERSION"),
        "service": "mw-server"
    }))
}
