# Slice 2 (tranche 2) - Collaboration: branches, three-way merge, revert

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** make the repository collaborative: list and delete branches, merge two branches with element-level conflict reporting, and revert a branch without rewriting history.

**Architecture:** the merge is a pure function over three OKF documents (base, ours, theirs) in server/src/merge.rs, so it is unit-testable without any HTTP or storage. It reuses the engine's canonical-JSON comparison discipline: two elements are the same if their serialised forms are the same. The server layer turns a conflict-free merge into a multi-parent commit and a conflicting merge into a 409 that names every conflict, so a human or an agent can resolve it deliberately. History is append-only: a revert is a NEW commit whose content matches the target, never a rewrite.

This is tranche 2 of Slice 2. It does NOT include element locks, the audit log, identity and roles, the PostgreSQL backend, the CLI or deployment; those are named at the end and get their own plans.

**Tech Stack:** unchanged - Rust stable (edition 2021, rust-version 1.75), axum 0.7, tokio, rusqlite, serde/serde_json, sha2/hex, tower and tempfile for tests.

**Spec:** docs/superpowers/specs/2026-09-17-modelwrite-platform-design.md (sections 4.2 and 4.3). Predecessor: docs/superpowers/plans/2026-09-17-modelwrite-slice-2-repository-service.md.

## Global Constraints

- Rust stable; edition 2021; rust-version 1.75; no new dependencies unless a task says so explicitly.
- Every source file begins with: // SPDX-License-Identifier: AGPL-3.0-or-later
- Cargo is not on PATH in fresh shells: begin every shell command sequence with $env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path
- Use --no-fail-fast when running a test suite.
- Stage and commit only the paths a task owns, plus Cargo.lock when that task changes it. Never amend or rewrite a commit.
- History is APPEND-ONLY: no operation may delete or rewrite a commit row. Reverting is a new commit. Deleting a branch removes only the branch pointer, never its commits.
- The engine's contracts are fixed: this tranche consumes okf::diff, okf::validate and gate::run; it never changes engine/ or the corpus.
- Every request path returns a structured error and never panics: 400 malformed, 404 missing, 409 conflict OR merge conflict, 422 invalid model, 500 internal with a generic message.
- A merge conflict is NOT an error in the transport sense: it is a 409 whose body lists every conflict with its base, ours and theirs values, because an agent must be able to act on it.
- Names (project, branch) are validated by api::validate_name before use.
- Tests drive the router in process with tower oneshot; no port is bound.

---

### Task 1: Branch listing and deletion

**Files:**
- Modify: server/src/store/mod.rs (add delete_branch)
- Modify: server/src/store/sqlite.rs (implement it)
- Modify: server/src/api.rs (two handlers)
- Modify: server/src/lib.rs (two routes)
- Test: server/tests/api.rs (branch lifecycle)

**Interfaces:**
- Consumes: the existing Store trait, list_branches (already implemented but unrouted), ApiState, validate_name, map_store_error.
- Produces:
  - Store::delete_branch(&self, project: &str, name: &str) -> Result<(), StoreError>, which returns NotFound when the branch does not exist and never touches the commits table.
  - GET /projects/:project/branches -> 200 [{"name", "tip"}], 404 when the project is unknown.
  - DELETE /projects/:project/branches/:name -> 204 on success, 404 when the branch or project is unknown, and it never deletes commits.

- [ ] **Step 1: Add delete_branch to the trait**

In server/src/store/mod.rs, after create_branch:

```rust
    /// Remove a branch pointer. This never deletes commits: the objects a branch pointed
    /// at stay in the store, so a deleted branch can be recreated at the same hash and no
    /// history is ever lost.
    fn delete_branch(&self, project: &str, name: &str) -> Result<(), StoreError>;
```

- [ ] **Step 2: Implement it in the SQLite backend**

In server/src/store/sqlite.rs, after create_branch:

```rust
    fn delete_branch(&self, project: &str, name: &str) -> Result<(), StoreError> {
        let removed = self.with(|c| {
            c.execute(
                "DELETE FROM branches WHERE project = ?1 AND name = ?2",
                params![project, name],
            )
        })?;
        if removed == 0 {
            return Err(StoreError::NotFound(format!("branch {}", name)));
        }
        Ok(())
    }
```

- [ ] **Step 3: Add the handlers**

In server/src/api.rs:

```rust
pub async fn list_branches(
    State(state): State<ApiState>,
    Path(project): Path<String>,
) -> Result<Json<Value>, ApiError> {
    if state.store.project(&project).map_err(map_store_error)?.is_none() {
        return Err(ApiError::not_found(format!("project {}", project)));
    }
    let branches = state.store.list_branches(&project).map_err(map_store_error)?;
    let out: Vec<Value> = branches
        .iter()
        .map(|(name, tip)| json!({ "name": name, "tip": tip }))
        .collect();
    Ok(Json(Value::Array(out)))
}

pub async fn delete_branch(
    State(state): State<ApiState>,
    Path((project, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    validate_name("branch name", &name)?;
    state
        .store
        .delete_branch(&project, &name)
        .map_err(map_store_error)?;
    Ok(StatusCode::NO_CONTENT)
}
```

