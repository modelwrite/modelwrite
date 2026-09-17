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
    K: Ord + Clone + std::fmt::Debug + std::hash::Hash + Eq,
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

    // Our order first, then what only theirs added, in their order. Iterating the maps
    // would sort by key, and a merge that silently reshuffles a document makes every
    // merge look like a rewrite. A key present only in base was deleted by both sides, so
    // it never appears here - which is exactly what should happen to it.
    let mut keys: Vec<K> = Vec::new();
    let mut known: std::collections::HashSet<K> = std::collections::HashSet::new();
    for item in ours {
        let k = key(item);
        if known.insert(k.clone()) {
            keys.push(k);
        }
    }
    for item in theirs {
        let k = key(item);
        if known.insert(k.clone()) {
            keys.push(k);
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

/// One relationship as a unit: a source, a target, a kind, and the set of labels carried
/// between them.
///
/// Keying an edge by ALL of its fields, label included, makes key-equality the same thing
/// as content-equality - so the three-way rule's conflict branches can never fire for an
/// edge, and an edit on one side looks like a delete plus an add. Concretely: if ours
/// removes a Satisfy link while theirs relabels it Verify, both sides drop the old key and
/// theirs adds a new one, so ours' deletion is silently discarded. Grouping by identity
/// (source, target, kind) and comparing the label SET as content is what makes that case a
/// modifiedVersusDeleted conflict, as it should be. The label set also means two edges
/// sharing a triple are never collapsed into one.
#[derive(Clone, Serialize, serde::Deserialize)]
struct EdgeUnit {
    source: String,
    target: String,
    kind: String,
    labels: Vec<String>,
}

fn edge_units(edges: &[GraphEdge]) -> Vec<EdgeUnit> {
    let mut grouped: BTreeMap<(String, String, String), std::collections::BTreeSet<String>> =
        BTreeMap::new();
    for edge in edges {
        grouped
            .entry((edge.source.clone(), edge.target.clone(), edge.kind.clone()))
            .or_default()
            .insert(edge.label.clone());
    }
    grouped
        .into_iter()
        .map(|((source, target, kind), labels)| EdgeUnit {
            source,
            target,
            kind,
            labels: labels.into_iter().collect(),
        })
        .collect()
}

fn edges_from_units(units: &[EdgeUnit]) -> Vec<GraphEdge> {
    let mut edges = Vec::new();
    for unit in units {
        for label in &unit.labels {
            edges.push(GraphEdge {
                source: unit.source.clone(),
                target: unit.target.clone(),
                kind: unit.kind.clone(),
                label: label.clone(),
            });
        }
    }
    edges
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
    // merge_unit returns None only when it pushed a conflict, and a conflict aborts the
    // merge before this value is used; the fallback exists so the type checks out, not to
    // silently prefer ours.
    .unwrap_or_else(|| ours.activities.clone());

    let base_nodes = base
        .graph
        .as_ref()
        .map(|g| g.nodes.clone())
        .unwrap_or_default();
    let our_nodes = ours
        .graph
        .as_ref()
        .map(|g| g.nodes.clone())
        .unwrap_or_default();
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

    let base_edges = base
        .graph
        .as_ref()
        .map(|g| g.edges.clone())
        .unwrap_or_default();
    let our_edges = ours
        .graph
        .as_ref()
        .map(|g| g.edges.clone())
        .unwrap_or_default();
    let their_edges = theirs
        .graph
        .as_ref()
        .map(|g| g.edges.clone())
        .unwrap_or_default();
    let edges = edges_from_units(&merge_keyed(
        "edge",
        &edge_units(&base_edges),
        &edge_units(&our_edges),
        &edge_units(&their_edges),
        |unit: &EdgeUnit| format!("{}|{}|{}", unit.source, unit.target, unit.kind),
        &mut conflicts,
    ));

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
    // A graph section is only emitted if some input had one: inventing an empty graph
    // would turn a document that never had one into one that appears to.
    merged.graph = if base.graph.is_some() || ours.graph.is_some() || theirs.graph.is_some() {
        Some(okf::types::Graph { nodes, edges })
    } else {
        None
    };
    recompute_summary(&mut merged);

    MergeOutcome {
        merged: Some(merged),
        conflicts,
    }
}
