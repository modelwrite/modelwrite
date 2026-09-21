// SPDX-License-Identifier: AGPL-3.0-or-later
//! Deterministic layered graph layout.
//!
//! The workbench rule is that diagrams are RENDERED FROM THE MODEL, never hand-placed: the
//! data is the source of truth and the picture is a pure function of it. This module is that
//! function. Given an OKF graph it returns a box and a coordinate for every node plus the
//! canvas extent, and - because every iteration is over a sorted collection and every tie is
//! broken by node id - the same model always produces byte-identical geometry.
//!
//! Two layouts are provided:
//!
//! * [structure_layout] - a layered (Sugiyama-style) arrangement of the whole graph. Nodes are
//!   assigned a layer by longest path from the roots (the containment + traceability + flow
//!   edges), ordered within a layer by barycentre to reduce crossings (containment edges
//!   weighted double so a parent stays near its children), then given coordinates. Cycles are
//!   collapsed through strongly-connected-component condensation so a cyclic state machine
//!   cannot wedge the algorithm.
//! * [process_layout] - when activities are joined by include/triggers edges, a left-to-right
//!   flow view of those activities with the requirements they satisfy hanging beneath them.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

use okf::types::Graph;
use serde::Serialize;

// ---------------------------------------------------------------------------
// Public geometry types.
// ---------------------------------------------------------------------------

/// Horizontal and vertical breathing room for a layout. All three are pure inputs so a
/// caller can tune the density without touching the algorithm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutSpacing {
    /// Padding between the canvas edge and the outermost node.
    pub margin: f64,
    /// Horizontal gap between adjacent layers.
    pub h_gap: f64,
    /// Vertical gap between nodes stacked within a layer.
    pub v_gap: f64,
}

impl Default for LayoutSpacing {
    fn default() -> Self {
        Self {
            margin: 28.0,
            h_gap: 44.0,
            v_gap: 16.0,
        }
    }
}

/// The placed box of a single node: its top-left corner and its size. The renderer draws the
/// box exactly as sized here so the geometry of the picture and the geometry of the layout can
/// never disagree.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeBox {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl NodeBox {
    pub fn center_x(&self) -> f64 {
        self.x + self.width / 2.0
    }
    pub fn center_y(&self) -> f64 {
        self.y + self.height / 2.0
    }
}

/// A complete layout: one box per node (sorted by id) and the canvas extent.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagramLayout {
    pub nodes: Vec<NodeBox>,
    pub width: f64,
    pub height: f64,
}

impl DiagramLayout {
    /// The box for a node id, when the layout contains it.
    pub fn box_for(&self, id: &str) -> Option<&NodeBox> {
        self.nodes.iter().find(|n| n.id == id)
    }
}

// ---------------------------------------------------------------------------
// Node sizing.
// ---------------------------------------------------------------------------

/// The smallest box a node may occupy.
pub const NODE_MIN_W: f64 = 64.0;
/// The widest box a node may occupy; longer names truncate and expose their full text on hover.
pub const NODE_MAX_W: f64 = 260.0;
/// The height of every node box: a name line and a small kind line.
pub const NODE_H: f64 = 48.0;
/// The estimated horizontal advance per character, shared by sizing and truncation so the two
/// can never disagree about how much text fits a box.
pub const CHAR_W: f64 = 7.4;
/// The horizontal padding inside a node box (left + right), also shared.
pub const NODE_PAD_X: f64 = 30.0;

/// A deterministic estimate of a label's rendered width: every character contributes a fixed
/// em, so the same string always measures the same width. Pure, no font table, no system fonts.
pub fn node_size(label: &str) -> (f64, f64) {
    let chars = label.chars().count() as f64;
    let width = (chars * CHAR_W + NODE_PAD_X).clamp(NODE_MIN_W, NODE_MAX_W);
    (width, NODE_H)
}