- [ ] **Step 4: Add the routes**

In server/src/lib.rs, next to the existing branch route:

```rust
        .route(
            "/projects/:project/branches",
            post(api::create_branch).get(api::list_branches),
        )
        .route(
            "/projects/:project/branches/:name",
            axum::routing::delete(api::delete_branch),
        )
```

- [ ] **Step 5: Test the branch lifecycle**

Append to server/tests/api.rs:

```rust
#[tokio::test]
async fn branches_can_be_listed_and_deleted_without_losing_commits() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "m", "okf": tiny_okf() }),
        ))
        .await
        .unwrap();
    let hash = json_body(committed).await["hash"].as_str().unwrap().to_string();
    router
        .clone()
        .oneshot(post(
            "/projects/coffee/branches",
            serde_json::json!({ "name": "review", "from": hash }),
        ))
        .await
        .unwrap();

    let listed = router
        .clone()
        .oneshot(get("/projects/coffee/branches"))
        .await
        .unwrap();
    let branches = json_body(listed).await;
    let names: Vec<String> = branches
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"main".to_string()));
    assert!(names.contains(&"review".to_string()));

    let deleted = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/projects/coffee/branches/review")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);

    // The commit the deleted branch pointed at must still be readable.
    let still_there = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/projects/coffee/commits/{}", hash))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(still_there.status(), StatusCode::OK);

    let gone = router
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/projects/coffee/branches/review")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
}
```

The test file already has a helper `get(uri)`; if it does not, add one next to `post`.

- [ ] **Step 6: Run, lint and commit**

```powershell
cargo test -p mw-server --no-fail-fast
cargo fmt --all
cargo clippy -p mw-server --all-targets -- -D warnings
git add server
git commit -m "feat: list and delete branches without touching history"
```

Expected: 19 tests pass.

---
### Task 2: The three-way merge engine

**Files:**
- Create: server/src/merge.rs
- Modify: server/src/lib.rs (declare pub mod merge)
- Modify: server/src/store/mod.rs and sqlite.rs (add commit_merge, a multi-parent commit)
- Test: server/tests/merge.rs

**Interfaces:**
- Consumes: okf::types::{OkfRoot, Element, Requirement, GraphNode, GraphEdge, Summary}; serde_json.
- Produces:
  - merge::merge(base: &OkfRoot, ours: &OkfRoot, theirs: &OkfRoot) -> MergeOutcome, where MergeOutcome { merged: Option<OkfRoot>, conflicts: Vec<Conflict> }. merged is Some only when there are no conflicts.
  - merge::Conflict { subject: String, kind: String, base: Option<String>, ours: Option<String>, theirs: Option<String> }, serialised camelCase. kind is one of bothAdded, bothModified, modifiedVersusDeleted.
  - Store::commit_merge(&self, project, branch, parents: &[String], okf_hash, author, message) -> Result<Commit, StoreError>, which writes a commit with EXPLICIT parents (a merge has two) and moves the branch tip, in one transaction.

Merge rules, which the tests pin:

- At the element level (everything keyed by section:id, plus graph nodes keyed graphnode:id and edges keyed by a canonical JSON array of source, target, kind and label): if ours equals theirs, take it; else if ours equals base, take theirs; else if theirs equals base, take ours; else it is a conflict. An element added on both sides with different content is bothAdded; one side modifying while the other deletes is modifiedVersusDeleted; both modifying differently is bothModified.
- The state machine and the activity list are merged ALL OR NOTHING under the same rule, because they are ordered structures whose internal identity is not stable across an edit. A change to either on both sides is a conflict; a change on one side is taken. This is a documented tranche-2 limitation, not an accident.
- summary is RECOMPUTED from the merged model rather than merged as data, because it is derived: a merged document whose counts contradict its content would be a lie the validator could only warn about.
- exportedAt and okf come from ours; project must be identical in all three documents or the merge conflicts on doc:project.
- Ordering: sections keep our order, then append items that only theirs added, in their order. The result is deterministic.

- [ ] **Step 1: Write server/src/merge.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Three-way merge over OKF documents, as a pure function.
//!
//! Two elements are the same when their canonical JSON is the same, which is the same
//! discipline the engine's diff uses. Nothing here touches storage or HTTP, so every rule
//! is unit-testable on its own.

use std::collections::BTreeMap;

use okf::types::{
    Activity, Element, GraphEdge, GraphNode, OkfRoot, Requirement, StateMachine, Summary,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub subject: String,
    pub kind: String,
    pub base: Option<String>,
    pub ours: Option<String>,
    pub theirs: Option<String>,
}

#[derive(Debug)]
pub struct MergeOutcome {
    pub merged: Option<OkfRoot>,
    pub conflicts: Vec<Conflict>,
}

