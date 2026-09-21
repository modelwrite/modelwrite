// SPDX-License-Identifier: AGPL-3.0-or-later
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use server::auth::AuthConfig;
use server::store::{self, StoreConfig};
use server::trial::{self, TrialService};
use server::{app, AppState};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port: u16 = std::env::var("MW_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let host = std::env::var("MW_BIND").unwrap_or_else(|_| "127.0.0.1".to_string());
    let allow_open = std::env::var("MW_ALLOW_OPEN")
        .map(|v| v.eq_ignore_ascii_case("yes"))
        .unwrap_or(false);
    let registered = std::env::var("MW_REGISTERED")
        .map(|v| v.eq_ignore_ascii_case("yes"))
        .unwrap_or(false);

    if registered {
        return run_registered(host, port, allow_open).await;
    }

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
    // every request is accepted as an anonymous admin.
    let auth = AuthConfig::from_env()?;
    if matches!(&auth, AuthConfig::Open) {
        eprintln!(
            "WARNING: mw-server is running WITHOUT authentication (open mode); \
             every request is accepted as an anonymous admin. Set MW_AUTH_TOKEN to \
             require a shared bearer token, or MW_AUTH_JWKS to the path of a JWKS \
             file to require signed JWTs."
        );
    }

    let ip = server::resolve_bind(&host, matches!(&auth, AuthConfig::Open), allow_open)?;
    let store = store::open_store(&config)?;
    let addr = SocketAddr::new(ip, port);
    let listener = tokio::net::TcpListener::bind(addr).await?;
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

/// Run the REGISTERED tier: identity store, per-trial databases, sessions and the rolling
/// lifecycle. A second instance of this same binary, parameterised by MW_TRIAL_DATA_ROOT.
async fn run_registered(host: String, port: u16, allow_open: bool) -> anyhow::Result<()> {
    let data_root = std::env::var("MW_TRIAL_DATA_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("registered-data"));
    let archive_root = std::env::var("MW_TRIAL_ARCHIVE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| data_root.join("archive"));
    std::fs::create_dir_all(&data_root).map_err(|e| {
        anyhow::anyhow!(
            "could not create the registered data root {}: {}",
            data_root.display(),
            e
        )
    })?;

    let identity = trial::IdentityStore::open(&data_root.join("identity.db"))?;
    let registry = store::TrialRegistry::new(data_root.clone());
    let mailer = trial::mailer_from_env();
    let config = trial::TrialConfig::from_env();
    let tier = TrialService::new(identity, registry, mailer, config, archive_root);

    let ip = server::resolve_bind(&host, false, allow_open)?;
    let addr = SocketAddr::new(ip, port);
    let listener = tokio::net::TcpListener::bind(addr).await?;

    // The nightly reaper: a plain thread, not a request path, that archives trials idle for
    // 21 days. It runs on an interval and never touches the live model fleet.
    let reaper_tier = tier.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(24 * 3600));
        match reaper_tier.reap(store::now_seconds()) {
            Ok(0) => {}
            Ok(n) => eprintln!("reaper: archived {} trial(s)", n),
            Err(e) => eprintln!("reaper error: {}", e),
        }
    });

    // The ApiState placeholder store is never read in registered mode: every store access
    // resolves to the caller's own trial database.
    let placeholder = store::sqlite::SqliteStore::open(std::path::Path::new(":memory:"))?;
    let router = server::registered::app_registered(
        tier,
        Arc::new(placeholder),
        data_root.join("evidence"),
        server::max_body_bytes_from_env(),
    );
    println!("mw-server (registered) listening on http://{}", addr);
    axum::serve(listener, router).await?;
    Ok(())
}
