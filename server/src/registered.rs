// SPDX-License-Identifier: AGPL-3.0-or-later
//! The registered-trial router: the no-JS registration flow and the per-request lifecycle
//! gate, plus the same workbench route table as the showcase, backed by a per-trial store
//! resolver. The trial identity arrives from the SESSION, never from the request.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{middleware, Router};

use crate::api::ApiState;
use crate::auth::{self, AuthConfig};
use crate::store::{self, Store};
use crate::trial::TrialService;
use crate::ui::register;

/// Build the registered-tier router. `showcase_store` is an unused placeholder in registered
/// mode: every store access resolves to the caller's own trial database through the resolver.
pub fn app_registered(
    tier: Arc<TrialService>,
    showcase_store: Arc<dyn Store>,
    evidence_dir: std::path::PathBuf,
    max_body_bytes: u64,
) -> Router {
    let api_state = ApiState {
        store: showcase_store,
        evidence_dir,
        auth: AuthConfig::Session {
            sessions: tier.clone(),
        },
        max_body_bytes,
        registered: Some(tier.clone()),
    };
    let workbench = crate::workbench_router(api_state, max_body_bytes);

    let auth_routes = Router::new()
        .route("/", get(register::welcome))
        .route(
            "/register",
            get(register::register_form).post(register::submit_register),
        )
        .route(
            "/login",
            get(register::login_form).post(register::submit_login),
        )
        .route("/logout", post(register::logout))
        .route(
            "/account",
            get(register::account_page).post(register::update_consent),
        )
        .with_state(tier.clone());

    auth_routes
        .merge(workbench)
        .layer(middleware::from_fn_with_state(tier, lifecycle))
}

/// A write is any method that mutates; reads and navigations never refuse on lifecycle
/// grounds.
fn is_write(method: &Method) -> bool {
    matches!(
        method,
        &Method::POST | &Method::PUT | &Method::PATCH | &Method::DELETE
    )
}

/// The per-request lifecycle gate: resolve the session, refuse a write to a read-only trial
/// with the restart message, and refresh the rolling window on every authenticated request
/// (read, write or import). Reads still work while read-only and, as activity, restart the
/// window.
async fn lifecycle(
    State(tier): State<Arc<TrialService>>,
    request: Request,
    next: middleware::Next,
) -> Response {
    let method = request.method().clone();
    if let Some(token) = auth::session_token(request.headers()) {
        if let Some(session) = tier.resolve_session(token) {
            let now = store::now_seconds();
            if is_write(&method) {
                if let Some(message) = tier.write_refusal_for(&session.trial_id, now) {
                    return (StatusCode::FORBIDDEN, message).into_response();
                }
            }
            let _ = tier.record_activity(&session.trial_id, now);
        }
    }
    next.run(request).await
}
