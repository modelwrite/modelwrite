// SPDX-License-Identifier: AGPL-3.0-or-later
//! The offline path: opens a SQLite store directly and performs the store-level
//! operations, so an air-gapped user can commit, merge, gate and inspect history with no
//! server. Offline is NOT a lesser mode: every write validates the document and enforces
//! element locks exactly as the server does, because an air-gapped site is where a model
//! is edited and committed, and a commit already in the history cannot be un-made.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::{json, Value};

use server::store::{
    now_epoch, now_seconds, sqlite::SqliteStore, AuditEntry, Commit, CommitGuard, GateRun, Store,
};

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
            holder,
        } => {
            require_project(&store, &project)?;
            let bytes = std::fs::read(&file)
                .map_err(|e| format!("cannot read {}: {}", file.display(), e))?;
            let root: okf::types::OkfRoot = serde_json::from_slice(&bytes)
                .map_err(|e| format!("not an OKF document: {}", e))?;
            let report = okf::validate::validate(&root);
            if !report.valid {
                return Err(format!(
                    "the model failed validation: {}",
                    report.errors.join("; ")
                ));
            }

            let now = now_seconds();
            let tip_hash = store.branch_tip(&project, &branch).map_err(map)?;
            let touched = match tip_hash.as_deref() {
                Some(tip) => {
                    let tip_model = load_model(&store, &project, tip)?;
                    server::api::touched_elements(&tip_model, &root)
                }
                None => all_touched(&root),
            };
            let holder_name = holder.as_deref().unwrap_or("");
            let guard = CommitGuard {
                holder: holder_name,
                elements: &touched,
                now,
                expected_tip: tip_hash.as_deref(),
            };
            if let Some(h) = holder.as_deref() {
                let held = store.holders_of(&project, &touched, now).map_err(map)?;
                if let Some(blocked) = held.iter().find(|l| l.holder != h) {
                    return Err(lock_refusal_message(
                        &blocked.element,
                        &blocked.holder,
                        blocked.expires_at,
                    ));
                }
            }
            let okf_hash = store.put_blob(&bytes).map_err(map)?;
            let audit = audit_entry(&project, "commit.create", &branch, &message);
            let commit = commit_refusal_guard(
                &store,
                &project,
                &branch,
                store.commit_model(
                    &project,
                    &branch,
                    &okf_hash,
                    "",
                    &message,
                    Some(guard),
                    Some(&audit),
                ),
            )?;
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
        Command::Merge {
            project,
            branch,
            other,
            message,
            holder,
        } => merge(
            &store,
            &project,
            &branch,
            &other,
            &message,
            holder.as_deref(),
        ),
        Command::Reset {
            project,
            branch,
            to,
            message,
            holder,
        } => {
            require_project(&store, &project)?;
            let tip_hash = store
                .branch_tip(&project, &branch)
                .map_err(map)?
                .ok_or_else(|| format!("branch {} not found", branch))?;
            let target = store
                .commit(&project, &to)
                .map_err(map)?
                .ok_or_else(|| format!("commit {} not found", to))?;
            let tip_model = load_model(&store, &project, &tip_hash)?;
            let target_model = load_model(&store, &project, &to)?;
            let report = okf::validate::validate(&target_model);
            if !report.valid {
                return Err(format!(
                    "the model failed validation: {}",
                    report.errors.join("; ")
                ));
            }
            let touched = server::api::touched_elements(&tip_model, &target_model);
            let holder_name = holder.as_deref().unwrap_or("");
            let guard = CommitGuard {
                holder: holder_name,
                elements: &touched,
                now: now_seconds(),
                expected_tip: Some(&tip_hash),
            };
            let audit = audit_entry(
                &project,
                "branch.reset",
                &branch,
                &format!("reset to {}", to),
            );
            let commit = commit_refusal_guard(
                &store,
                &project,
                &branch,
                store.commit_model(
                    &project,
                    &branch,
                    &target.okf_hash,
                    "",
                    &message,
                    Some(guard),
                    Some(&audit),
                ),
            )?;
            Ok(server::api::commit_json(&commit))
        }
        Command::Gate {
            project,
            reference,
            candidate,
        } => gate(&store, &project, &reference, &candidate),
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

/// Three-way merge of other into branch, mirroring the server's merge endpoint.
fn merge(
    store: &SqliteStore,
    project: &str,
    branch: &str,
    other: &str,
    message: &str,
    holder: Option<&str>,
) -> Result<Value, String> {
    require_project(store, project)?;
    let ours_tip = store
        .branch_tip(project, branch)
        .map_err(map)?
        .ok_or_else(|| format!("branch {} not found", branch))?;
    let theirs_tip = store
        .branch_tip(project, other)
        .map_err(map)?
        .ok_or_else(|| format!("branch {} not found", other))?;

    let base_hash = common_ancestor(store, project, &ours_tip, &theirs_tip)?;
    let base = load_model(store, project, &base_hash)?;
    let ours = load_model(store, project, &ours_tip)?;
    let theirs = load_model(store, project, &theirs_tip)?;

    let outcome = server::merge::merge(&base, &ours, &theirs);
    if !outcome.conflicts.is_empty() {
        store
            .append_audit(&AuditEntry {
                id: 0,
                project: project.to_string(),
                at: now_seconds(),
                actor: "anonymous".to_string(),
                mechanism: "offline".to_string(),
                action: "merge.conflict".to_string(),
                subject: branch.to_string(),
                detail: format!("merge conflict between {} and {}", branch, other),
            })
            .map_err(map)?;
        let subjects: Vec<String> = outcome
            .conflicts
            .iter()
            .map(|c| format!("{} ({})", c.subject, c.kind))
            .collect();
        return Err(format!(
            "merge conflict between {} and {}: {}",
            branch,
            other,
            subjects.join("; ")
        ));
    }

    let merged = outcome
        .merged
        .ok_or_else(|| "the merge reported no conflicts but produced nothing".to_string())?;
    let report = okf::validate::validate(&merged);
    if !report.valid {
        return Err("the merged model failed validation and was not stored".to_string());
    }

    let holder_name = holder.unwrap_or("");
    let touched = server::api::touched_elements(&ours, &merged);
    {
        let held = store
            .holders_of(project, &touched, now_seconds())
            .map_err(map)?;
        if let Some(blocked) = held.iter().find(|l| l.holder != holder_name) {
            return Err(lock_refusal_message(
                &blocked.element,
                &blocked.holder,
                blocked.expires_at,
            ));
        }
    }

    let bytes = serde_json::to_vec(&merged)
        .map_err(|e| format!("the merged model could not be stored: {}", e))?;
    let okf_hash = store.put_blob(&bytes).map_err(map)?;
    let touched = server::api::touched_elements(&ours, &merged);
    let parents = vec![ours_tip.clone(), theirs_tip.clone()];
    let guard = CommitGuard {
        holder: holder_name,
        elements: &touched,
        now: now_seconds(),
        expected_tip: Some(&parents[0]),
    };
    let audit = audit_entry(
        project,
        "merge.clean",
        branch,
        &format!("merged {} into {}", other, branch),
    );
    let commit = commit_refusal_guard(
        store,
        project,
        branch,
        store.commit_merge(
            project,
            branch,
            &parents,
            &okf_hash,
            "",
            message,
            Some(guard),
            Some(&audit),
        ),
    )?;
    Ok(json!({
        "commit": server::api::commit_json(&commit),
        "base": base_hash,
    }))
}

/// Run the gate on two stored commits and record the run, mirroring the server's gate
/// endpoint. The returned evidence is the same verdict and record the server produces.
fn gate(
    store: &SqliteStore,
    project: &str,
    reference: &str,
    candidate: &str,
) -> Result<Value, String> {
    require_project(store, project)?;
    let reference_model = load_model(store, project, reference)?;
    let candidate_model = load_model(store, project, candidate)?;

    let outcome = gate::run(&reference_model, &candidate_model, false);
    let branch = store
        .commit(project, candidate)
        .map_err(map)?
        .map(|c| c.branch)
        .unwrap_or_else(|| "main".to_string());

    let run = GateRun {
        project: project.to_string(),
        branch,
        reference_hash: reference.to_string(),
        candidate_hash: candidate.to_string(),
        passed: outcome.passed,
        evidence: outcome.evidence.to_string(),
        created_at: now_epoch(),
    };
    let verdict = if outcome.passed { "passed" } else { "failed" };
    let audit = AuditEntry {
        id: 0,
        project: project.to_string(),
        at: now_seconds(),
        actor: "anonymous".to_string(),
        mechanism: "offline".to_string(),
        action: "gate.run".to_string(),
        subject: candidate.to_string(),
        detail: format!(
            "{}: candidate {} against reference {}",
            verdict, candidate, reference
        ),
    };
    store.record_gate_run(&run, Some(&audit)).map_err(map)?;

    Ok(outcome.evidence)
}

/// Refuse an operation on a project that does not exist, mirroring the HTTP layer's 404.
fn require_project(store: &SqliteStore, project: &str) -> Result<(), String> {
    if store.project(project).map_err(map)?.is_none() {
        return Err(format!("project {} not found", project));
    }
    Ok(())
}

/// Load the OKF document behind a commit hash.
fn load_model(
    store: &SqliteStore,
    project: &str,
    hash: &str,
) -> Result<okf::types::OkfRoot, String> {
    let commit = store
        .commit(project, hash)
        .map_err(map)?
        .ok_or_else(|| format!("commit {} not found", hash))?;
    let bytes = store
        .blob(&commit.okf_hash)
        .map_err(map)?
        .ok_or_else(|| "the stored model is missing".to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| format!("the stored model could not be read: {}", e))
}

/// Every element an incoming document names, used when a branch has no tip yet.
fn all_touched(root: &okf::types::OkfRoot) -> Vec<String> {
    let mut ids = okf::diff::element_ids(root);
    if let Some(graph) = &root.graph {
        for edge in &graph.edges {
            ids.insert(edge.source.clone());
            ids.insert(edge.target.clone());
        }
    }
    ids.into_iter().collect()
}

/// Every commit reachable from a tip, as a set.
fn ancestry_set(store: &SqliteStore, project: &str, tip: &str) -> Result<HashSet<String>, String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut stack: Vec<String> = vec![tip.to_string()];
    while let Some(hash) = stack.pop() {
        if !seen.insert(hash.clone()) {
            continue;
        }
        if let Some(commit) = store.commit(project, &hash).map_err(map)? {
            for parent in &commit.parents {
                stack.push(parent.clone());
            }
        }
    }
    Ok(seen)
}