/// Truncate a label to fit a box of the given width, appending an ellipsis when it must. The
/// renderer draws this truncated string and exposes the full label on hover via a title.
pub fn truncate_label(label: &str, width: f64) -> String {
    let max_chars = ((width - NODE_PAD_X) / CHAR_W).floor().max(1.0) as usize;
    let chars: Vec<char> = label.chars().collect();
    if chars.len() <= max_chars {
        return label.to_string();
    }
    if max_chars <= 1 {
        return format!("{}…", chars[0]);
    }
    chars[..max_chars - 1].iter().collect::<String>() + "…"
}

// ---------------------------------------------------------------------------
// Edge semantics.
// ---------------------------------------------------------------------------

/// The weight an edge carries in the barycentre ordering. Containment edges (part/contains)
/// count double so a parent and its children attract each other and hierarchy stays legible.
fn edge_weight(kind: &str) -> f64 {
    if kind == "part" || kind == "contains" {
        2.0
    } else {
        1.0
    }
}

/// Whether an edge kind participates in the layer (rank) assignment.
///
/// Association is deliberately excluded: in the corpus it points back up the part tree
/// (child -> parent), and giving it a direction would either create spurious cycles or drag a
/// parent down to its child's layer and flatten the hierarchy. Generalization points from the
/// specific to the general, so it is reversed (the general parent sits left of the specific).
fn rank_direction(kind: &str) -> Option<RankDir> {
    match kind {
        "association" => None,
        "generalization" => Some(RankDir::Reverse),
        _ => Some(RankDir::Forward),
    }
}

#[derive(Clone, Copy)]
enum RankDir {
    Forward,
    Reverse,
}

// ---------------------------------------------------------------------------
// Public entry points.
// ---------------------------------------------------------------------------

/// The layered view of the whole model.
pub fn structure_layout(graph: &Graph) -> DiagramLayout {
    // Nodes sorted by id: the index space every later step works in, and the order the
    // result is returned in, so determinism is structural rather than incidental.
    let mut nodes: Vec<&okf::types::GraphNode> = graph.nodes.iter().collect();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    let index: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();

    let ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();
    let labels: Vec<String> = nodes
        .iter()
        .map(|n| {
            if n.name.is_empty() {
                n.id.clone()
            } else {
                n.name.clone()
            }
        })
        .collect();

    let mut rank_edges: Vec<(usize, usize)> = Vec::new();
    let mut bary_edges: Vec<(usize, usize, f64)> = Vec::new();
    for edge in &graph.edges {
        let (Some(&u), Some(&v)) = (
            index.get(edge.source.as_str()),
            index.get(edge.target.as_str()),
        ) else {
            continue; // dangling endpoint: drawn by the renderer, not placed here
        };
        let weight = edge_weight(&edge.kind);
        bary_edges.push((u, v, weight));
        bary_edges.push((v, u, weight));
        match rank_direction(&edge.kind) {
            Some(RankDir::Forward) => rank_edges.push((u, v)),
            Some(RankDir::Reverse) => rank_edges.push((v, u)),
            None => {}
        }
    }

    layered_layout(
        &ids,
        &labels,
        &rank_edges,
        &bary_edges,
        &LayoutSpacing::default(),
    )
}

