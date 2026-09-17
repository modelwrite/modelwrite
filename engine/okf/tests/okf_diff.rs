// SPDX-License-Identifier: AGPL-3.0-or-later
use okf::{diff, types::OkfRoot};

fn expected() -> OkfRoot {
    serde_json::from_str(&test_support::load_okf_expected()).expect("corpus fixture parses")
}

#[test]
fn identical_models_diff_equal() {
    let r = expected();
    assert!(diff::diff(&r, &r).equal);
}

#[test]
fn removed_requirement_is_missing_element() {
    let mut candidate = expected();
    candidate.requirements.pop();
    let d = diff::diff(&expected(), &candidate);
    assert!(!d.equal);
    assert_eq!(d.missing_elements.len(), 1);
}

#[test]
fn removed_edge_is_missing_edge() {
    let mut candidate = expected();
    candidate.graph.as_mut().expect("graph present").edges.pop();
    let d = diff::diff(&expected(), &candidate);
    assert!(!d.equal);
    assert_eq!(d.missing_edges.len(), 1);
}

#[test]
fn renamed_element_is_changed_attribute() {
    let mut candidate = expected();
    candidate.structure[0].name.push_str(" X");
    let d = diff::diff(&expected(), &candidate);
    assert!(!d.equal);
    assert_eq!(d.changed_attributes.len(), 1);
}

#[test]
fn activity_change_is_a_difference() {
    let mut candidate = expected();
    candidate.activities[0].name.push_str(" X");
    let d = diff::diff(&expected(), &candidate);
    assert!(!d.equal);
    assert_eq!(d.changed_attributes.len(), 1);
}

#[test]
fn document_level_change_is_a_difference() {
    let mut candidate = expected();
    candidate.project.push_str(" X");
    let d = diff::diff(&expected(), &candidate);
    assert!(!d.equal);
    assert_eq!(d.changed_attributes.len(), 1);
}

#[test]
fn state_machine_change_is_a_difference() {
    let mut candidate = expected();
    candidate
        .state_machine
        .as_mut()
        .expect("state machine present")
        .name
        .push_str(" X");
    let d = diff::diff(&expected(), &candidate);
    assert!(!d.equal);
    assert!(d.changed_attributes.iter().any(|k| k == "doc:stateMachine"));
}
#[test]
fn report_serializes_with_camel_case_keys() {
    // The report is a published contract (the MCP okf.diff tool and any agent binding
    // to it). Pin the key style so a serde rename cannot silently change the contract.
    let report = diff::diff(&expected(), &expected());
    let value = serde_json::to_value(&report).expect("report serializes");
    let keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(|k| k.as_str())
        .collect();
    assert!(keys.contains(&"missingElements"), "keys: {:?}", keys);
    assert!(keys.contains(&"changedAttributes"), "keys: {:?}", keys);
    assert!(!keys.contains(&"missing_elements"), "keys: {:?}", keys);
}