fn json<T: Serialize>(value: &T) -> Option<String> {
    serde_json::to_string(value).ok()
}

/// Resolve one key by the three-way rule. Returns Ok(Some(value)) to take a value,
/// Ok(None) when the key is absent in the result, or Err(conflict).
fn resolve(
    subject: &str,
    base: Option<&String>,
    ours: Option<&String>,
    theirs: Option<&String>,
) -> Result<Option<String>, Conflict> {
    if ours == theirs {
        return Ok(ours.cloned());
    }
    if ours == base {
        return Ok(theirs.cloned());
    }
    if theirs == base {
        return Ok(ours.cloned());
    }
    let kind = match (base, ours, theirs) {
        (None, Some(_), Some(_)) => "bothAdded",
        (Some(_), Some(_), None) | (Some(_), None, Some(_)) => "modifiedVersusDeleted",
        _ => "bothModified",
    };
    Err(Conflict {
        subject: subject.to_string(),
        kind: kind.to_string(),
        base: base.cloned(),
        ours: ours.cloned(),
        theirs: theirs.cloned(),
    })
}

/// Merge a Vec of items keyed by some id, keeping our order and appending theirs.
fn merge_keyed<T, K, F>(
    section: &str,
    base: &[T],
    ours: &[T],
    theirs: &[T],
    key: F,
    conflicts: &mut Vec<Conflict>,
) -> Vec<T>
where
    T: Clone + Serialize + serde::de::DeserializeOwned,
    K: Ord + Clone,
    F: Fn(&T) -> K,
{
    let to_map = |items: &[T]| -> BTreeMap<K, (String, T)> {
        items
            .iter()
            .map(|item| (key(item), (json(item).unwrap_or_default(), item.clone())))
            .collect()
    };
    let base_map = to_map(base);
    let ours_map = to_map(ours);
    let theirs_map = to_map(theirs);

    let mut keys: Vec<K> = ours_map.keys().cloned().collect();
    for k in theirs_map.keys() {
        if !keys.contains(k) {
            keys.push(k.clone());
        }
    }

    let mut out: Vec<T> = Vec::new();
    for k in keys {
        let subject = format!("{}:{:?}", section, k);
        let b = base_map.get(&k).map(|(j, _)| j);
        let o = ours_map.get(&k).map(|(j, _)| j);
        let t = theirs_map.get(&k).map(|(j, _)| j);
        match resolve(&subject, b, o, t) {
            Ok(Some(chosen)) => {
                if let Ok(item) = serde_json::from_str::<T>(&chosen) {
                    out.push(item);
                }
            }
            Ok(None) => {}
            Err(conflict) => conflicts.push(conflict),
        }
    }
    out
}

/// Merge a document-level value that is treated as one unit.
fn merge_unit<T>(
    subject: &str,
    base: Option<&T>,
    ours: Option<&T>,
    theirs: Option<&T>,
    conflicts: &mut Vec<Conflict>,
) -> Option<T>
where
    T: Clone + Serialize + serde::de::DeserializeOwned,
{
    let b = base.and_then(json);
    let o = ours.and_then(json);
    let t = theirs.and_then(json);
    match resolve(subject, b.as_ref(), o.as_ref(), t.as_ref()) {
        Ok(Some(chosen)) => serde_json::from_str(&chosen).ok(),
        Ok(None) => None,
        Err(conflict) => {
            conflicts.push(conflict);
            None
        }
    }
}

fn edge_key(edge: &GraphEdge) -> String {
    let parts = [
        edge.source.as_str(),
        edge.target.as_str(),
        edge.kind.as_str(),
        edge.label.as_str(),
    ];
    serde_json::to_string(&parts).unwrap_or_default()
}

/// Recompute the derived counts so the merged document cannot contradict itself.
fn recompute_summary(merged: &mut OkfRoot) {
    merged.summary = Summary {
        blocks: merged.structure.len() as u64,
        requirements: merged.requirements.len() as u64,
        interfaces: merged.interfaces.len() as u64,
        signals: merged.signals.len() as u64,
        activities: merged.activities.len() as u64,
        graph_nodes: merged.graph.as_ref().map(|g| g.nodes.len()).unwrap_or(0) as u64,
        graph_edges: merged.graph.as_ref().map(|g| g.edges.len()).unwrap_or(0) as u64,
    };
}