/// The LOWEST commit both branches share.
fn common_ancestor(
    store: &SqliteStore,
    project: &str,
    ours: &str,
    theirs: &str,
) -> Result<String, String> {
    let our_side = ancestry_set(store, project, ours)?;
    let their_side = ancestry_set(store, project, theirs)?;
    let mut shared: Vec<String> = our_side.intersection(&their_side).cloned().collect();
    if shared.is_empty() {
        return Err("the branches share no common ancestor".to_string());
    }
    shared.sort();

    let mut ancestries: HashMap<String, HashSet<String>> = HashMap::new();
    for candidate in &shared {
        ancestries.insert(candidate.clone(), ancestry_set(store, project, candidate)?);
    }

    let mut lowest: Vec<String> = Vec::new();
    for candidate in &shared {
        let is_ancestor_of_another = shared.iter().any(|other| {
            other != candidate
                && ancestries
                    .get(other)
                    .map(|side| side.contains(candidate))
                    .unwrap_or(false)
        });
        if !is_ancestor_of_another {
            lowest.push(candidate.clone());
        }
    }

    match lowest.len() {
        1 => Ok(lowest.remove(0)),
        0 => Err("no shared commit survived the merge base search".to_string()),
        _ => Err("the branches have more than one possible merge base, so the merge cannot be computed automatically".to_string()),
    }
}

