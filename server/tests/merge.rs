// SPDX-License-Identifier: AGPL-3.0-or-later
use okf::types::OkfRoot;
use serde_json::json;
use server::merge::merge;

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
    assert!(
        outcome.merged.is_none(),
        "a conflict must not produce a document"
    );
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
#[test]
fn our_section_order_is_preserved() {
    // A merge must not reshuffle a document: if it did, every merge would look like a
    // rewrite and a reviewer could not see what actually changed. The ids here are
    // deliberately NOT in sorted order, so a key-sorted merge fails this test.
    let mut base_value = model("Block", false);
    base_value["structure"] = json!([
        {"id": "b2", "name": "Second", "kind": "block", "stereotypes": ["Block"], "attributes": [], "documentation": ""},
        {"id": "b1", "name": "First", "kind": "block", "stereotypes": ["Block"], "attributes": [], "documentation": ""}
    ]);
    let base = okf(base_value.clone());
    let ours = okf(base_value.clone());

    // They add a third block, which must land AFTER our two, not sorted among them.
    let mut their_value = base_value;
    their_value["structure"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "b0", "name": "Added", "kind": "block",
            "stereotypes": ["Block"], "attributes": [], "documentation": ""
        }));
    let theirs = okf(their_value);

    let outcome = merge(&base, &ours, &theirs);
    assert!(outcome.conflicts.is_empty(), "{:?}", outcome.conflicts);
    let names: Vec<String> = outcome
        .merged
        .expect("clean merge")
        .structure
        .iter()
        .map(|e| e.name.clone())
        .collect();
    assert_eq!(names, vec!["Second", "First", "Added"]);
}

#[test]
fn both_sides_adding_the_same_element_differently_conflicts() {
    let base = okf(model("Block", false));
    let mut our_value = model("Block", false);
    our_value["requirements"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "r9", "name": "Ours", "kind": "requirement",
            "stereotypes": ["Requirement"], "attributes": [], "documentation": "",
            "reqId": "9.1", "reqText": "ours"
        }));
    let mut their_value = model("Block", false);
    their_value["requirements"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "r9", "name": "Theirs", "kind": "requirement",
            "stereotypes": ["Requirement"], "attributes": [], "documentation": "",
            "reqId": "9.2", "reqText": "theirs"
        }));
    let outcome = merge(&base, &okf(our_value), &okf(their_value));
    assert!(outcome.merged.is_none());
    assert_eq!(outcome.conflicts.len(), 1);
    assert_eq!(outcome.conflicts[0].kind, "bothAdded");
}

#[test]
fn a_different_project_name_conflicts() {
    let base = okf(model("Block", false));
    let mut our_value = model("Block", false);
    our_value["project"] = json!("one");
    let mut their_value = model("Block", false);
    their_value["project"] = json!("two");
    let outcome = merge(&base, &okf(our_value), &okf(their_value));
    assert!(outcome.merged.is_none());
    assert_eq!(outcome.conflicts[0].subject, "doc:project");
}
#[test]
fn deleting_the_same_element_on_both_sides_merges_cleanly() {
    // The both-delete path is what the merged key list relies on: a key present only in
    // base never enters the list, so the element is dropped and no conflict is raised.
    // Deleting the same thing on two branches is agreement, not disagreement.
    let base = okf(model("Block", true));
    let ours = okf(model("Block", false));
    let theirs = okf(model("Block", false));
    let outcome = merge(&base, &ours, &theirs);
    assert!(outcome.conflicts.is_empty(), "{:?}", outcome.conflicts);
    let merged = outcome.merged.expect("a shared deletion merges cleanly");
    assert_eq!(merged.requirements.len(), 1);
    assert!(
        merged.requirements.iter().all(|r| r.id != "r2"),
        "the element both sides deleted must not come back"
    );
}
