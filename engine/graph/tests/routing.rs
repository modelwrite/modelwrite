// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tests for the orthogonal edge router: anchors are side centres, segments are axis-aligned,
//! routes avoid node boxes, and the output is deterministic.

use graph::layout::NodeBox;
use graph::routing::{self, Side};

fn node(id: &str, x: f64, y: f64, w: f64, h: f64) -> NodeBox {
    NodeBox {
        id: id.to_string(),
        x,
        y,
        width: w,
        height: h,
    }
}

fn seg_axis_aligned(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-6 || (a.1 - b.1).abs() < 1e-6
}

fn seg_intersects_box(a: (f64, f64), b: (f64, f64), bx: &NodeBox) -> bool {
    // A segment intersects a box if it comes within the box's interior (with a small tolerance).
    let eps = 0.1;
    let (x0, x1) = if a.0 <= b.0 { (a.0, b.0) } else { (b.0, a.0) };
    let (y0, y1) = if a.1 <= b.1 { (a.1, b.1) } else { (b.1, a.1) };
    // Axis-aligned: overlap in the constant axis, and the varying axis spans the box.
    if (a.0 - b.0).abs() < 1e-6 {
        // vertical segment at x = a.0
        let inside_x = a.0 > bx.x + eps && a.0 < bx.x + bx.width - eps;
        let overlaps_y = y1 > bx.y + eps && y0 < bx.y + bx.height - eps;
        inside_x && overlaps_y
    } else {
        // horizontal segment at y = a.1
        let inside_y = a.1 > bx.y + eps && a.1 < bx.y + bx.height - eps;
        let overlaps_x = x1 > bx.x + eps && x0 < bx.x + bx.width - eps;
        inside_y && overlaps_x
    }
}

#[test]
fn same_row_routes_a_straight_line_between_side_centres() {
    let s = node("s", 0.0, 0.0, 100.0, 50.0);
    let t = node("t", 200.0, 0.0, 100.0, 50.0);
    let pts = routing::route(&s, &t, &[&s, &t]);
    assert_eq!(pts.len(), 2, "same row needs no elbow");
    assert_eq!(pts[0], (100.0, 25.0), "leaves right side centre");
    assert_eq!(pts[1], (200.0, 25.0), "arrives left side centre");
}

#[test]
fn diagonal_edge_routes_orthogonally() {
    let s = node("s", 0.0, 0.0, 100.0, 50.0);
    let t = node("t", 200.0, 150.0, 100.0, 50.0);
    let pts = routing::route(&s, &t, &[&s, &t]);
    assert!(pts.len() >= 3, "a diagonal needs an elbow");
    for pair in pts.windows(2) {
        assert!(
            seg_axis_aligned(pair[0], pair[1]),
            "segments are axis-aligned"
        );
    }
    assert_eq!(pts[0], (100.0, 25.0), "leaves source right side centre");
    assert_eq!(
        pts[pts.len() - 1],
        (200.0, 175.0),
        "arrives target left side centre"
    );
}

#[test]
fn vertical_edge_leaves_bottom_and_arrives_top() {
    let s = node("s", 0.0, 0.0, 100.0, 50.0);
    let t = node("t", 0.0, 150.0, 100.0, 50.0);
    let pts = routing::route(&s, &t, &[&s, &t]);
    assert_eq!(pts.len(), 2, "vertical drop is a straight line");
    assert_eq!(pts[0], (50.0, 50.0), "leaves bottom side centre");
    assert_eq!(pts[1], (50.0, 150.0), "arrives top side centre");
}

#[test]
fn route_avoids_an_intermediate_box() {
    // A box sits directly between source and target on the straight line; the route must turn
    // in a free channel rather than cross it.
    let s = node("s", 0.0, 0.0, 100.0, 50.0);
    let m = node("m", 150.0, 0.0, 100.0, 50.0);
    let t = node("t", 350.0, 0.0, 100.0, 50.0);
    let pts = routing::route(&s, &t, &[&s, &m, &t]);
    for pair in pts.windows(2) {
        assert!(
            !seg_intersects_box(pair[0], pair[1], &m),
            "route must not cross the intermediate box: {:?}->{:?}",
            pair[0],
            pair[1]
        );
    }
}

#[test]
fn route_does_not_cross_any_obstacle() {
    // A small cluster of boxes; every edge between distinct boxes must avoid all the others.
    let boxes = [
        node("a", 0.0, 0.0, 80.0, 40.0),
        node("b", 200.0, 0.0, 80.0, 40.0),
        node("c", 200.0, 120.0, 80.0, 40.0),
        node("d", 400.0, 60.0, 80.0, 40.0),
    ];
    let refs: Vec<&NodeBox> = boxes.iter().collect();
    for i in 0..boxes.len() {
        for j in 0..boxes.len() {
            if i == j {
                continue;
            }
            let pts = routing::route(&boxes[i], &boxes[j], &refs);
            for pair in pts.windows(2) {
                for k in 0..boxes.len() {
                    if k == i || k == j {
                        continue;
                    }
                    assert!(
                        !seg_intersects_box(pair[0], pair[1], &boxes[k]),
                        "edge {}-{} segment {:?}->{:?} crosses box {}",
                        boxes[i].id,
                        boxes[j].id,
                        pair[0],
                        pair[1],
                        boxes[k].id
                    );
                }
            }
        }
    }
}