/// The process view, when the model has activities joined by include/triggers edges. Activities
/// are laid out left-to-right in flow order; the requirements they satisfy hang beneath them.
pub fn process_layout(graph: &Graph) -> Option<DiagramLayout> {
    let node_by_id: HashMap<&str, &okf::types::GraphNode> =
        graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    let activity_ids: HashSet<&str> = graph
        .nodes
        .iter()
        .filter(|n| n.kind == "activity")
        .map(|n| n.id.as_str())
        .collect();
    if activity_ids.is_empty() {
        return None;
    }

    // The flow edges between activities: include / triggers.
    let mut flow: Vec<(&str, &str)> = Vec::new();
    for edge in &graph.edges {
        if (edge.kind == "include" || edge.kind == "triggers")
            && activity_ids.contains(edge.source.as_str())
            && activity_ids.contains(edge.target.as_str())
        {
            flow.push((edge.source.as_str(), edge.target.as_str()));
        }
    }
    if flow.is_empty() {
        return None;
    }

    // The requirements the activities satisfy (dependency edges whose source is an activity and
    // whose target is a requirement node), so the process view shows what each step proves.
    let mut requirements: Vec<&str> = Vec::new();
    for edge in &graph.edges {
        if edge.kind != "dependency" {
            continue;
        }
        if !activity_ids.contains(edge.source.as_str()) {
            continue;
        }
        if let Some(node) = node_by_id.get(edge.target.as_str()) {
            if node.kind == "requirement" {
                requirements.push(edge.target.as_str());
            }
        }
    }
    requirements.sort_unstable();
    requirements.dedup();

    // Lay the activities out by flow order (longest path over the flow edges).
    let mut activity_nodes: Vec<&okf::types::GraphNode> = graph
        .nodes
        .iter()
        .filter(|n| n.kind == "activity")
        .collect();
    activity_nodes.sort_by(|a, b| a.id.cmp(&b.id));
    let activity_index: HashMap<&str, usize> = activity_nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    let activity_ids_sorted: Vec<String> = activity_nodes.iter().map(|n| n.id.clone()).collect();
    let activity_labels: Vec<String> = activity_nodes
        .iter()
        .map(|n| {
            if n.name.is_empty() {
                n.id.clone()
            } else {
                n.name.clone()
            }
        })
        .collect();

    let mut rank_edges: Vec<(usize, usize)> = Vec::new();
    let mut bary_edges: Vec<(usize, usize, f64)> = Vec::new();
    for (source, target) in &flow {
        if let (Some(&u), Some(&v)) = (activity_index.get(source), activity_index.get(target)) {
            rank_edges.push((u, v));
            bary_edges.push((u, v, 1.0));
            bary_edges.push((v, u, 1.0));
        }
    }

    let activity_layout = layered_layout(
        &activity_ids_sorted,
        &activity_labels,
        &rank_edges,
        &bary_edges,
        &LayoutSpacing::default(),
    );

    // Hang requirements beneath the activity that satisfies them (the first satisfier, by id,
    // when several activities feed one requirement). Requirements are sorted by id for stability.
    let spacing = LayoutSpacing::default();
    let activity_bottom = activity_layout
        .nodes
        .iter()
        .map(|n| n.y + n.height)
        .fold(spacing.margin, f64::max);

    // Map each requirement to the activities that satisfy it, grouped by satisfier so several
    // requirements of one step stack under it. The bucket order is sorted by requirement id.
    let mut under: HashMap<&str, Vec<String>> = HashMap::new();
    for edge in &graph.edges {
        if edge.kind != "dependency" || !activity_ids.contains(edge.source.as_str()) {
            continue;
        }
        if requirements.contains(&edge.target.as_str()) {
            let bucket = under.entry(edge.source.as_str()).or_default();
            if !bucket.contains(&edge.target) {
                bucket.push(edge.target.clone());
            }
        }
    }
    for bucket in under.values_mut() {
        bucket.sort_unstable();
    }

    let mut requirement_boxes: Vec<NodeBox> = Vec::new();
    let req_gap = 26.0;
    for activity in &activity_layout.nodes {
        let Some(bucket) = under.get(activity.id.as_str()).cloned() else {
            continue;
        };
        let mut y = activity_bottom + req_gap;
        for req_id in bucket {
            let name = node_by_id
                .get(req_id.as_str())
                .map(|n| {
                    if n.name.is_empty() {
                        n.id.clone()
                    } else {
                        n.name.clone()
                    }
                })
                .unwrap_or_else(|| req_id.clone());
            let (w, h) = node_size(&name);
            requirement_boxes.push(NodeBox {
                id: req_id,
                x: activity.x,
                y,
                width: w,
                height: h,
            });
            y += h + spacing.v_gap;
        }
    }

    // Merge, then sort the whole node list by id so the contract (sorted by id) holds.
    let mut all_nodes = activity_layout.nodes;
    all_nodes.extend(requirement_boxes);
    all_nodes.sort_by(|a, b| a.id.cmp(&b.id));

    let mut width = activity_layout.width;
    let mut height = activity_layout.height;
    for node in &all_nodes {
        width = width.max(node.x + node.width + spacing.margin);
        height = height.max(node.y + node.height + spacing.margin);
    }

    Some(DiagramLayout {
        nodes: all_nodes,
        width,
        height,
    })
}

