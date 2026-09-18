// SPDX-License-Identifier: AGPL-3.0-or-later
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use server::auth::AuthConfig;
use server::store::sqlite::SqliteStore;
use server::{app, AppState};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port: u16 = std::env::var("MW_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let db_path = std::env::var("MW_DB").unwrap_or_else(|_| "modelwrite.db".to_string());
    let evidence_dir = std::env::var("MW_EVIDENCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("docs/evidence"));

    // Authentication is opt-in. With no token configured the service runs in OPEN mode:
    // every request is accepted as an anonymous admin. That keeps pilots and air-gapped
    // installs working, so the warning is deliberate and loud, not an error.
    let auth = AuthConfig::from_env()?;
    if matches!(&auth, AuthConfig::Open) {
        eprintln!(
            "WARNING: mw-server is running WITHOUT authentication (open mode); \
             every request is accepted as an anonymous admin. Set MW_AUTH_TOKEN to \
             require a bearer token."
        );
    }

    let store = Arc::new(SqliteStore::open(std::path::Path::new(&db_path))?);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("mw-server listening on http://{} (db {})", addr, db_path);
    axum::serve(
        listener,
        app(AppState {
            store,
            evidence_dir,
            auth,
        }),
    )
    .await?;
    Ok(())
}
