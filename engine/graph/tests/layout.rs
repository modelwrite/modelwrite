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

// ---------------------------------------------------------------------------
// Composition. The canvas follows the BOXES, not the rank structure: a rank that holds many
// boxes tightens the spacing between them instead of stretching the drawing to the height of the
// rank. Composition may improve; legibility may not regress.
// ---------------------------------------------------------------------------

/// One star: a container and the given number of children, every child in the container's single,
/// deep rank. This is the shape a filtered whole so often has - one hub and the rank around it.
fn star(parts: usize) -> Graph {
    let mut nodes = vec![GraphNode {
        id: "root".to_string(),
        kind: "block".to_string(),
        name: "Root".to_string(),
        stereotypes: Vec::new(),
    }];
    let mut edges = Vec::new();
    for i in 0..parts {
        let id = format!("part-{i:02}");
        nodes.push(GraphNode {
            id: id.clone(),
            kind: "block".to_string(),
            name: "Part".to_string(),
            stereotypes: Vec::new(),
        });
        edges.push(GraphEdge {
            source: "root".to_string(),
            target: id,
            kind: "part".to_string(),
            label: String::new(),
        });
    }
    Graph { nodes, edges }
}

/// One chain of the given length, each box in a rank of its own.
fn chain(count: usize) -> Graph {
    let nodes = (0..count)
        .map(|i| GraphNode {
            id: format!("n{i}"),
            kind: "block".to_string(),
            name: "Node".to_string(),
            stereotypes: Vec::new(),
        })
        .collect();
    let edges = (1..count)
        .map(|i| GraphEdge {
            source: format!("n{}", i - 1),
            target: format!("n{i}"),
            kind: "part".to_string(),
            label: String::new(),
        })
        .collect();
    Graph { nodes, edges }
}

/// The size a label lands at when a canvas is fitted into one 1920x1080 slide.
fn slide_fit(width: f64, height: f64) -> f64 {
    (1920.0 / width).min(1080.0 / height)
}

/// The y of every box whose id starts with the prefix, ascending.
fn ys_of(layout: &DiagramLayout, prefix: &str) -> Vec<f64> {
    let mut ys: Vec<f64> = layout
        .nodes
        .iter()
        .filter(|n| n.id.starts_with(prefix))
        .map(|n| n.y)
        .collect();
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
    ys
}

#[test]
fn a_deep_rank_tightens_its_spacing_so_the_canvas_follows_the_boxes() {
    let g = star(11);
    let layout = layout::structure_layout(&g);
    let spacing = LayoutSpacing::default();

    // The same boxes laid out with the spacing held constant: one column of eleven children.
    let (root_w, _) = layout::node_size("Root");
    let (part_w, part_h) = layout::node_size("Part");
    let strip_w = spacing.margin * 2.0 + root_w + spacing.h_gap + part_w;
    let strip_h = spacing.margin * 2.0 + 11.0 * part_h + 10.0 * spacing.v_gap;

    // The columns do not move: only the gap between stacked boxes follows the content.
    assert!(
        (layout.width - strip_w).abs() < 1e-9,
        "the horizontal layout must be untouched: {:.1} vs {:.1}",
        layout.width,
        strip_w
    );
    assert!(
        layout.height < strip_h,
        "eleven stacked boxes must tighten their spacing: {:.0} tall, not {:.0}",
        layout.height,
        strip_h
    );
    // ... and never past the floor: two boxes always stay visibly apart.
    let ys = ys_of(&layout, "part-");
    let gap = ys[1] - ys[0] - part_h;
    assert!(
        (gap - 12.0).abs() < 1e-9,
        "the gap must tighten to the tightest routable channel, got {gap:.2}"
    );
    // COMPOSITION IMPROVING WITHOUT legibility regressing: fitted into one slide the same boxes
    // read at least as large as they did with the constant spacing.
    assert!(
        slide_fit(layout.width, layout.height) > slide_fit(strip_w, strip_h),
        "tightening the canvas must not shrink a label at slide scale"
    );
    // Nothing is dropped and nothing overlaps (a closed gap would overlap the boxes).
    assert_eq!(layout.nodes.len(), 12, "every box is still placed");
    for i in 0..layout.nodes.len() {
        for j in (i + 1)..layout.nodes.len() {
            let (a, b) = (&layout.nodes[i], &layout.nodes[j]);
            let overlap = a.x < b.x + b.width
                && b.x < a.x + a.width
                && a.y < b.y + b.height
                && b.y < a.y + a.height;
            assert!(!overlap, "boxes {} and {} overlap", a.id, b.id);
        }
    }
}