pub fn merge(base: &OkfRoot, ours: &OkfRoot, theirs: &OkfRoot) -> MergeOutcome {
    let mut conflicts: Vec<Conflict> = Vec::new();

    if base.project != ours.project || base.project != theirs.project {
        conflicts.push(Conflict {
            subject: "doc:project".to_string(),
            kind: "bothModified".to_string(),
            base: Some(base.project.clone()),
            ours: Some(ours.project.clone()),
            theirs: Some(theirs.project.clone()),
        });
    }

    let structure = merge_keyed(
        "structure",
        &base.structure,
        &ours.structure,
        &theirs.structure,
        |e: &Element| e.id.clone(),
        &mut conflicts,
    );
    let interfaces = merge_keyed(
        "interfaces",
        &base.interfaces,
        &ours.interfaces,
        &theirs.interfaces,
        |e: &Element| e.id.clone(),
        &mut conflicts,
    );
    let signals = merge_keyed(
        "signals",
        &base.signals,
        &ours.signals,
        &theirs.signals,
        |e: &Element| e.id.clone(),
        &mut conflicts,
    );
    let requirements = merge_keyed(
        "requirements",
        &base.requirements,
        &ours.requirements,
        &theirs.requirements,
        |r: &Requirement| r.id.clone(),
        &mut conflicts,
    );

    // Ordered structures merge all or nothing: see the rule list above.
    let state_machine: Option<StateMachine> = merge_unit(
        "doc:stateMachine",
        base.state_machine.as_ref(),
        ours.state_machine.as_ref(),
        theirs.state_machine.as_ref(),
        &mut conflicts,
    );
    let activities: Vec<Activity> = merge_unit(
        "doc:activities",
        Some(&base.activities),
        Some(&ours.activities),
        Some(&theirs.activities),
        &mut conflicts,
    )
    .unwrap_or_else(|| ours.activities.clone());

    let base_nodes = base.graph.as_ref().map(|g| g.nodes.clone()).unwrap_or_default();
    let our_nodes = ours.graph.as_ref().map(|g| g.nodes.clone()).unwrap_or_default();
    let their_nodes = theirs
        .graph
        .as_ref()
        .map(|g| g.nodes.clone())
        .unwrap_or_default();
    let nodes = merge_keyed(
        "graphnode",
        &base_nodes,
        &our_nodes,
        &their_nodes,
        |n: &GraphNode| n.id.clone(),
        &mut conflicts,
    );

    let base_edges = base.graph.as_ref().map(|g| g.edges.clone()).unwrap_or_default();
    let our_edges = ours.graph.as_ref().map(|g| g.edges.clone()).unwrap_or_default();
    let their_edges = theirs
        .graph
        .as_ref()
        .map(|g| g.edges.clone())
        .unwrap_or_default();
    let edges = merge_keyed(
        "edge",
        &base_edges,
        &our_edges,
        &their_edges,
        edge_key,
        &mut conflicts,
    );

    if !conflicts.is_empty() {
        return MergeOutcome {
            merged: None,
            conflicts,
        };
    }

    let mut merged = ours.clone();
    merged.structure = structure;
    merged.interfaces = interfaces;
    merged.signals = signals;
    merged.requirements = requirements;
    merged.state_machine = state_machine;
    merged.activities = activities;
    merged.graph = Some(okf::types::Graph { nodes, edges });
    recompute_summary(&mut merged);

    MergeOutcome {
        merged: Some(merged),
        conflicts,
    }
}
```

- [ ] **Step 2: Declare the module and add commit_merge**

In server/src/lib.rs add `pub mod merge;`.

In server/src/store/mod.rs, after commit_model:

```rust
    /// Write a commit with EXPLICIT parents and move the branch tip, in one transaction.
    /// A merge commit has two parents, so the parent list cannot be derived from the tip.
    fn commit_merge(
        &self,
        project: &str,
        branch: &str,
        parents: &[String],
        okf_hash: &str,
        author: &str,
        message: &str,
    ) -> Result<Commit, StoreError>;