/// The control-structure view, when the model declares control-structure nodes by STPA
/// stereotype (Controller, ControlledProcess, ControlAction, Feedback). Controllers, control
/// actions and controlled processes are laid left-to-right in the forward flow; feedback nodes
/// hang in a band beneath them, so the control loop reads as a loop rather than a generic
/// layered graph. Reuses the same layered engine as the other two views; no new geometry.
pub fn control_layout(graph: &Graph) -> Option<DiagramLayout> {
    use crate::stpa::{
        is_stereotype, STEREOTYPE_CONTROLLER, STEREOTYPE_CONTROL_ACTION, STEREOTYPE_FEEDBACK,
        STEREOTYPE_PROCESS,
    };

    let is_control_node = |n: &okf::types::GraphNode| {
        is_stereotype(n, STEREOTYPE_CONTROLLER)
            || is_stereotype(n, STEREOTYPE_PROCESS)
            || is_stereotype(n, STEREOTYPE_CONTROL_ACTION)
            || is_stereotype(n, STEREOTYPE_FEEDBACK)
    };
    let control_count = graph.nodes.iter().filter(|n| is_control_node(n)).count();
    if control_count == 0 {
        return None;
    }

    // The forward row: controllers, control actions and controlled processes. Feedback is
    // placed separately below because its flow runs back toward the controller.
    let is_forward = |n: &okf::types::GraphNode| {
        is_stereotype(n, STEREOTYPE_CONTROLLER)
            || is_stereotype(n, STEREOTYPE_CONTROL_ACTION)
            || is_stereotype(n, STEREOTYPE_PROCESS)
    };
    let mut forward: Vec<&okf::types::GraphNode> =
        graph.nodes.iter().filter(|n| is_forward(n)).collect();
    forward.sort_by(|a, b| a.id.cmp(&b.id));
    let forward_index: HashMap<&str, usize> = forward
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    let forward_ids: Vec<String> = forward.iter().map(|n| n.id.clone()).collect();
    let forward_labels: Vec<String> = forward
        .iter()
        .map(|n| {
            if n.name.is_empty() {
                n.id.clone()
            } else {
                n.name.clone()
            }
        })
        .collect();

    let mut rank_edges: Vec<(usize, usize)> = Vec::new();
    let mut bary_edges: Vec<(usize, usize, f64)> = Vec::new();
    for edge in &graph.edges {
        if edge.kind != "triggers" {
            continue;
        }
        if let (Some(&u), Some(&v)) = (
            forward_index.get(edge.source.as_str()),
            forward_index.get(edge.target.as_str()),
        ) {
            rank_edges.push((u, v));
            bary_edges.push((u, v, 1.0));
            bary_edges.push((v, u, 1.0));
        }
    }

    let forward_layout = layered_layout(
        &forward_ids,
        &forward_labels,
        &rank_edges,
        &bary_edges,
        &LayoutSpacing::default(),
    );

    // Feedback nodes in a band beneath the forward row, sorted by id for determinism.
    let spacing = LayoutSpacing::default();
    let row_bottom = forward_layout
        .nodes
        .iter()
        .map(|n| n.y + n.height)
        .fold(spacing.margin, f64::max);
    let fb_gap = 56.0;
    let mut feedback: Vec<&okf::types::GraphNode> = graph
        .nodes
        .iter()
        .filter(|n| is_stereotype(n, STEREOTYPE_FEEDBACK))
        .collect();
    feedback.sort_by(|a, b| a.id.cmp(&b.id));
    let mut feedback_boxes: Vec<NodeBox> = Vec::new();
    let mut cursor = spacing.margin;
    for node in &feedback {
        let label = if node.name.is_empty() {
            node.id.clone()
        } else {
            node.name.clone()
        };
        let (w, h) = node_size(&label);
        feedback_boxes.push(NodeBox {
            id: node.id.clone(),
            x: cursor,
            y: row_bottom + fb_gap,
            width: w,
            height: h,
        });
        cursor += w + spacing.h_gap;
    }

    let mut all_nodes = forward_layout.nodes;
    all_nodes.extend(feedback_boxes);
    all_nodes.sort_by(|a, b| a.id.cmp(&b.id));

    let mut width = forward_layout.width;
    let mut height = forward_layout.height;
    for node in &all_nodes {
        width = width.max(node.x + node.width + spacing.margin);
        height = height.max(node.y + node.height + spacing.margin);
    }

    Some(DiagramLayout {
        nodes: all_nodes,
        width,
        height,
    })
}

