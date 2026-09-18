// SPDX-License-Identifier: AGPL-3.0-or-later
use std::net::SocketAddr;
use std::path::PathBuf;

use server::auth::AuthConfig;
use server::store::{self, StoreConfig};
use server::{app, AppState};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port: u16 = std::env::var("MW_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    // MW_DATABASE_URL selects the PostgreSQL backend; MW_DB (default modelwrite.db) is a
    // path to a SQLite file. Both open behind the same Store trait, so nothing else changes.
    let config = StoreConfig::from_env()?;
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
             require a shared bearer token, or MW_AUTH_JWKS to the path of a JWKS \
             file to require signed JWTs."
        );
    }

    let store = store::open_store(&config)?;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    // Never print a connection URL: it can carry a password. Report the backend and, for
    // SQLite, the path.
    let db_desc = match &config {
        StoreConfig::Sqlite(path) => path.display().to_string(),
        StoreConfig::Postgres(_) => "postgres".to_string(),
    };
    println!("mw-server listening on http://{} (db {})", addr, db_desc);
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