/// The exact message the server uses when it refuses a write for a live lease held by
/// another holder.
fn lock_refusal_message(element: &str, holder: &str, expires_at: i64) -> String {
    format!(
        "{} is locked by {} until {}; a caller who holds this lease must supply the holder field to proceed",
        element, holder, expires_at
    )
}

/// Call a commit-producing store method and, if it is refused because an element is locked,
/// record commit.refused before surfacing the refusal, exactly as the server does.
fn commit_refusal_guard(
    store: &SqliteStore,
    project: &str,
    branch: &str,
    result: Result<Commit, server::store::StoreError>,
) -> Result<Commit, String> {
    match result {
        Ok(commit) => Ok(commit),
        Err(error) if server::store::is_lock_refusal(&error) => {
            let detail = error.to_string();
            if let Err(recording) = store.append_audit(&AuditEntry {
                id: 0,
                project: project.to_string(),
                at: now_seconds(),
                actor: "anonymous".to_string(),
                mechanism: "offline".to_string(),
                action: "commit.refused".to_string(),
                subject: branch.to_string(),
                detail: detail.clone(),
            }) {
                eprintln!("could not record the refusal: {}", recording);
            }
            Err(map(error))
        }
        Err(error) => Err(map(error)),
    }
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
    match error {
        server::store::StoreError::Locked {
            element,
            holder,
            expires_at,
        } => lock_refusal_message(&element, &holder, expires_at),
        other => other.to_string(),
    }
}
