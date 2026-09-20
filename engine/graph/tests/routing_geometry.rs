// SPDX-License-Identifier: AGPL-3.0-or-later
//! Geometry assertions for the graph-level router. These are the strongest tests for an
//! engineering diagram: an edge never passes through a node box, edges leaving one source leave
//! from distinct points, every arrowhead lands on its target's border (never a corner), crossings
//! carry a jump, and the whole route is a pure function of the graph.

use std::collections::HashMap;

use graph::layout::{self, DiagramLayout, NodeBox};
use graph::routing::{self, EdgeRequest, Lane, RoutedEdge};
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
            gnode("req-service-time", "requirement"),
            gnode("req-one-payment", "requirement"),
            gnode("req-no-early-provision", "requirement"),
            gnode("act-clear-floor", "activity"),
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

fn lane_for(kind: &str) -> Lane {
    match kind {
        "include" | "triggers" | "transition" => Lane::Flow,
        "dependency" => Lane::Dependency,
        _ => Lane::Other,
    }
}

/// The draw-order-ordered routes of every node-to-node, non-self edge, alongside the endpoint
/// ids. The index assigned to each request is its draw position, exactly as the renderer does.
fn route_all(graph: &Graph, layout: &DiagramLayout) -> Vec<(String, String, Lane, RoutedEdge)> {
    let box_by_id: HashMap<&str, &NodeBox> =
        layout.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut edges: Vec<&GraphEdge> = graph.edges.iter().collect();
    edges.sort_by(|a, b| {
        (&a.source, &a.target, &a.kind, &a.label).cmp(&(&b.source, &b.target, &b.kind, &b.label))
    });
    let mut requests: Vec<EdgeRequest> = Vec::new();
    let mut meta: Vec<(String, String, Lane)> = Vec::new();
    for (pos, e) in edges.iter().enumerate() {
        let node_to_node =
            box_by_id.contains_key(e.source.as_str()) && box_by_id.contains_key(e.target.as_str());
        if !node_to_node || e.source == e.target {
            continue;
        }
        requests.push(EdgeRequest {
            source: e.source.clone(),
            target: e.target.clone(),
            lane: lane_for(&e.kind),
            index: pos,
        });
        meta.push((e.source.clone(), e.target.clone(), lane_for(&e.kind)));
    }
    meta.into_iter()
        .zip(routing::route_graph(&layout.nodes, &requests))
        .map(|((s, t, l), r)| (s, t, l, r))
        .collect()
}

fn box_for<'a>(layout: &'a DiagramLayout, id: &str) -> &'a NodeBox {
    layout
        .nodes
        .iter()
        .find(|n| n.id == id)
        .unwrap_or_else(|| panic!("node {id} not in layout"))
}

/// Whether an axis-aligned segment passes through the interior of a box.
fn seg_intersects_box(a: (f64, f64), b: (f64, f64), bx: &NodeBox) -> bool {
    let eps = 0.1;
    let (x0, x1) = if a.0 <= b.0 { (a.0, b.0) } else { (b.0, a.0) };
    let (y0, y1) = if a.1 <= b.1 { (a.1, b.1) } else { (b.1, a.1) };
    if (a.0 - b.0).abs() < 1e-6 {
        let inside_x = a.0 > bx.x + eps && a.0 < bx.x + bx.width - eps;
        let overlaps_y = y1 > bx.y + eps && y0 < bx.y + bx.height - eps;
        inside_x && overlaps_y
    } else {
        let inside_y = a.1 > bx.y + eps && a.1 < bx.y + bx.height - eps;
        let overlaps_x = x1 > bx.x + eps && x0 < bx.x + bx.width - eps;
        inside_y && overlaps_x
    }
}

