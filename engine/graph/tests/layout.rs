// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tests for the deterministic layered diagram layout. The layout is a pure function of
//! the model: the same model must produce byte-identical coordinates, every node must be
//! placed, boxes must not overlap, containment edges must point left-to-right (parent
//! before child), and the process view must order the cafe-stand flow with its parallel
//! branch side by side.

use graph::layout::{self, DiagramLayout, LayoutSpacing};
use okf::types::{Graph, GraphEdge, GraphNode, OkfRoot};

fn expected() -> OkfRoot {
    serde_json::from_str(&test_support::load_okf_expected()).expect("corpus fixture parses")
}

fn graph(nodes: Vec<(&str, &str, &str)>, edges: Vec<(&str, &str, &str, &str)>) -> Graph {
    Graph {
        nodes: nodes
            .into_iter()
            .map(|(id, kind, name)| GraphNode {
                id: id.to_string(),
                kind: kind.to_string(),
                name: name.to_string(),
                stereotypes: Vec::new(),
            })
            .collect(),
        edges: edges
            .into_iter()
            .map(|(source, target, kind, label)| GraphEdge {
                source: source.to_string(),
                target: target.to_string(),
                kind: kind.to_string(),
                label: label.to_string(),
            })
            .collect(),
    }
}

fn box_for<'a>(layout: &'a DiagramLayout, id: &str) -> &'a layout::NodeBox {
    layout
        .nodes
        .iter()
        .find(|n| n.id == id)
        .unwrap_or_else(|| panic!("node {id} not in layout"))
}

#[test]
fn structure_layout_is_deterministic_and_byte_identical() {
    let g = expected().graph.clone().expect("corpus graph");
    let a = layout::structure_layout(&g);
    let b = layout::structure_layout(&g);
    assert_eq!(a, b, "two runs of the same model must agree");
    let json_a = serde_json::to_string(&a).expect("serialize");
    let json_b = serde_json::to_string(&b).expect("serialize");
    assert_eq!(json_a, json_b, "serialized layout must be byte-identical");
}

#[test]
fn structure_layout_covers_every_node_sorted_by_id() {
    let g = expected().graph.clone().expect("corpus graph");
    let layout = layout::structure_layout(&g);
    assert_eq!(layout.nodes.len(), 99, "every corpus node is placed");
    let mut expected_ids: Vec<String> = g.nodes.iter().map(|n| n.id.clone()).collect();
    expected_ids.sort();
    let actual_ids: Vec<String> = layout.nodes.iter().map(|n| n.id.clone()).collect();
    assert_eq!(actual_ids, expected_ids, "nodes are returned sorted by id");
    // canvas has positive extent
    assert!(layout.width > 0.0 && layout.height > 0.0);
}

#[test]
fn structure_layout_boxes_do_not_overlap() {
    let g = expected().graph.clone().expect("corpus graph");
    let layout = layout::structure_layout(&g);
    for i in 0..layout.nodes.len() {
        for j in (i + 1)..layout.nodes.len() {
            let a = &layout.nodes[i];
            let b = &layout.nodes[j];
            let overlap_x = a.x < b.x + b.width && b.x < a.x + a.width;
            let overlap_y = a.y < b.y + b.height && b.y < a.y + a.height;
            assert!(
                !(overlap_x && overlap_y),
                "nodes {} and {} overlap",
                a.id,
                b.id
            );
        }
    }
}

#[test]
fn containment_edges_point_left_to_right() {
    let g = expected().graph.clone().expect("corpus graph");
    let layout = layout::structure_layout(&g);
    for edge in &g.edges {
        if edge.kind != "part" && edge.kind != "contains" {
            continue;
        }
        let parent = box_for(&layout, &edge.source);
        let child = box_for(&layout, &edge.target);
        assert!(
            parent.x < child.x,
            "containment parent {} must be left of child {}",
            edge.source,
            edge.target
        );
    }
}

#[test]
fn process_layout_orders_the_cafe_stand_flow_with_a_parallel_branch() {
    let g = graph(
        vec![
            ("act-order", "activity", "Take Order"),
            ("act-pay", "activity", "Take Payment"),
            ("act-brew", "activity", "Provision Beverage"),
            ("act-toast", "activity", "Provision Food"),
            ("act-complete", "activity", "Complete Order"),
            ("req-one", "requirement", "One Payment"),
            ("req-service", "requirement", "Service Time"),
        ],
        vec![
            ("act-order", "act-pay", "include", "then"),
            ("act-pay", "act-brew", "triggers", "on PaymentApproved"),
            ("act-pay", "act-toast", "triggers", "on PaymentApproved"),
            ("act-brew", "act-complete", "include", "when BeverageReady"),
            ("act-toast", "act-complete", "include", "when FoodReady"),
            ("act-pay", "req-one", "dependency", "Satisfy"),
            ("act-complete", "req-service", "dependency", "Satisfy"),
        ],
    );
    let layout = layout::process_layout(&g).expect("cafe-stand has a process flow");

    let order = box_for(&layout, "act-order");
    let pay = box_for(&layout, "act-pay");
    let brew = box_for(&layout, "act-brew");
    let toast = box_for(&layout, "act-toast");
    let complete = box_for(&layout, "act-complete");

    // Left-to-right flow order.
    assert!(order.x < pay.x, "Take Order before Take Payment");
    assert!(pay.x < brew.x, "Take Payment before Provision Beverage");
    assert!(pay.x < toast.x, "Take Payment before Provision Food");
    assert!(
        brew.x < complete.x,
        "Provision Beverage before Complete Order"
    );
    assert!(toast.x < complete.x, "Provision Food before Complete Order");
    // The parallel branch sits side by side at the same column.
    assert!(
        (brew.x - toast.x).abs() < 0.001,
        "the two provisioning steps are one parallel layer"
    );

    // Requirements are present and sit below the activities.
    let req_one = box_for(&layout, "req-one");
    let req_service = box_for(&layout, "req-service");
    assert!(
        req_one.y > pay.y,
        "requirements hang below the activity row"
    );
    assert!(
        req_service.y > complete.y,
        "requirements hang below the activity row"
    );
}

#[test]
fn process_layout_is_none_without_an_activity_flow() {
    // The corpus has no activity-to-activity include/triggers edges, so it has no process view.
    let g = expected().graph.clone().expect("corpus graph");
    assert!(layout::process_layout(&g).is_none());
}

#[test]
fn node_size_grows_with_label_and_stays_bounded() {
    let (w_short, h) = layout::node_size("A");
    let (w_long, _) = layout::node_size("A very long element name that keeps going");
    assert!(w_short < w_long, "wider labels get wider boxes");
    assert!(h > 0.0);
    assert!(w_long <= layout::NODE_MAX_W + 1e-9, "width is bounded");
    assert!(w_short >= layout::NODE_MIN_W - 1e-9, "width has a floor");
}

#[test]
fn empty_graph_layout_is_empty_and_stable() {
    let g = Graph {
        nodes: Vec::new(),
        edges: Vec::new(),
    };
    let layout = layout::structure_layout(&g);
    assert!(layout.nodes.is_empty());
    assert_eq!(layout, layout::structure_layout(&g));
}

#[test]
fn spacing_default_is_pure_and_cloneable() {
    let s = LayoutSpacing::default();
    let c = s;
    assert_eq!(s, c);
    assert!(s.h_gap > 0.0 && s.v_gap > 0.0 && s.margin >= 0.0);
}
