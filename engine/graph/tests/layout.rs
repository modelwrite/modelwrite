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

/// Every distinct column (x offset) a layout placed a box in, ascending.
fn columns_of(layout: &DiagramLayout) -> Vec<f64> {
    let mut xs: Vec<f64> = layout.nodes.iter().map(|n| n.x).collect();
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    xs.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    xs
}

/// The area the boxes themselves occupy, which is what a canvas can be full of.
fn box_area(layout: &DiagramLayout) -> f64 {
    layout.nodes.iter().map(|n| n.width * n.height).sum()
}

/// Every pair of boxes is disjoint.
fn assert_no_overlap(layout: &DiagramLayout) {
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
fn a_deep_rank_is_cut_into_columns_so_the_canvas_composes() {
    let g = star(11);
    let layout = layout::structure_layout(&g);
    let spacing = LayoutSpacing::default();

    // What the same boxes are with every rank held in one column: a strip twelve boxes tall.
    let (root_w, _) = layout::node_size("Root");
    let (part_w, part_h) = layout::node_size("Part");
    let strip_w = spacing.margin * 2.0 + root_w + spacing.h_gap + part_w;
    let strip_h = spacing.margin * 2.0 + 11.0 * part_h + 10.0 * spacing.v_gap;

    // COMPOSITION: a landscape picture rather than a portrait strip, with the boxes taking up more
    // of the canvas, and reading larger when the canvas is fitted onto a slide.
    assert!(
        layout.width > layout.height,
        "a star must compose as a landscape picture, got {:.0}x{:.0}",
        layout.width,
        layout.height
    );
    assert!(
        box_area(&layout) / (layout.width * layout.height)
            > box_area(&layout) / (strip_w * strip_h),
        "the boxes must take up more of the compacted canvas than of the strip"
    );
    assert!(
        slide_fit(layout.width, layout.height) > slide_fit(strip_w, strip_h),
        "compaction must not shrink a label at slide scale: {:.2} px against {:.2} px",
        13.0 * slide_fit(layout.width, layout.height),
        13.0 * slide_fit(strip_w, strip_h)
    );
    // The wide rank really was cut, and its boxes stay right of the container they belong to.
    assert!(
        columns_of(&layout).len() > 2,
        "eleven boxes in one rank must be placed as several columns"
    );
    let root = layout.box_for("root").expect("the container is placed");
    for part in layout.nodes.iter().filter(|n| n.id.starts_with("part-")) {
        assert!(root.x < part.x, "the container stays left of its parts");
    }
    assert_eq!(layout.nodes.len(), 12, "every box is still placed");
    assert_no_overlap(&layout);
}

#[test]
fn a_slide_picture_is_free_to_improve() {
    // A drawing that already reads on a slide is not held back by the survey rule: the compaction
    // pulls it into the frame's shape as far as it can.
    let g = star(8);
    let layout = layout::structure_layout(&g);
    let spacing = LayoutSpacing::default();
    let (root_w, _) = layout::node_size("Root");
    let (part_w, part_h) = layout::node_size("Part");
    let strip_w = spacing.margin * 2.0 + root_w + spacing.h_gap + part_w;
    let strip_h = spacing.margin * 2.0 + 8.0 * part_h + 7.0 * spacing.v_gap;
    assert!(
        13.0 * slide_fit(strip_w, strip_h) >= 11.0,
        "this drawing is a slide picture before anything is done to it"
    );
    assert!(
        slide_fit(layout.width, layout.height) > slide_fit(strip_w, strip_h),
        "a slide picture may be made to read larger"
    );
    assert_eq!(layout.nodes.len(), 9);
    assert_no_overlap(&layout);
}

#[test]
fn a_survey_stays_a_survey_and_composes_better() {
    let g = expected().graph.clone().expect("corpus graph");
    let layout = layout::structure_layout(&g);
    let spacing = LayoutSpacing::default();
    // Without compaction the corpus's deepest rank (thirty boxes) alone is 1,904 units tall, which
    // makes the drawing 2,114 tall for a drawing 2,130 wide: a near-square canvas that fits a
    // 16:9 slide only by leaving half of it empty.
    let strip_h = spacing.margin * 2.0 + 30.0 * layout::NODE_H + 29.0 * spacing.v_gap;
    assert!(
        layout.width > layout.height,
        "the whole model must compose as a landscape picture, got {:.0}x{:.0}",
        layout.width,
        layout.height
    );
    assert!(
        layout.height < strip_h,
        "the whole model must compose inside the frame, not to the height of its deepest rank: \
         {:.0} tall against {:.0}",
        layout.height,
        strip_h
    );
    // It is still a SURVEY: a whole-model view is not a slide picture, and the product says so on
    // the export's face rather than pretending otherwise. Compaction may not quietly promote it.
    let stated = 13.0 * slide_fit(layout.width, layout.height);
    assert!(
        stated < 11.0,
        "a survey must stay a survey: the whole model now states {stated:.2} px"
    );
    assert_eq!(layout.nodes.len(), 99, "every node is still placed");
    assert_no_overlap(&layout);
}

#[test]
fn stacked_boxes_never_come_closer_than_the_channel_a_route_turns_in() {
    // The gap between two boxes in one column is the channel an orthogonal route turns in between
    // them. The router's stub and lane clearance is 12 units, and a narrower gap leaves routes
    // crossing boxes - measured on the corpus, a gap of 11 leaves 4 crossing edges and 12 leaves
    // none - so the layout never closes it further.
    let corpus = expected().graph.clone().expect("corpus graph");
    for g in [star(60), corpus] {
        let layout = layout::structure_layout(&g);
        for column in columns_of(&layout) {
            let mut boxes: Vec<&layout::NodeBox> =
                layout.nodes.iter().filter(|n| n.x == column).collect();
            boxes.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap());
            for pair in boxes.windows(2) {
                let gap = pair[1].y - (pair[0].y + pair[0].height);
                assert!(
                    gap >= 12.0 - 1e-9,
                    "boxes {} and {} are {gap:.2} apart in their column",
                    pair[0].id,
                    pair[1].id
                );
            }
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