```

In server/src/store/sqlite.rs, implement it by the same pattern as commit_model, except the
parents come from the argument instead of the branch tip, and every parent must already
exist (return NotFound otherwise so a merge cannot invent ancestry).

- [ ] **Step 3: Write the merge tests**

Create server/tests/merge.rs. Build the documents with serde_json::json! and parse them
into OkfRoot, so every fixture is unambiguous and no test depends on string escaping:

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use okf::types::OkfRoot;
use server::merge::merge;
use serde_json::json;

fn okf(value: serde_json::Value) -> OkfRoot {
    serde_json::from_value(value).expect("test document parses")
}

fn requirement(id: &str, req_id: &str) -> serde_json::Value {
    json!({
        "id": id,
        "name": id,
        "kind": "requirement",
        "stereotypes": ["Requirement"],
        "attributes": [],
        "documentation": "",
        "reqId": req_id,
        "reqText": "text"
    })
}

/// One block, one requirement and the satisfy edge between them, optionally plus a second
/// requirement and its edge, so a test can change exactly one thing at a time.
fn model(block_name: &str, with_second_requirement: bool) -> serde_json::Value {
    let mut requirements = vec![requirement("r1", "1.1")];
    let mut nodes = vec![
        json!({"id": "b1", "kind": "block", "name": "Block"}),
        json!({"id": "r1", "kind": "requirement", "name": "r1"}),
    ];
    let mut edges = vec![json!({
        "source": "b1",
        "target": "r1",
        "kind": "dependency",
        "label": "Satisfy"
    })];
    if with_second_requirement {
        requirements.push(requirement("r2", "1.2"));
        nodes.push(json!({"id": "r2", "kind": "requirement", "name": "r2"}));
        edges.push(json!({
            "source": "b1",
            "target": "r2",
            "kind": "dependency",
            "label": "Satisfy"
        }));
    }
    json!({
        "project": "coffee",
        "exportedAt": "2026-09-17T00:00:00Z",
        "summary": {},
        "stateMachine": {"name": "sm", "regions": []},
        "requirements": requirements,
        "structure": [{
            "id": "b1",
            "name": block_name,
            "kind": "block",
            "stereotypes": ["Block"],
            "attributes": [],
            "documentation": ""
        }],
        "graph": {"nodes": nodes, "edges": edges}
    })
}

#[test]
fn a_change_on_one_side_is_taken() {
    let base = okf(model("Block", false));
    let ours = okf(model("Block", false));
    let theirs = okf(model("Renamed", false));
    let outcome = merge(&base, &ours, &theirs);
    assert!(outcome.conflicts.is_empty(), "{:?}", outcome.conflicts);
    assert_eq!(outcome.merged.unwrap().structure[0].name, "Renamed");
}

#[test]
fn identical_changes_merge_without_conflict() {
    let base = okf(model("Block", false));
    let ours = okf(model("Renamed", false));
    let theirs = okf(model("Renamed", false));
    let outcome = merge(&base, &ours, &theirs);
    assert!(outcome.conflicts.is_empty(), "{:?}", outcome.conflicts);
    assert_eq!(outcome.merged.unwrap().structure[0].name, "Renamed");
}

#[test]
fn two_different_changes_to_one_element_conflict() {
    let base = okf(model("Block", false));
    let ours = okf(model("Ours", false));
    let theirs = okf(model("Theirs", false));
    let outcome = merge(&base, &ours, &theirs);
    assert!(outcome.merged.is_none(), "a conflict must not produce a document");
    assert_eq!(outcome.conflicts.len(), 1);
    assert_eq!(outcome.conflicts[0].kind, "bothModified");
    assert!(outcome.conflicts[0].subject.contains("b1"));
    assert!(outcome.conflicts[0].ours.is_some());
    assert!(outcome.conflicts[0].theirs.is_some());
}

#[test]
fn an_element_added_by_one_side_is_kept_and_the_summary_is_recomputed() {
    let base = okf(model("Block", false));
    let ours = okf(model("Block", false));
    let theirs = okf(model("Block", true));
    let outcome = merge(&base, &ours, &theirs);
    assert!(outcome.conflicts.is_empty(), "{:?}", outcome.conflicts);
    let merged = outcome.merged.expect("clean merge");
    assert_eq!(merged.requirements.len(), 2);
    assert_eq!(merged.summary.requirements, 2, "the summary is recomputed");
}

#[test]
fn an_edge_added_by_one_side_is_kept() {
    let base = okf(model("Block", false));
    let mut our_value = model("Block", false);
    our_value["graph"]["edges"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "source": "b1",
            "target": "r1",
            "kind": "dependency",
            "label": "Verify"
        }));
    let ours = okf(our_value);
    let theirs = okf(model("Block", false));
    let outcome = merge(&base, &ours, &theirs);
    assert!(outcome.conflicts.is_empty(), "{:?}", outcome.conflicts);
    let merged = outcome.merged.expect("clean merge");
    assert_eq!(merged.graph.as_ref().unwrap().edges.len(), 2);
}

#[test]
fn deleting_on_one_side_while_the_other_modifies_is_a_conflict() {
    let base = okf(model("Block", true));
    let ours = okf(model("Block", false)); // r2 deleted here
    let mut their_value = model("Block", true);
    their_value["requirements"][1]["reqText"] = json!("changed by them");
    let theirs = okf(their_value);
    let outcome = merge(&base, &ours, &theirs);
    assert!(outcome.merged.is_none());
    assert_eq!(outcome.conflicts.len(), 1);
    assert_eq!(outcome.conflicts[0].kind, "modifiedVersusDeleted");
}
```
- [ ] **Step 4: Run, lint and commit**

```powershell
cargo test -p mw-server --no-fail-fast
cargo fmt --all
cargo clippy -p mw-server --all-targets -- -D warnings
git add server
git commit -m "feat: add the three-way merge engine and multi-parent commits"
```

Expected: 25 tests pass (19 plus the six merge tests).

---
### Task 3: The merge endpoint

**Files:**
- Create: server/src/merge_api.rs
- Modify: server/src/lib.rs (declare the module, add the route)
- Test: server/tests/merge_api.rs

**Interfaces:**
- Consumes: store::{Store, commit_merge, commit, branch_tip}; merge::{merge, MergeOutcome}; the gate_api pattern for loading a model behind a commit.
- Produces: POST /projects/:project/merge {"branch", "other", "author", "message"} ->
  - 201 {"commit": {...}, "base": ancestor-hash} when the merge is clean, with a commit whose parents are [branch tip, other tip];
  - 409 {"conflicts": [...], "base": ancestor-hash} when the documents conflict, and NO commit is written;
  - 404 when either branch is unknown.