// ---------------------------------------------------------------------------
// The layered engine shared by both layouts.
// ---------------------------------------------------------------------------

fn layered_layout(
    ids: &[String],
    labels: &[String],
    rank_edges: &[(usize, usize)],
    bary_edges: &[(usize, usize, f64)],
    spacing: &LayoutSpacing,
) -> DiagramLayout {
    let n = ids.len();
    let sizes: Vec<(f64, f64)> = labels.iter().map(|l| node_size(l)).collect();

    let layers = assign_layers(n, rank_edges);
    let mut layers = settle_isolates(n, rank_edges, bary_edges, layers);
    let order = order_layers(&mut layers, n, bary_edges);
    let (positions, width, height) = assign_positions(&order, &sizes, spacing);

    let mut nodes: Vec<NodeBox> = (0..n)
        .map(|i| NodeBox {
            id: ids[i].clone(),
            x: positions[i].0,
            y: positions[i].1,
            width: sizes[i].0,
            height: sizes[i].1,
        })
        .collect();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    DiagramLayout {
        nodes,
        width,
        height,
    }
}

/// Assign a layer to every node by longest path from the roots, collapsing cycles first so the
/// relaxation always terminates. Roots (nodes with no incoming rank edge) sit at layer 0.
fn assign_layers(n: usize, rank_edges: &[(usize, usize)]) -> Vec<usize> {
    // Deduplicated, sorted adjacency (sorted neighbours keep the DFS deterministic).
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(u, v) in rank_edges {
        adj[u].push(v);
    }
    for list in adj.iter_mut() {
        list.sort_unstable();
        list.dedup();
    }

    // Strongly connected components: a cyclic subgraph (a state machine's transitions, say)
    // becomes one component and one layer, so the longest-path relaxation below cannot loop.
    let comps = kosaraju(n, &adj);
    let comp_count = comps.len();
    let mut node_comp = vec![0usize; n];
    for (ci, comp) in comps.iter().enumerate() {
        for &u in comp {
            node_comp[u] = ci;
        }
    }

    // Condensation: components as nodes, deduplicated edges between them.
    let mut cadj: Vec<Vec<usize>> = vec![Vec::new(); comp_count];
    for u in 0..n {
        for &v in &adj[u] {
            let cu = node_comp[u];
            let cv = node_comp[v];
            if cu != cv {
                cadj[cu].push(cv);
            }
        }
    }
    let mut indeg = vec![0usize; comp_count];
    for list in &mut cadj {
        list.sort_unstable();
        list.dedup();
        for &cv in list.iter() {
            indeg[cv] += 1;
        }
    }

    // Longest path over the condensation DAG, in a deterministic topological order (Kahn with a
    // min-heap on component index).
    let mut layer = vec![0usize; comp_count];
    let mut remaining = indeg.clone();
    let mut heap: BinaryHeap<Reverse<usize>> = (0..comp_count)
        .filter(|&i| remaining[i] == 0)
        .map(Reverse)
        .collect();
    while let Some(Reverse(cu)) = heap.pop() {
        for &cv in &cadj[cu] {
            if layer[cv] < layer[cu] + 1 {
                layer[cv] = layer[cu] + 1;
            }
            remaining[cv] -= 1;
            if remaining[cv] == 0 {
                heap.push(Reverse(cv));
            }
        }
    }

    (0..n).map(|u| layer[node_comp[u]]).collect()
}

