// SPDX-License-Identifier: AGPL-3.0-or-later
pub mod layout;
pub mod routing;
pub mod stpa;
pub mod symbol;

use std::collections::{HashMap, HashSet};

use okf::types::OkfRoot;
use petgraph::unionfind::UnionFind;
use serde::Serialize;

/// Published as camelCase JSON, like every other contract the platform exposes (the
/// OKF document, the gate evidence, the diff report), so a consumer never has to
/// special-case one payload's key style.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphStats {
    pub node_count: usize,
    pub edge_count: usize,
    pub isolated: Vec<String>,
    pub component_count: usize,
    pub component_sizes: Vec<usize>,
}

pub fn graph_stats(root: &OkfRoot) -> GraphStats {
    let graph = root.graph.as_ref().expect("graph required; validate first");
    let node_count = graph.nodes.len();
    let edge_count = graph.edges.len();
    let index: HashMap<&str, usize> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    let mut uf = UnionFind::new(node_count);
    let mut degree = vec![0usize; node_count];
    for e in &graph.edges {
        if let (Some(&a), Some(&b)) = (index.get(e.source.as_str()), index.get(e.target.as_str())) {
            degree[a] += 1;
            degree[b] += 1;
            uf.union(a, b);
        }
    }
    let isolated: Vec<String> = graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, _)| degree[*i] == 0)
        .map(|(_, n)| n.id.clone())
        .collect();
    let mut sizes: HashMap<usize, usize> = HashMap::new();
    for i in 0..node_count {
        *sizes.entry(uf.find(i)).or_insert(0) += 1;
    }
    let mut component_sizes: Vec<usize> = sizes.into_values().collect();
    component_sizes.sort_unstable_by(|a, b| b.cmp(a));
    let component_count = component_sizes.len();
    GraphStats {
        node_count,
        edge_count,
        isolated,
        component_count,
        component_sizes,
    }
}

/// The connected components of the graph as per-node membership lists.
///
/// This is the per-node companion to [graph_stats]: the stats say HOW MANY components
/// exist and their sizes, this names WHICH nodes belong to each. The model-health view
/// ("what is broken in my model?") needs the membership to list every member of an
/// isolated group rather than only counting it.
///
/// Deterministic: members are sorted by node id, and components are ordered by size
/// descending then by their first member id. A healthy model has exactly one component.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Components {
    pub count: usize,
    pub groups: Vec<Vec<String>>,
}

/// Compute the connected components of the graph and their membership. Edges whose
/// endpoint is not a node are ignored (they are the dangling links the health view reports
/// separately), so a component never contains a name that is not a real node.
pub fn components(root: &OkfRoot) -> Components {
    let graph = root.graph.as_ref().expect("graph required; validate first");
    let node_count = graph.nodes.len();
    let index: HashMap<&str, usize> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    let mut uf = UnionFind::new(node_count);
    for e in &graph.edges {
        if let (Some(&a), Some(&b)) = (index.get(e.source.as_str()), index.get(e.target.as_str())) {
            uf.union(a, b);
        }
    }
    let mut by_root: HashMap<usize, Vec<&str>> = HashMap::new();
    for (i, node) in graph.nodes.iter().enumerate() {
        by_root
            .entry(uf.find(i))
            .or_default()
            .push(node.id.as_str());
    }
    let mut groups: Vec<Vec<String>> = by_root
        .into_values()
        .map(|mut ids| {
            ids.sort_unstable();
            ids.into_iter().map(str::to_string).collect()
        })
        .collect();
    groups.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a[0].cmp(&b[0])));
    Components {
        count: groups.len(),
        groups,
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageReport {
    pub total: usize,
    pub satisfied: usize,
    pub refined: usize,
    pub verified: usize,
    pub allocated: usize,
    pub covered: usize,
    pub uncovered: Vec<String>,
}

/// Requirement coverage read from dependency edges whose label is exactly
/// Satisfy, Refine, Verify or Allocate.
pub fn requirement_coverage(root: &OkfRoot) -> CoverageReport {
    let req_ids: HashSet<&str> = root.requirements.iter().map(|r| r.id.as_str()).collect();
    let graph = root.graph.as_ref().expect("graph required; validate first");
    let mut satisfied = 0usize;
    let mut refined = 0usize;
    let mut verified = 0usize;
    let mut allocated = 0usize;
    let mut covered: HashSet<&str> = HashSet::new();
    for e in &graph.edges {
        if e.kind != "dependency" {
            continue;
        }
        let s = e.source.as_str();
        let t = e.target.as_str();
        match e.label.as_str() {
            "Satisfy" => {
                if req_ids.contains(t) {
                    satisfied += 1;
                    covered.insert(t);
                }
            }
            "Refine" => {
                if req_ids.contains(t) {
                    refined += 1;
                    covered.insert(t);
                }
            }
            "Verify" => {
                if req_ids.contains(t) {
                    verified += 1;
                    covered.insert(t);
                }
            }
            "Allocate" if req_ids.contains(t) || req_ids.contains(s) => {
                allocated += 1;
                // Count only endpoints that really are requirements. An allocate edge
                // can connect a function to a part, and inserting a non-requirement id
                // here would inflate the covered count reported in the gate evidence.
                if req_ids.contains(t) {
                    covered.insert(t);
                }
                if req_ids.contains(s) {
                    covered.insert(s);
                }
            }
            _ => {}
        }
    }
    let mut uncovered: Vec<String> = root
        .requirements
        .iter()
        .filter(|r| !covered.contains(r.id.as_str()))
        .map(|r| r.id.clone())
        .collect();
    uncovered.sort();
    CoverageReport {
        total: req_ids.len(),
        satisfied,
        refined,
        verified,
        allocated,
        covered: covered.len(),
        uncovered,
    }
}