- [ ] **Step 1: Write server/src/merge_api.rs**

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::{commit_json, load_model, map_store_error, validate_name, ApiState};
use crate::error::ApiError;
use crate::merge::merge;

#[derive(Deserialize)]
pub struct MergeRequest {
    pub branch: String,
    pub other: String,
    pub author: String,
    pub message: String,
}

/// Every commit reachable from a tip, tip first. The walk stops at a commit already seen,
/// so a malformed cycle in stored data cannot loop forever.
fn ancestry(state: &ApiState, project: &str, tip: &str) -> Result<Vec<String>, ApiError> {
    let mut seen: Vec<String> = Vec::new();
    let mut stack: Vec<String> = vec![tip.to_string()];
    while let Some(hash) = stack.pop() {
        if seen.contains(&hash) {
            continue;
        }
        if let Some(commit) = state.store.commit(project, &hash).map_err(map_store_error)? {
            for parent in &commit.parents {
                stack.push(parent.clone());
            }
        }
        seen.push(hash);
    }
    Ok(seen)
}

/// The nearest commit both branches share, found by walking our ancestry and taking the
/// first hash that theirs also reaches. With a linear history that is exactly the fork
/// point; with merge commits it is an approximation, which is documented rather than
/// hidden: a general lowest-common-ancestor is a later refinement.
fn common_ancestor(state: &ApiState, project: &str, ours: &str, theirs: &str) -> Result<String, ApiError> {
    let our_side = ancestry(state, project, ours)?;
    let their_side = ancestry(state, project, theirs)?;
    our_side
        .iter()
        .find(|hash| their_side.contains(hash))
        .cloned()
        .ok_or_else(|| ApiError::conflict("the branches share no common ancestor"))
}

pub async fn merge_branches(
    State(state): State<ApiState>,
    Path(project): Path<String>,
    Json(body): Json<MergeRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    validate_name("branch name", &body.branch)?;
    validate_name("branch name", &body.other)?;

    let ours_tip = state
        .store
        .branch_tip(&project, &body.branch)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("branch {}", body.branch)))?;
    let theirs_tip = state
        .store
        .branch_tip(&project, &body.other)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("branch {}", body.other)))?;

    let base_hash = common_ancestor(&state, &project, &ours_tip, &theirs_tip)?;
    let base = load_model(&state, &project, &base_hash)?;
    let ours = load_model(&state, &project, &ours_tip)?;
    let theirs = load_model(&state, &project, &theirs_tip)?;

    let outcome = merge(&base, &ours, &theirs);
    if !outcome.conflicts.is_empty() {
        // A conflict is not a transport failure: nothing is written, and the caller gets
        // every conflicting subject with its base, ours and theirs values so a human or an
        // agent can resolve it deliberately rather than guess.
        return Ok((
            StatusCode::CONFLICT,
            Json(json!({
                "base": base_hash,
                "branch": body.branch,
                "other": body.other,
                "conflicts": outcome.conflicts
            })),
        ));
    }

    let merged = outcome
        .merged
        .ok_or_else(|| ApiError::internal("the merge reported no conflicts but produced nothing"))?;
    let bytes = serde_json::to_vec(&merged).map_err(|e| {
        eprintln!("merged model could not be serialised: {}", e);
        ApiError::internal("the merged model could not be stored")
    })?;
    let okf_hash = state.store.put_blob(&bytes).map_err(map_store_error)?;
    let commit = state
        .store
        .commit_merge(
            &project,
            &body.branch,
            &[ours_tip, theirs_tip],
            &okf_hash,
            &body.author,
            &body.message,
        )
        .map_err(map_store_error)?;

    Ok((
        StatusCode::CREATED,
        Json(json!({ "commit": commit_json(&commit), "base": base_hash })),
    ))
}
```

This requires three items to be public in server/src/api.rs: ApiState (already public),
map_store_error (already public), validate_name (already public), and commit_json and
load_model, which the implementer must make public (they are currently private helpers of
api.rs and gate_api.rs respectively). Move load_model into api.rs if that is simpler, and
delete the copy in gate_api.rs so there is exactly one loader.

- [ ] **Step 2: Add the route**

In server/src/lib.rs add `pub mod merge_api;` and:

```rust
        .route("/projects/:project/merge", post(merge_api::merge_branches))
