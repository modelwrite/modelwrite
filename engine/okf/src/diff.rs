// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::types::OkfRoot;

#[derive(Debug, Clone, Serialize)]
pub struct DiffReport {
    pub equal: bool,
    pub missing_elements: Vec<String>,
    pub extra_elements: Vec<String>,
    pub missing_edges: Vec<String>,
    pub extra_edges: Vec<String>,
    pub changed_attributes: Vec<String>,
}

/// Every element id across all sections, plus every graph node id.
pub fn element_ids(root: &OkfRoot) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for el in &root.structure {
        ids.insert(el.id.clone());
    }
    for el in &root.interfaces {
        ids.insert(el.id.clone());
    }
    for el in &root.signals {
        ids.insert(el.id.clone());
    }
    for r in &root.requirements {
        ids.insert(r.id.clone());
    }
    if let Some(sm) = &root.state_machine {
        for region in &sm.regions {
            for s in &region.states {
                ids.insert(s.id.clone());
            }
        }
    }
    if let Some(graph) = &root.graph {
        for n in &graph.nodes {
            ids.insert(n.id.clone());
        }
    }
    ids
}

/// Every relationship as a source|target|kind|label key.
pub fn edge_keys(root: &OkfRoot) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    if let Some(graph) = &root.graph {
        for e in &graph.edges {
            let parts = [
                e.source.as_str(),
                e.target.as_str(),
                e.kind.as_str(),
                e.label.as_str(),
            ];
            if let Ok(key) = serde_json::to_string(&parts) {
                keys.insert(key);
            }
        }
    }
    keys
}

/// Canonical JSON per comparable unit, keyed by "<section>:<id>".
///
/// The universe covers the document-level fields, every element-bearing section, and
/// the activities section (which carries its own nodes and edges). Section scoping
/// matters: the graph mirrors elements, so the same id appears both as a section item
/// and as a graph node, and keying by id alone would let the graph entry mask a removal
/// or a change in the section that owns the element.
pub fn attribute_keys(root: &OkfRoot) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();

    if let Ok(v) = serde_json::to_string(&root.project) {
        map.insert("doc:project".to_string(), v);
    }
    if let Ok(v) = serde_json::to_string(&root.okf) {
        map.insert("doc:okf".to_string(), v);
    }
    if let Ok(v) = serde_json::to_string(&root.exported_at) {
        map.insert("doc:exportedAt".to_string(), v);
    }
    if let Ok(v) = serde_json::to_string(&root.summary) {
        map.insert("doc:summary".to_string(), v);
    }
    if let Ok(v) = serde_json::to_string(&root.provenance) {
        map.insert("doc:provenance".to_string(), v);
    }
    if let Some(sm) = &root.state_machine {
        if let Ok(v) = serde_json::to_string(sm) {
            map.insert("doc:stateMachine".to_string(), v);
        }
    }

    let mut put = |section: &str, id: &str, json: String| {
        map.insert(format!("{}:{}", section, id), json);
    };

    for el in &root.structure {
        if let Ok(v) = serde_json::to_string(el) {
            put("structure", &el.id, v);
        }
    }
    for el in &root.interfaces {
        if let Ok(v) = serde_json::to_string(el) {
            put("interfaces", &el.id, v);
        }
    }
    for el in &root.signals {
        if let Ok(v) = serde_json::to_string(el) {
            put("signals", &el.id, v);
        }
    }
    for r in &root.requirements {
        if let Ok(v) = serde_json::to_string(r) {
            put("requirements", &r.id, v);
        }
    }
    if let Some(sm) = &root.state_machine {
        for region in &sm.regions {
            for s in &region.states {
                if let Ok(v) = serde_json::to_string(s) {
                    put("state", &s.id, v);
                }
            }
        }
    }
    for (i, a) in root.activities.iter().enumerate() {
        if let Ok(v) = serde_json::to_string(a) {
            put("activity", &i.to_string(), v);
        }
    }
    if let Some(graph) = &root.graph {
        for n in &graph.nodes {
            if let Ok(v) = serde_json::to_string(n) {
                put("graphnode", &n.id, v);
            }
        }
    }
    map
}

pub fn diff(reference: &OkfRoot, candidate: &OkfRoot) -> DiffReport {
    let ref_attrs = attribute_keys(reference);
    let cand_attrs = attribute_keys(candidate);
    let ref_edges = edge_keys(reference);
    let cand_edges = edge_keys(candidate);

    let mut missing_elements: Vec<String> = ref_attrs
        .keys()
        .filter(|k| !cand_attrs.contains_key(*k))
        .cloned()
        .collect();
    let mut extra_elements: Vec<String> = cand_attrs
        .keys()
        .filter(|k| !ref_attrs.contains_key(*k))
        .cloned()
        .collect();

    let mut changed_attributes: Vec<String> = Vec::new();
    for (id, ref_value) in &ref_attrs {
        if let Some(cand_value) = cand_attrs.get(id) {
            if cand_value != ref_value {
                changed_attributes.push(id.clone());
            }
        }
    }

    let mut missing_edges: Vec<String> = ref_edges.difference(&cand_edges).cloned().collect();
    let mut extra_edges: Vec<String> = cand_edges.difference(&ref_edges).cloned().collect();

    missing_elements.sort();
    extra_elements.sort();
    missing_edges.sort();
    extra_edges.sort();
    changed_attributes.sort();

    let equal = missing_elements.is_empty()
        && extra_elements.is_empty()
        && missing_edges.is_empty()
        && extra_edges.is_empty()
        && changed_attributes.is_empty();

    DiffReport {
        equal,
        missing_elements,
        extra_elements,
        missing_edges,
        extra_edges,
        changed_attributes,
    }
}