#[test]
fn a_drawing_that_already_composes_keeps_the_spacing_it_was_given() {
    // Eight ranks of one box each: the drawing is wide and shallow, so it already has the shape of
    // the frame it is consumed at and nothing about it may change.
    let g = chain(8);
    let layout = layout::structure_layout(&g);
    let spacing = LayoutSpacing::default();
    let expected_h = spacing.margin * 2.0 + layout::NODE_H;
    assert!(
        (layout.height - expected_h).abs() < 1e-9,
        "a landscape drawing keeps its height: {:.1} vs {:.1}",
        layout.height,
        expected_h
    );
    let mut xs: Vec<f64> = layout.nodes.iter().map(|n| n.x).collect();
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    for (i, x) in xs.iter().enumerate() {
        let expected = spacing.margin + i as f64 * (layout::NODE_H + spacing.v_gap + spacing.h_gap);
        assert!(
            (x - expected).abs() < 1e-9,
            "a landscape drawing keeps its columns: {x:.1} vs {expected:.1}"
        );
    }
}

#[test]
fn the_corpus_canvas_follows_the_boxes_not_the_deepest_rank() {
    let g = expected().graph.clone().expect("corpus graph");
    let layout = layout::structure_layout(&g);
    let spacing = LayoutSpacing::default();
    // The corpus's deepest rank holds thirty boxes. With the spacing held constant that rank alone
    // is 1,904 units tall and makes the whole drawing 2,114 tall for a drawing 2,130 wide - a
    // near-square canvas that fits a 16:9 slide only by leaving half of it empty.
    let strip_h = spacing.margin * 2.0 + 30.0 * layout::NODE_H + 29.0 * spacing.v_gap;
    assert!(
        layout.height < strip_h,
        "the whole model must follow its boxes: {:.0} tall, not {:.0}",
        layout.height,
        strip_h
    );
    assert!(
        layout.height > spacing.margin * 2.0 + 30.0 * layout::NODE_H,
        "the boxes still need their own height and a gap between them"
    );
    assert!(
        slide_fit(layout.width, layout.height) > slide_fit(layout.width, strip_h),
        "the whole model must read larger on a slide than it did, measured {:.2} px",
        13.0 * slide_fit(layout.width, layout.height)
    );
}

#[test]
fn the_vertical_gap_never_closes_completely() {
    // Sixty children in one rank: the frame cannot be filled, so the gap goes to its floor and
    // stops there rather than letting the boxes touch.
    let g = star(60);
    let layout = layout::structure_layout(&g);
    let spacing = LayoutSpacing::default();
    let (_, part_h) = layout::node_size("Part");
    let ys = ys_of(&layout, "part-");
    assert_eq!(ys.len(), 60);
    for pair in ys.windows(2) {
        let gap = pair[1] - pair[0] - part_h;
        assert!(
            (gap - 12.0).abs() < 1e-9,
            "the floor keeps the boxes and the routes apart, got a gap of {gap:.2}"
        );
    }
    // The floor is a property of the layout, not of this drawing: a caller that asks for less than
    // the routable channel keeps its own smaller gap.
    assert!(spacing.v_gap >= 12.0, "the default spacing is routable");
}