```

- [ ] **Step 3: Write server/tests/merge_api.rs**

Three tests, each starting from a fresh temporary store:

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
//! The merge endpoint end to end: a clean merge writes a two-parent commit, a conflict
//! writes nothing and names the conflicts, and the branch tips are left as they were.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

fn state(dir: &std::path::Path) -> server::AppState {
    let store = server::store::sqlite::SqliteStore::open(&dir.join("mw.db")).unwrap();
    server::AppState {
        store: std::sync::Arc::new(store),
        evidence_dir: dir.to_path_buf(),
    }
}

fn post(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn model(block_name: &str) -> Value {
    json!({
        "project": "coffee",
        "exportedAt": "2026-09-17T00:00:00Z",
        "summary": {},
        "stateMachine": {"name": "sm", "regions": []},
        "requirements": [{
            "id": "r1", "name": "r1", "kind": "requirement",
            "stereotypes": ["Requirement"], "attributes": [], "documentation": "",
            "reqId": "1.1", "reqText": "text"
        }],
        "structure": [{
            "id": "b1", "name": block_name, "kind": "block",
            "stereotypes": ["Block"], "attributes": [], "documentation": ""
        }],
        "graph": {
            "nodes": [
                {"id": "b1", "kind": "block", "name": "Block"},
                {"id": "r1", "kind": "requirement", "name": "r1"}
            ],
            "edges": [{"source": "b1", "target": "r1", "kind": "dependency", "label": "Satisfy"}]
        }
    })
}

async fn seed(router: &axum::Router) -> String {
    router
        .clone()
        .oneshot(post("/projects", json!({ "name": "coffee" })))
        .await
        .unwrap();
    let committed = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            json!({ "branch": "main", "author": "alex", "message": "base", "okf": model("Block") }),
        ))
        .await
        .unwrap();
    let hash = json_body(committed).await["hash"].as_str().unwrap().to_string();
    router
        .clone()
        .oneshot(post(
            "/projects/coffee/branches",
            json!({ "name": "feature", "from": hash }),
        ))
        .await
        .unwrap();
    hash
}

#[tokio::test]
async fn a_clean_merge_writes_a_two_parent_commit() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    let _base = seed(&router).await;

    // The feature branch renames the block; main is untouched.
    router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            json!({ "branch": "feature", "author": "alex", "message": "rename", "okf": model("Renamed") }),
        ))
        .await
        .unwrap();

    let merged = router
        .clone()
        .oneshot(post(
            "/projects/coffee/merge",
            json!({ "branch": "main", "other": "feature", "author": "alex", "message": "merge feature" }),
        ))
        .await
        .unwrap();
    assert_eq!(merged.status(), StatusCode::CREATED);
    let body = json_body(merged).await;
    assert_eq!(body["commit"]["parents"].as_array().unwrap().len(), 2);

    // The merged model is what the branch now serves, and it carries the rename.
    let tip = body["commit"]["hash"].as_str().unwrap().to_string();
    let fetched = router
        .oneshot(get(&format!("/projects/coffee/commits/{}", tip)))
        .await
        .unwrap();
    let served = json_body(fetched).await;
    assert_eq!(served["structure"][0]["name"], "Renamed");
}

#[tokio::test]
async fn a_conflicting_merge_writes_nothing_and_names_every_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    seed(&router).await;

    router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            json!({ "branch": "feature", "author": "alex", "message": "theirs", "okf": model("Theirs") }),
        ))
        .await
        .unwrap();
    router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            json!({ "branch": "main", "author": "alex", "message": "ours", "okf": model("Ours") }),
        ))
        .await
        .unwrap();

    let conflicted = router
        .clone()
        .oneshot(post(
            "/projects/coffee/merge",
            json!({ "branch": "main", "other": "feature", "author": "alex", "message": "merge" }),
        ))
        .await
        .unwrap();
    assert_eq!(conflicted.status(), StatusCode::CONFLICT);
    let body = json_body(conflicted).await;
    let conflicts = body["conflicts"].as_array().unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0]["kind"], "bothModified");

    // Nothing was written: main still points at our commit, not at a merge.
    let history = router
        .oneshot(get("/projects/coffee/commits?branch=main"))
        .await
        .unwrap();
    let history = json_body(history).await;
    let commits = history.as_array().unwrap();
    assert_eq!(commits.len(), 2, "the base commit and ours, and no merge commit");
    for commit in commits {
        assert_eq!(commit["parents"].as_array().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn merging_an_unknown_branch_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    seed(&router).await;
    let response = router
        .oneshot(post(
            "/projects/coffee/merge",
            json!({ "branch": "main", "other": "nope", "author": "alex", "message": "merge" }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
```

- [ ] **Step 4: Run, lint and commit**

```powershell
cargo test -p mw-server --no-fail-fast
cargo fmt --all
cargo clippy -p mw-server --all-targets -- -D warnings
git add server
git commit -m "feat: merge two branches with element-level conflict reporting"
```

Expected: 28 tests pass.

---

### Task 4: Revert without rewriting history

**Files:**
- Modify: server/src/api.rs (one handler)
- Modify: server/src/lib.rs (one route)
- Test: server/tests/api.rs (one test)

**Interfaces:**
- Consumes: store::{commit, commit_model, blob, branch_tip}; validate_name; commit_json.
- Produces: POST /projects/:project/branches/:name/reset {"to": commit-hash, "author", "message"} -> 201 {"commit": {...}} where the new commit's CONTENT equals the target commit's content and its parent is the branch's previous tip. 404 when the branch or the target commit is unknown.