/// Kosaraju's algorithm, iterative-free and deterministic: neighbours are visited in sorted
/// order, so the finish order and therefore the component partition are a pure function of the
/// sorted adjacency.
fn kosaraju(n: usize, adj: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let mut rev: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (u, list) in adj.iter().enumerate() {
        for &v in list {
            rev[v].push(u);
        }
    }
    for list in rev.iter_mut() {
        list.sort_unstable();
        list.dedup();
    }

    fn dfs(u: usize, graph: &[Vec<usize>], visited: &mut [bool], out: &mut Vec<usize>) {
        visited[u] = true;
        for &v in &graph[u] {
            if !visited[v] {
                dfs(v, graph, visited, out);
            }
        }
        out.push(u);
    }

    let mut visited = vec![false; n];
    let mut finish = Vec::with_capacity(n);
    for u in 0..n {
        if !visited[u] {
            dfs(u, adj, &mut visited, &mut finish);
        }
    }

    let mut visited = vec![false; n];
    let mut comps: Vec<Vec<usize>> = Vec::new();
    for &u in finish.iter().rev() {
        if !visited[u] {
            let mut comp = Vec::new();
            dfs(u, &rev, &mut visited, &mut comp);
            comp.sort_unstable();
            comps.push(comp);
        }
    }
    // Sort components by their smallest member so the condensation is stable.
    comps.sort_by_key(|c| c[0]);
    comps
}

/// Nodes that carry no rank edge at all (they are joined to the rest of the model only by
/// association, say) inherit a neighbour's layer so they stay beside what they connect to
/// instead of collapsing to the far-left root column.
fn settle_isolates(
    n: usize,
    rank_edges: &[(usize, usize)],
    bary_edges: &[(usize, usize, f64)],
    mut layers: Vec<usize>,
) -> Vec<usize> {
    let mut rank_degree = vec![0usize; n];
    for &(u, v) in rank_edges {
        rank_degree[u] += 1;
        rank_degree[v] += 1;
    }
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(u, v, _) in bary_edges {
        adj[u].push(v);
    }
    for list in adj.iter_mut() {
        list.sort_unstable();
        list.dedup();
    }
    // A few fixed-point passes: an isolated node adopts the layer of the first ranked neighbour.
    for _ in 0..n {
        let mut changed = false;
        for u in 0..n {
            if rank_degree[u] != 0 {
                continue;
            }
            for &v in &adj[u] {
                if rank_degree[v] != 0 && layers[u] != layers[v] {
                    layers[u] = layers[v];
                    changed = true;
                    break;
                }
            }
        }
        if !changed {
            break;
        }
    }
    layers
}

