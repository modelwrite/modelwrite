// SPDX-License-Identifier: AGPL-3.0-or-later
pub mod error;
pub mod store;

use std::path::PathBuf;
use std::sync::Arc;

use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;

pub use error::ApiError;

#[derive(Clone)]
pub struct AppState {
    pub evidence_dir: PathBuf,
}

/// The whole HTTP surface as a router, so tests drive it in-process and never bind a port.
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/version", get(version))
        .with_state(Arc::new(state))
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