Why a new commit rather than a move: a branch pointer that moves backwards makes every
hash that was built on the old tip unreachable from the branch while still stored, so a
colleague who already pulled it holds a history the repository no longer explains. A revert
commit keeps every commit reachable and records who restored what.

- [ ] **Step 1: Add the handler to server/src/api.rs**

```rust
#[derive(Deserialize)]
pub struct ResetBranch {
    pub to: String,
    pub author: String,
    pub message: String,
}

/// Restore a branch to the CONTENT of an earlier commit by appending a new commit. The
/// history is never rewritten: the old tip stays reachable, and the revert is itself a
/// commit with an author and a message.
pub async fn reset_branch(
    State(state): State<ApiState>,
    Path((project, name)): Path<(String, String)>,
    Json(body): Json<ResetBranch>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    validate_name("branch name", &name)?;
    if state.store.branch_tip(&project, &name).map_err(map_store_error)?.is_none() {
        return Err(ApiError::not_found(format!("branch {}", name)));
    }
    let target = state
        .store
        .commit(&project, &body.to)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::not_found(format!("commit {}", body.to)))?;
    // The target's model is already stored, so this reuses its blob rather than copying
    // the bytes: the restored content is byte-identical to the original by construction.
    let commit = state
        .store
        .commit_model(&project, &name, &target.okf_hash, &body.author, &body.message)
        .map_err(map_store_error)?;
    Ok((StatusCode::CREATED, Json(commit_json(&commit))))
}
```

- [ ] **Step 2: Add the route**

```rust
        .route(
            "/projects/:project/branches/:name/reset",
            post(api::reset_branch),
        )
```

- [ ] **Step 3: Test it**

Append to server/tests/api.rs:

```rust
#[tokio::test]
async fn a_reset_appends_a_commit_and_keeps_the_old_tip_reachable() {
    let dir = tempfile::tempdir().unwrap();
    let router = server::app(state(dir.path()));
    router
        .clone()
        .oneshot(post("/projects", serde_json::json!({ "name": "coffee" })))
        .await
        .unwrap();
    let first = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "one", "okf": tiny_okf() }),
        ))
        .await
        .unwrap();
    let first_hash = json_body(first).await["hash"].as_str().unwrap().to_string();

    let mut second_model = tiny_okf();
    second_model["project"] = serde_json::json!("changed");
    let second = router
        .clone()
        .oneshot(post(
            "/projects/coffee/commits",
            serde_json::json!({ "branch": "main", "author": "alex", "message": "two", "okf": second_model }),
        ))
        .await
        .unwrap();
    let second_hash = json_body(second).await["hash"].as_str().unwrap().to_string();

    let reset = router
        .clone()
        .oneshot(post(
            "/projects/coffee/branches/main/reset",
            serde_json::json!({ "to": first_hash, "author": "alex", "message": "revert to one" }),
        ))
        .await
        .unwrap();
    assert_eq!(reset.status(), StatusCode::CREATED);
    let revert = json_body(reset).await;
    assert_eq!(revert["parents"], serde_json::json!([second_hash]));

    // The reverted content matches the target, and the superseded commit is still there.
    let tip = revert["hash"].as_str().unwrap().to_string();
    let fetched = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/projects/coffee/commits/{}", tip))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(json_body(fetched).await, tiny_okf());

    let superseded = router
        .oneshot(
            Request::builder()
                .uri(format!("/projects/coffee/commits/{}", second_hash))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(superseded.status(), StatusCode::OK);
}
```

- [ ] **Step 4: Run, lint and commit**

```powershell
cargo test --workspace --no-fail-fast
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add server
git commit -m "feat: revert a branch by appending a commit"
```

Expected: 29 service tests pass, and the whole workspace with them.

---

## Completion criteria

- [ ] cargo test --workspace --no-fail-fast passes, including the 11 new service tests.
- [ ] cargo fmt --all -- --check and cargo clippy --workspace --all-targets -- -D warnings are clean.
- [ ] A clean merge writes a commit with two parents whose content is the merged document, and the branch serves it.
- [ ] A conflicting merge returns 409 with every conflict naming its base, ours and theirs values, and writes NOTHING.
- [ ] A reset appends a commit whose content matches the target and whose parent is the previous tip; the superseded commit remains readable.
- [ ] Deleting a branch removes the pointer and leaves every commit readable.
- [ ] No request path panics; every failure is a structured status code.

## What the next tranche must add (not in this plan)

- Element-level locks with leases and expiry, and optimistic check-and-set on commit.
- The append-only audit log for commits, branch operations, locks and gate runs.
- Identity (OIDC and LDAP), roles, per-project scoping and tokens for CLI and CI use.
- The PostgreSQL backend behind the same Store trait. NOTE: this machine has no Docker and no PostgreSQL, so that tranche must be verified in CI with a Postgres service container and record honestly that it was not exercised locally.
- The modelwrite CLI, and the docker compose and Helm deployment skeleton.
- A proper lowest-common-ancestor for merge bases once merge commits exist, replacing the first-match walk.

