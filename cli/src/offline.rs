// SPDX-License-Identifier: AGPL-3.0-or-later
//! The offline path: opens a SQLite store directly and performs the store-level
//! operations, so an air-gapped user can commit and inspect history with no server.

use std::path::Path;

use serde_json::{json, Value};

use server::store::{now_seconds, sqlite::SqliteStore, AuditEntry, Store};

use crate::Command;

/// Run a command against the SQLite store at db, with no server and no network.
pub fn run(db: &Path, command: Command) -> Result<Value, String> {
    let store =
        SqliteStore::open(db).map_err(|e| format!("cannot open {}: {}", db.display(), e))?;
    match command {
        Command::ProjectCreate { name } => {
            let audit = audit_entry(&name, "project.create", &name, "project created");
            let project = store.create_project(&name, Some(&audit)).map_err(map)?;
            Ok(json!({ "name": project.name, "createdAt": project.created_at }))
        }
        Command::ProjectList => {
            let projects = store.list_projects().map_err(map)?;
            Ok(Value::Array(
                projects
                    .iter()
                    .map(|p| json!({ "name": p.name, "createdAt": p.created_at }))
                    .collect(),
            ))
        }
        Command::Commit {
            project,
            branch,
            message,
            file,
            holder: _,
        } => {
            require_project(&store, &project)?;
            let bytes = std::fs::read(&file)
                .map_err(|e| format!("cannot read {}: {}", file.display(), e))?;
            serde_json::from_slice::<Value>(&bytes)
                .map_err(|e| format!("{} is not valid JSON: {}", file.display(), e))?;
            let okf_hash = store.put_blob(&bytes).map_err(map)?;
            let audit = audit_entry(&project, "commit.create", &branch, &message);
            let commit = store
                .commit_model(
                    &project,
                    &branch,
                    &okf_hash,
                    "",
                    &message,
                    None,
                    Some(&audit),
                )
                .map_err(map)?;
            Ok(server::api::commit_json(&commit))
        }
        Command::Log { project, branch } => {
            let commits = store.commits_on(&project, &branch).map_err(map)?;
            Ok(Value::Array(
                commits.iter().map(server::api::commit_json).collect(),
            ))
        }
        Command::BranchList { project } => {
            require_project(&store, &project)?;
            let branches = store.list_branches(&project).map_err(map)?;
            Ok(Value::Array(
                branches
                    .iter()
                    .map(|(name, tip)| json!({ "name": name, "tip": tip }))
                    .collect(),
            ))
        }
        Command::BranchCreate {
            project,
            name,
            from,
        } => {
            let audit = audit_entry(&project, "branch.create", &name, &format!("from {}", from));
            store
                .create_branch(&project, &name, &from, Some(&audit))
                .map_err(map)?;
            Ok(json!({ "name": name, "tip": from }))
        }
        Command::BranchDelete { project, name } => {
            let audit = audit_entry(&project, "branch.delete", &name, "branch deleted");
            store
                .delete_branch(&project, &name, Some(&audit))
                .map_err(map)?;
            Ok(Value::Null)
        }
        Command::Merge { .. } => Err(
            "merge needs the OKF engine and is available only against a server; run with --server"
                .to_string(),
        ),
        Command::Reset {
            project,
            branch,
            to,
            message,
            holder: _,
        } => {
            require_project(&store, &project)?;
            store
                .branch_tip(&project, &branch)
                .map_err(map)?
                .ok_or_else(|| format!("branch {} not found", branch))?;
            let target = store
                .commit(&project, &to)
                .map_err(map)?
                .ok_or_else(|| format!("commit {} not found", to))?;
            let audit = audit_entry(
                &project,
                "branch.reset",
                &branch,
                &format!("reset to {}", to),
            );
            let commit = store
                .commit_model(
                    &project,
                    &branch,
                    &target.okf_hash,
                    "",
                    &message,
                    None,
                    Some(&audit),
                )
                .map_err(map)?;
            Ok(server::api::commit_json(&commit))
        }
        Command::Gate { .. } => Err(
            "gate needs the OKF engine and is available only against a server; run with --server"
                .to_string(),
        ),
        Command::LockAcquire {
            project,
            branch,
            elements,
            holder,
            ttl_seconds,
        } => {
            let now = now_seconds();
            let detail = format!(
                "{} element(s) by {} until {}",
                elements.len(),
                holder,
                now + ttl_seconds
            );
            let audit = audit_entry(&project, "lock.acquire", &elements.join(","), &detail);
            let locks = store
                .acquire_locks(
                    &project,
                    &branch,
                    &elements,
                    &holder,
                    ttl_seconds,
                    now,
                    Some(&audit),
                )
                .map_err(map)?;
            serde_json::to_value(&locks).map_err(|e| e.to_string())
        }
        Command::LockRelease {
            project,
            holder,
            ids,
        } => {
            let audit = audit_entry(&project, "lock.release", &ids.join(","), "released lock(s)");
            let released = store
                .release_locks(&project, &holder, &ids, Some(&audit))
                .map_err(map)?;
            Ok(json!({ "released": released }))
        }
        Command::LockList { project } => {
            require_project(&store, &project)?;
            let locks = store.locks(&project, now_seconds()).map_err(map)?;
            serde_json::to_value(&locks).map_err(|e| e.to_string())
        }
        Command::Audit { project, limit } => {
            require_project(&store, &project)?;
            let entries = store.audit(&project, limit).map_err(map)?;
            serde_json::to_value(&entries).map_err(|e| e.to_string())
        }
    }
}

/// Refuse an operation on a project that does not exist, mirroring the HTTP layer's 404.
fn require_project(store: &SqliteStore, project: &str) -> Result<(), String> {
    if store.project(project).map_err(map)?.is_none() {
        return Err(format!("project {} not found", project));
    }
    Ok(())
}

/// An audit entry for an offline mutation. Nobody was authenticated (there is no server),
/// so the actor is anonymous and the mechanism records how the write really happened.
fn audit_entry(project: &str, action: &str, subject: &str, detail: &str) -> AuditEntry {
    AuditEntry {
        id: 0,
        project: project.to_string(),
        at: now_seconds(),
        actor: "anonymous".to_string(),
        mechanism: "offline".to_string(),
        action: action.to_string(),
        subject: subject.to_string(),
        detail: detail.to_string(),
    }
}

fn map(error: server::store::StoreError) -> String {
    error.to_string()
}