#[test]
fn routing_is_deterministic() {
    let s = node("s", 0.0, 0.0, 100.0, 50.0);
    let t = node("t", 200.0, 150.0, 100.0, 50.0);
    let m = node("m", 150.0, 0.0, 100.0, 50.0);
    let refs = [&s, &t, &m];
    let a = routing::route(&s, &t, &refs);
    let b = routing::route(&s, &t, &refs);
    assert_eq!(a, b, "the same boxes always route identically");
}

#[test]
fn sides_are_facing_and_deterministic() {
    let s = node("s", 0.0, 0.0, 100.0, 50.0);
    let t = node("t", 200.0, 10.0, 100.0, 50.0);
    assert_eq!(routing::sides(&s, &t), (Side::Right, Side::Left));
    let above = node("above", 0.0, -200.0, 100.0, 50.0);
    assert_eq!(routing::sides(&s, &above), (Side::Top, Side::Bottom));
}

// ---------------------------------------------------------------------------
// Real-model validation: every routed edge must avoid every non-endpoint box.
// ---------------------------------------------------------------------------

use graph::layout::{self, DiagramLayout};
use okf::types::{Graph, GraphEdge, GraphNode, OkfRoot};

fn gnode(id: &str, kind: &str) -> GraphNode {
    GraphNode {
        id: id.to_string(),
        kind: kind.to_string(),
        name: id.to_string(),
        stereotypes: Vec::new(),
    }
}

fn gedge(source: &str, target: &str, kind: &str, label: &str) -> GraphEdge {
    GraphEdge {
        source: source.to_string(),
        target: target.to_string(),
        kind: kind.to_string(),
        label: label.to_string(),
    }
}

/// The cafe-stand graph (13 nodes, 15 edges) exactly as the e2e model declares it.
fn cafe_stand_graph() -> Graph {
    Graph {
        nodes: vec![
            gnode("cafe-stand", "block"),
            gnode("service-window", "block"),
            gnode("provisioning-bay", "block"),
            gnode("act-order", "activity"),
            gnode("act-pay", "activity"),
            gnode("act-brew", "activity"),
            gnode("act-toast", "activity"),
            gnode("act-complete", "activity"),
            gnode("act-clear-floor", "activity"),
            gnode("req-service-time", "requirement"),
            gnode("req-one-payment", "requirement"),
            gnode("req-no-early-provision", "requirement"),
            gnode("req-floor-clear", "requirement"),
        ],
        edges: vec![
            gedge("cafe-stand", "service-window", "part", "serviceWindow"),
            gedge("cafe-stand", "provisioning-bay", "part", "provisioningBay"),
            gedge("act-order", "act-pay", "include", "then"),
            gedge("act-pay", "act-brew", "triggers", "on PaymentApproved"),
            gedge("act-pay", "act-toast", "triggers", "on PaymentApproved"),
            gedge("act-brew", "act-complete", "include", "when BeverageReady"),
            gedge("act-toast", "act-complete", "include", "when FoodReady"),
            gedge("act-pay", "act-brew", "dependency", "Satisfy"),
            gedge("act-pay", "act-toast", "dependency", "Satisfy"),
            gedge(
                "act-complete",
                "req-no-early-provision",
                "dependency",
                "Satisfy",
            ),
            gedge("act-complete", "req-service-time", "dependency", "Satisfy"),
            gedge("act-pay", "req-one-payment", "dependency", "Satisfy"),
            gedge("act-pay", "act-clear-floor", "triggers", "service begins"),
            gedge(
                "act-clear-floor",
                "act-complete",
                "include",
                "continues during service",
            ),
            gedge(
                "act-clear-floor",
                "req-floor-clear",
                "dependency",
                "Satisfy",
            ),
        ],
    }
}

/// Every routed edge (both endpoints placed) must keep clear of every non-endpoint box.
fn assert_no_edge_crosses_a_box(graph: &Graph, layout: &DiagramLayout) {
    let box_by_id: std::collections::HashMap<&str, &NodeBox> =
        layout.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let all: Vec<&NodeBox> = layout.nodes.iter().collect();
    for edge in &graph.edges {
        let (Some(s), Some(t)) = (
            box_by_id.get(edge.source.as_str()),
            box_by_id.get(edge.target.as_str()),
        ) else {
            continue; // dangling endpoint, not placed
        };
        let pts = routing::route(s, t, &all);
        for w in pts.windows(2) {
            for other in &all {
                if other.id == s.id || other.id == t.id {
                    continue;
                }
                assert!(
                    !seg_intersects_box(w[0], w[1], other),
                    "edge {} -> {} segment {:?}->{:?} crosses box {}",
                    edge.source,
                    edge.target,
                    w[0],
                    w[1],
                    other.id
                );
            }
        }
    }
}

#[test]
fn corpus_structure_edges_avoid_every_box() {
    let root: OkfRoot =
        serde_json::from_str(&test_support::load_okf_expected()).expect("corpus parses");
    let g = root.graph.clone().expect("corpus graph");
    let layout = layout::structure_layout(&g);
    assert_no_edge_crosses_a_box(&g, &layout);
}

#[test]
fn cafe_stand_process_edges_avoid_every_box() {
    let g = cafe_stand_graph();
    let layout = layout::process_layout(&g).expect("cafe-stand has a process flow");
    assert_no_edge_crosses_a_box(&g, &layout);
}
