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
    // A fresh install mounts /data but not /data/evidence, and the first gate run would 500
    // if the directory were missing. Create it up front so the failure (if any) surfaces at
    // startup, on a path a deploy can see, rather than as a 500 after a run was recorded.
    std::fs::create_dir_all(&evidence_dir).map_err(|e| {
        anyhow::anyhow!(
            "could not create the evidence directory {}: {}",
            evidence_dir.display(),
            e
        )
    })?;

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

    // The bind address is configurable, but the DEFAULT stays loopback: a service that
    // listens on every interface the moment it starts is a service that is exposed by
    // accident. A container must set MW_BIND=0.0.0.0 to be reachable, which is a deliberate
    // act in a deployment file rather than a silent default.
    let host = std::env::var("MW_BIND").unwrap_or_else(|_| "127.0.0.1".to_string());
    let allow_open = std::env::var("MW_ALLOW_OPEN")
        .map(|v| v.eq_ignore_ascii_case("yes"))
        .unwrap_or(false);
    let ip = server::resolve_bind(&host, matches!(&auth, AuthConfig::Open), allow_open)?;

    let store = store::open_store(&config)?;
    let addr = SocketAddr::new(ip, port);
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