/// Order nodes within each layer by barycentre, two passes (forward then backward) plus a final
/// forward pass. Tie-breaks are by node index (== node id), so the ordering is deterministic.
fn order_layers(
    layers: &mut [usize],
    n: usize,
    bary_edges: &[(usize, usize, f64)],
) -> Vec<Vec<usize>> {
    let max_layer = layers.iter().copied().max().unwrap_or(0);
    let mut order: Vec<Vec<usize>> = vec![Vec::new(); max_layer + 1];
    for u in 0..n {
        order[layers[u]].push(u);
    }

    // Weighted undirected adjacency, for looking up neighbours and their weights.
    let mut adj: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
    for &(u, v, w) in bary_edges {
        adj[u].push((v, w));
    }
    for list in adj.iter_mut() {
        list.sort_by_key(|a| a.0);
        list.dedup_by(|a, b| a.0 == b.0);
    }

    // Forward pass: order each layer by the barycentre of its predecessors in the layer to the
    // left (already ordered).
    for layer in 1..=max_layer {
        let prev_order = order[layer - 1].clone();
        let prev_pos: HashMap<usize, usize> = prev_order
            .iter()
            .enumerate()
            .map(|(pos, &u)| (u, pos))
            .collect();
        sort_layer(&mut order[layer], &adj, &prev_pos);
    }
    // Backward pass: order each layer by the barycentre of its successors to the right.
    for layer in (0..max_layer).rev() {
        let next_order = order[layer + 1].clone();
        let next_pos: HashMap<usize, usize> = next_order
            .iter()
            .enumerate()
            .map(|(pos, &u)| (u, pos))
            .collect();
        sort_layer(&mut order[layer], &adj, &next_pos);
    }
    // A final forward pass so the leftmost layers settle after the backward rearrangement.
    for layer in 1..=max_layer {
        let prev_order = order[layer - 1].clone();
        let prev_pos: HashMap<usize, usize> = prev_order
            .iter()
            .enumerate()
            .map(|(pos, &u)| (u, pos))
            .collect();
        sort_layer(&mut order[layer], &adj, &prev_pos);
    }
    order
}

/// Sort one layer by the weighted mean position of each node's neighbours in the reference
/// layer. Nodes with no neighbour keep their relative order through the id tie-break.
fn sort_layer(
    layer: &mut [usize],
    adj: &[Vec<(usize, f64)>],
    reference_pos: &HashMap<usize, usize>,
) {
    layer.sort_by(|&a, &b| {
        let ba = barycentre(adj[a].as_slice(), reference_pos);
        let bb = barycentre(adj[b].as_slice(), reference_pos);
        ba.partial_cmp(&bb)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.cmp(&b))
    });
}

fn barycentre(neighbours: &[(usize, f64)], positions: &HashMap<usize, usize>) -> f64 {
    let mut total_w = 0.0;
    let mut sum = 0.0;
    for &(v, w) in neighbours {
        if let Some(&pos) = positions.get(&v) {
            total_w += w;
            sum += pos as f64 * w;
        }
    }
    if total_w == 0.0 {
        // No neighbour in the reference layer: park at the far end, but still deterministic
        // because the id tie-break orders equal values.
        f64::INFINITY
    } else {
        sum / total_w
    }
}

/// Turn the ordered layers into concrete coordinates: columns left-to-right, nodes stacked and
/// vertically centred within the tallest layer, the whole canvas padded by the margin.
fn assign_positions(
    order: &[Vec<usize>],
    sizes: &[(f64, f64)],
    spacing: &LayoutSpacing,
) -> (Vec<(f64, f64)>, f64, f64) {
    let n_layers = order.len();
    let mut layer_w = vec![0.0f64; n_layers];
    let mut layer_h = vec![0.0f64; n_layers];
    for (i, layer) in order.iter().enumerate() {
        for &u in layer {
            layer_w[i] = layer_w[i].max(sizes[u].0);
        }
        if !layer.is_empty() {
            let sum: f64 = layer.iter().map(|&u| sizes[u].1).sum();
            layer_h[i] = sum + (layer.len() - 1) as f64 * spacing.v_gap;
        }
    }
    let max_h = layer_h.iter().cloned().fold(0.0, f64::max);

    let mut x_off = vec![0.0f64; n_layers];
    let mut cursor = spacing.margin;
    for i in 0..n_layers {
        x_off[i] = cursor;
        cursor += layer_w[i] + spacing.h_gap;
    }
    let width = cursor - spacing.h_gap + spacing.margin;

    let mut positions = vec![(0.0, 0.0); sizes.len()];
    for (i, layer) in order.iter().enumerate() {
        let mut y = spacing.margin + (max_h - layer_h[i]) / 2.0;
        for &u in layer {
            positions[u] = (x_off[i], y);
            y += sizes[u].1 + spacing.v_gap;
        }
    }
    let height = spacing.margin + max_h + spacing.margin;
    (positions, width, height)
}