/// The proper crossing point of two axis-aligned segments, or None when parallel/touching.
fn segs_cross(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> Option<(f64, f64)> {
    let ah = (a.1 - b.1).abs() < 1e-6;
    let ch = (c.1 - d.1).abs() < 1e-6;
    if ah == ch {
        return None;
    }
    if ah {
        let (xlo, xhi) = if a.0 <= b.0 { (a.0, b.0) } else { (b.0, a.0) };
        let (ylo, yhi) = if c.1 <= d.1 { (c.1, d.1) } else { (d.1, c.1) };
        let x = c.0;
        let y = a.1;
        if x > xlo + 1e-6 && x < xhi - 1e-6 && y > ylo + 1e-6 && y < yhi - 1e-6 {
            Some((x, y))
        } else {
            None
        }
    } else {
        let (xlo, xhi) = if c.0 <= d.0 { (c.0, d.0) } else { (d.0, c.0) };
        let (ylo, yhi) = if a.1 <= b.1 { (a.1, b.1) } else { (b.1, a.1) };
        let x = a.0;
        let y = c.1;
        if x > xlo + 1e-6 && x < xhi - 1e-6 && y > ylo + 1e-6 && y < yhi - 1e-6 {
            Some((x, y))
        } else {
            None
        }
    }
}

/// Every segment of every routed edge must keep clear of every non-endpoint node box.
fn assert_no_edge_crosses_a_box(graph: &Graph, layout: &DiagramLayout) {
    for (src, tgt, _lane, route) in route_all(graph, layout) {
        let sb = box_for(layout, &src);
        let tb = box_for(layout, &tgt);
        for w in route.points.windows(2) {
            for other in &layout.nodes {
                if other.id == sb.id || other.id == tb.id {
                    continue;
                }
                assert!(
                    !seg_intersects_box(w[0], w[1], other),
                    "edge {} -> {} segment {:?}->{:?} crosses box {}",
                    src,
                    tgt,
                    w[0],
                    w[1],
                    other.id
                );
            }
        }
    }
}

/// No two edges leaving the same source may leave from the same point.
fn assert_distinct_source_anchors(graph: &Graph, layout: &DiagramLayout) {
    let mut by_src: HashMap<String, Vec<(f64, f64)>> = HashMap::new();
    for (src, _tgt, _lane, route) in route_all(graph, layout) {
        by_src.entry(src).or_default().push(route.points[0]);
    }
    for (src, pts) in by_src {
        for i in 0..pts.len() {
            for j in (i + 1)..pts.len() {
                let (ax, ay) = pts[i];
                let (bx, by) = pts[j];
                assert!(
                    (ax - bx).abs() > 1e-3 || (ay - by).abs() > 1e-3,
                    "two edges leave {} from the same point {:?}",
                    src,
                    pts[i]
                );
            }
        }
    }
}

/// Whether a point lies on a node's border, away from a corner.
fn on_border(p: (f64, f64), b: &NodeBox) -> bool {
    let tol = 0.6;
    let corner = 2.0;
    let on_left = (p.0 - b.x).abs() < tol && p.1 > b.y + corner && p.1 < b.y + b.height - corner;
    let on_right =
        (p.0 - (b.x + b.width)).abs() < tol && p.1 > b.y + corner && p.1 < b.y + b.height - corner;
    let on_top = (p.1 - b.y).abs() < tol && p.0 > b.x + corner && p.0 < b.x + b.width - corner;
    let on_bottom =
        (p.1 - (b.y + b.height)).abs() < tol && p.0 > b.x + corner && p.0 < b.x + b.width - corner;
    on_left || on_right || on_top || on_bottom
}

/// Every arrowhead (the route's last point) must land on its target's border, not a corner.
fn assert_arrowheads_on_border(graph: &Graph, layout: &DiagramLayout) {
    for (src, tgt, _lane, route) in route_all(graph, layout) {
        let tb = box_for(layout, &tgt);
        let tip = *route.points.last().expect("route has points");
        assert!(
            on_border(tip, tb),
            "arrowhead of {} -> {} lands at {:?}, off {}'s border",
            src,
            tgt,
            tip,
            tgt
        );
    }
}

/// Every crossing must be marked by a jump on the later-drawn edge.
fn assert_crossings_have_jumps(graph: &Graph, layout: &DiagramLayout) {
    let routes = route_all(graph, layout);
    for a in 0..routes.len() {
        for b in (a + 1)..routes.len() {
            let (ra, rb) = (&routes[a].3, &routes[b].3);
            for seg_a in ra.points.windows(2) {
                for seg_b in rb.points.windows(2) {
                    if let Some((px, py)) = segs_cross(seg_a[0], seg_a[1], seg_b[0], seg_b[1]) {
                        let marked = rb
                            .jumps
                            .iter()
                            .any(|j| (j.x - px).abs() < 1e-3 && (j.y - py).abs() < 1e-3);
                        assert!(
                            marked,
                            "crossing at {:?} between {}->{} and {}->{} is not marked with a jump",
                            (px, py),
                            routes[a].0,
                            routes[a].1,
                            routes[b].0,
                            routes[b].1
                        );
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The cafe-stand process diagram (fan-out, converging arrowheads, Satisfy lanes).
// ---------------------------------------------------------------------------

#[test]
fn process_routes_avoid_every_box() {
    let g = cafe_stand_graph();
    let layout = layout::process_layout(&g).expect("cafe-stand has a process flow");
    assert_no_edge_crosses_a_box(&g, &layout);
}

#[test]
fn process_edges_leaving_a_source_share_no_anchor() {
    let g = cafe_stand_graph();
    let layout = layout::process_layout(&g).expect("cafe-stand has a process flow");
    assert_distinct_source_anchors(&g, &layout);
}

#[test]
fn process_arrowheads_land_on_target_borders() {
    let g = cafe_stand_graph();
    let layout = layout::process_layout(&g).expect("cafe-stand has a process flow");
    assert_arrowheads_on_border(&g, &layout);
}

#[test]
fn process_crossings_carry_jumps() {
    let g = cafe_stand_graph();
    let layout = layout::process_layout(&g).expect("cafe-stand has a process flow");
    assert_crossings_have_jumps(&g, &layout);
}

#[test]
fn process_fanout_has_no_crossings() {
    // The three flow edges leaving Take Payment toward the provisioning steps share a trunk, so
    // no two edges that leave the SAME source in the SAME lane may cross each other (the original
    // defect was a self-crossing fan).
    let g = cafe_stand_graph();
    let layout = layout::process_layout(&g).expect("cafe-stand has a process flow");
    let routes = route_all(&g, &layout);
    for a in 0..routes.len() {
        for b in (a + 1)..routes.len() {
            if routes[a].0 != routes[b].0 || routes[a].2 != routes[b].2 {
                continue;
            }
            for seg_a in routes[a].3.points.windows(2) {
                for seg_b in routes[b].3.points.windows(2) {
                    assert!(
                        segs_cross(seg_a[0], seg_a[1], seg_b[0], seg_b[1]).is_none(),
                        "edges {}->{} and {}->{} cross; the fan-out trunk must keep them apart",
                        routes[a].0,
                        routes[a].1,
                        routes[b].0,
                        routes[b].1
                    );
                }
            }
        }
    }
}

#[test]
fn process_routing_is_deterministic() {
    let g = cafe_stand_graph();
    let layout = layout::process_layout(&g).expect("cafe-stand has a process flow");
    let a = route_all(&g, &layout);
    let b = route_all(&g, &layout);
    assert_eq!(a, b, "the same model must route identically");
}

// ---------------------------------------------------------------------------
// The 99-node coffee-machine structure diagram.
// ---------------------------------------------------------------------------

fn structure() -> (Graph, DiagramLayout) {
    let root: OkfRoot =
        serde_json::from_str(&test_support::load_okf_expected()).expect("corpus parses");
    let g = root.graph.clone().expect("corpus graph");
    let layout = layout::structure_layout(&g);
    (g, layout)
}

#[test]
fn structure_routes_avoid_every_box() {
    let (g, layout) = structure();
    assert_no_edge_crosses_a_box(&g, &layout);
}

#[test]
fn structure_edges_leaving_a_source_share_no_anchor() {
    let (g, layout) = structure();
    assert_distinct_source_anchors(&g, &layout);
}

#[test]
fn structure_arrowheads_land_on_target_borders() {
    let (g, layout) = structure();
    assert_arrowheads_on_border(&g, &layout);
}

#[test]
fn structure_crossings_carry_jumps() {
    let (g, layout) = structure();
    assert_crossings_have_jumps(&g, &layout);
}

#[test]
fn structure_routing_is_deterministic() {
    let (g, layout) = structure();
    let a = route_all(&g, &layout);
    let b = route_all(&g, &layout);
    assert_eq!(a, b, "the same model must route identically");
}
