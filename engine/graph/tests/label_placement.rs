// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tests for edge-label placement on the real corpus.
//!
//! The controller found the crowding by LOOKING at an exported drawing: near the containment
//! boundary the names converged, struck through each other, and disappeared under boxes. The old
//! rule put every label at the midpoint of the chord between its two anchors, which for the
//! coffee machine's fan-out through one shared trunk IS the trunk line: eleven names on one
//! vertical line. Measured on that export before the fix: 12 of 12 labels overlapped a node box,
//! 1 label overlapped another label, and 5 labels were struck through by an edge line that was not
//! their own. These tests hold the repair: no label box may overlap another label, a node box, or
//! any drawn line - the containment border is a drawn line, and it was what cut the names in half.

use std::collections::HashMap;

use graph::layout::{self, edge_label_size, DiagramLayout, NodeBox};
use graph::routing::{self, EdgeRequest, LabelPlacement, Lane};
use okf::types::{Graph, GraphEdge, OkfRoot};

fn corpus_graph() -> Graph {
    let root: OkfRoot =
        serde_json::from_str(&test_support::load_okf_expected()).expect("corpus parses");
    root.graph.expect("corpus graph")
}

fn lane_for(kind: &str) -> Lane {
    match kind {
        "include" | "triggers" | "transition" => Lane::Flow,
        "dependency" => Lane::Dependency,
        _ => Lane::Other,
    }
}

/// The drawn line of every labelled edge, in the renderer's own draw order, plus the label text.
/// This mirrors the renderer: the same sort, the same request set, the same routes - so a label
/// placed here is the label that paints.
fn lines_and_labels(graph: &Graph, layout: &DiagramLayout) -> (Vec<Vec<(f64, f64)>>, Vec<String>) {
    let box_by_id: HashMap<&str, &NodeBox> =
        layout.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut edges: Vec<&GraphEdge> = graph.edges.iter().collect();
    edges.sort_by(|a, b| {
        (&a.source, &a.target, &a.kind, &a.label).cmp(&(&b.source, &b.target, &b.kind, &b.label))
    });

    let mut requests: Vec<EdgeRequest> = Vec::new();
    let mut route_for: HashMap<usize, usize> = HashMap::new();
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
        route_for.insert(pos, requests.len() - 1);
    }
    let routed = routing::route_graph(&layout.nodes, &requests);

    // EVERY drawn line, exactly as the renderer hands them over: an edge with no name is still a
    // line on the drawing, and a label dropped across it is struck through just as badly.
    let mut lines = Vec::new();
    let mut labels = Vec::new();
    for (pos, e) in edges.iter().enumerate() {
        let line = match route_for.get(&pos) {
            Some(&ri) => routed[ri].points.clone(),
            None => {
                // An endpoint the layout did not place: the renderer draws it into the dangling
                // band, so the straight run between the two centres stands in for it here.
                let (Some(s), Some(t)) = (
                    box_by_id.get(e.source.as_str()),
                    box_by_id.get(e.target.as_str()),
                ) else {
                    continue;
                };
                vec![(s.center_x(), s.center_y()), (t.center_x(), t.center_y())]
            }
        };
        if line.len() < 2 {
            continue;
        }
        lines.push(line);
        labels.push(e.label.clone());
    }
    (lines, labels)
}

#[derive(Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    fn of(p: &LabelPlacement) -> Rect {
        Rect {
            x: p.left(),
            y: p.top(),
            w: p.width,
            h: p.height,
        }
    }
    fn of_box(b: &NodeBox) -> Rect {
        Rect {
            x: b.x,
            y: b.y,
            w: b.width,
            h: b.height,
        }
    }
    fn area_of_overlap(&self, o: &Rect) -> f64 {
        let w = (self.x + self.w).min(o.x + o.w) - self.x.max(o.x);
        let h = (self.y + self.h).min(o.y + o.h) - self.y.max(o.y);
        if w > 0.0 && h > 0.0 {
            w * h
        } else {
            0.0
        }
    }
    fn close_to(&self, o: &Rect, gap: f64) -> bool {
        self.x - gap < o.x + o.w
            && o.x < self.x + self.w + gap
            && self.y - gap < o.y + o.h
            && o.y < self.y + self.h + gap
    }
    fn segment_hits(&self, a: (f64, f64), b: (f64, f64)) -> bool {
        let (x0, x1) = if a.0 <= b.0 { (a.0, b.0) } else { (b.0, a.0) };
        let (y0, y1) = if a.1 <= b.1 { (a.1, b.1) } else { (b.1, a.1) };
        x0 < self.x + self.w && self.x < x1 && y0 < self.y + self.h && self.y < y1
    }
}

/// Everything the assertions need about one placement of the corpus. Computed once, so the four
/// tests below all read the same placement rather than each paying for its own.
struct Placed {
    lines: Vec<Vec<(f64, f64)>>,
    labels: Vec<String>,
    placements: Vec<Option<LabelPlacement>>,
    layout: DiagramLayout,
}

impl Placed {
    /// The line index, name and placement of every relationship that carries a name. The lines of
    /// the unnamed edges are in `lines` as obstacles but are never placed, so they are not here.
    fn named(&self) -> Vec<(usize, &str, &LabelPlacement)> {
        (0..self.labels.len())
            .filter(|&i| !self.labels[i].is_empty())
            .map(|i| {
                (
                    i,
                    self.labels[i].as_str(),
                    self.placements[i]
                        .as_ref()
                        .expect("every named relationship is placed"),
                )
            })
            .collect()
    }
}

fn placed() -> &'static Placed {
    static ONCE: std::sync::OnceLock<Placed> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        let graph = corpus_graph();
        let layout = layout::structure_layout(&graph);
        let (lines, labels) = lines_and_labels(&graph, &layout);
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let placements =
            routing::place_labels(&layout.nodes, &lines, &refs, (layout.width, layout.height));
        Placed {
            lines,
            labels,
            placements,
            layout,
        }
    })
}
/// The honest count this whole tranche is about: how many edge labels overlap another label or a
/// box, or are struck through by a drawn line. On the export the controller looked at, before the
/// fix, that was 12 box overlaps, 1 label overlap and 5 struck-through labels, out of 12 labels.
/// After it, all three figures are zero - and this test fails the moment any of them comes back.
#[test]
fn no_edge_label_overlaps_another_label_a_box_or_a_line() {
    let p = placed();
    let lines = &p.lines;
    let layout = &p.layout;
    // The relationships that carry a name: the unnamed edges are obstacles, not labels.
    let named = p.named();
    let rects: Vec<Rect> = named
        .iter()
        .map(|(_, _, placement)| Rect::of(placement))
        .collect();

    assert_eq!(
        named.len(),
        p.placements.iter().filter(|p| p.is_some()).count(),
        "every relationship name must be placed somewhere readable"
    );

    let mut label_overlaps = Vec::new();
    for i in 0..rects.len() {
        for j in (i + 1)..rects.len() {
            let area = rects[i].area_of_overlap(&rects[j]);
            if area > 0.0 {
                label_overlaps.push(format!(
                    "'{}' over '{}' by {:.1} square units",
                    named[i].1, named[j].1, area
                ));
            }
        }
    }
    assert!(
        label_overlaps.is_empty(),
        "{} edge label(s) overlap another label: {:?}",
        label_overlaps.len(),
        label_overlaps
    );

    let mut box_overlaps = Vec::new();
    for (_, label, placement) in &named {
        let rect = Rect::of(placement);
        for b in &layout.nodes {
            let area = rect.area_of_overlap(&Rect::of_box(b));
            if area > 0.0 {
                box_overlaps.push(format!("'{label}' over box '{}' by {area:.1}", b.id));
            }
        }
    }
    assert!(
        box_overlaps.is_empty(),
        "{} edge label(s) are drawn over a node box: {:?}",
        box_overlaps.len(),
        box_overlaps
    );

    // A label struck through by a drawn line - ANY drawn line, its own excepted, and that includes
    // the lines of the edges that carry no name. The containment border the controller saw cutting
    // the names in half is made of these lines.
    let mut struck = Vec::new();
    for (own, label, placement) in &named {
        let rect = Rect::of(placement);
        for (other, line) in lines.iter().enumerate() {
            if other == *own {
                continue;
            }
            if line.windows(2).any(|w| rect.segment_hits(w[0], w[1])) {
                struck.push(format!("'{label}' is crossed by the line of edge {other}"));
                break;
            }
        }
    }
    assert!(
        struck.is_empty(),
        "{} edge label(s) have an edge line struck through them: {:?}",
        struck.len(),
        struck
    );

    // The last-resort leader - one that has to pass behind a box to reach its line - is measured
    // rather than assumed away: if the count grows, the drawing has become denser than the
    // placement can carry and this says so instead of quietly getting worse.
    let box_rects: Vec<Rect> = layout.nodes.iter().map(Rect::of_box).collect();
    let behind_a_box = named
        .iter()
        .filter(|(_, _, placement)| {
            placement.leader.len() >= 2
                && placement
                    .leader
                    .windows(2)
                    .any(|w| box_rects.iter().any(|b| b.segment_hits(w[0], w[1])))
        })
        .count();
    assert!(
        behind_a_box <= 2,
        "measured {behind_a_box} leader(s) passing behind a node box"
    );

    for (_, label, p) in p.named() {
        assert!(
            p.left() >= -1e-9
                && p.top() >= -1e-9
                && p.right() <= layout.width + 1e-9
                && p.bottom() <= layout.height + 1e-9,
            "'{label}' is placed outside the drawing at ({}, {})",
            p.x,
            p.y
        );
    }
}
/// The distance from a point to a polyline, and the segment that distance lands on: the shortest
/// distance to any of its segments.
fn nearest_segment(p: (f64, f64), line: &[(f64, f64)]) -> (f64, (f64, f64), (f64, f64)) {
    let mut best = (f64::INFINITY, (0.0, 0.0), (0.0, 0.0));
    for w in line.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len2 = dx * dx + dy * dy;
        let t = if len2 <= 1e-12 {
            0.0
        } else {
            (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
        };
        let q = (a.0 + dx * t, a.1 + dy * t);
        let d = ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt();
        if d < best.0 - 1e-9 {
            best = (d, a, b);
        }
    }
    best
}
/// A label placed directly against its line must not carry a leader; a label that had to leave its
/// line must carry one, and it must run from the label's own box to the line it names.
#[test]
fn a_label_that_leaves_its_line_carries_a_leader_to_it() {
    // The standoff the placement keeps: half the label's height plus the clearance from the line.
    const STANDOFF: f64 = 3.0;
    let p = placed();
    let lines = &p.lines;
    let mut led = 0;
    for (own, label, placement) in p.named() {
        let line = &lines[own];
        let p = placement;
        let centre = (p.x, p.y);
        let (distance, _, _) = nearest_segment(centre, line);
        // A label placed beside its line stands off it by half the label in the perpendicular
        // direction, so the widest standoff the placement ever uses is half the label's longer
        // side plus the clearance. Anything further than that had to leave the line.
        let standoff = p.width.max(p.height) / 2.0 + STANDOFF;
        if p.leader.is_empty() {
            assert!(
                distance <= standoff + 1.0,
                "'{label}' carries no leader but sits {distance:.1} units from its own line"
            );
            continue;
        }
        if distance <= standoff + 1.0 {
            continue; // near enough to its line that a leader is tidiness, not necessity
        }
        led += 1;
        assert!(
            p.leader.len() >= 2,
            "'{label}' is {distance:.1} units from its line at ({}, {}) and must be led to it",
            p.x,
            p.y
        );
        let first = p.leader[0];
        let last = p.leader[p.leader.len() - 1];
        let on_box = (first.0 - p.left()).abs() < 1e-6
            || (first.0 - p.right()).abs() < 1e-6
            || (first.1 - p.top()).abs() < 1e-6
            || (first.1 - p.bottom()).abs() < 1e-6;
        assert!(on_box, "the leader for '{label}' starts off its own box");
        let tip = Rect {
            x: last.0 - 1e-6,
            y: last.1 - 1e-6,
            w: 2e-6,
            h: 2e-6,
        };
        assert!(
            line.windows(2).any(|w| tip.segment_hits(w[0], w[1])),
            "the leader for '{label}' does not land on its line"
        );
        // A leader is a straight run of right angles, and it never crosses a label's own text: it
        // leaves the box, so no point after the first is inside it.
        for w in p.leader.windows(2) {
            assert!(
                (w[0].0 - w[1].0).abs() < 1e-6 || (w[0].1 - w[1].1).abs() < 1e-6,
                "the leader for '{label}' is not orthogonal"
            );
        }
        assert!(
            !p.leader.iter().skip(1).any(|q| Rect::of(p).close_to(
                &Rect {
                    x: q.0,
                    y: q.1,
                    w: 0.0,
                    h: 0.0
                },
                0.0
            )),
            "the leader for '{label}' runs back through its own label"
        );
    }
    assert!(
        led > 0,
        "the corpus is dense enough to need at least one leader"
    );
}
/// The placement is a pure function of the geometry: the same model always places identically, so
/// the export stays byte-identical.
#[test]
fn edge_label_placement_is_deterministic() {
    let p = placed();
    let graph = corpus_graph();
    let layout = layout::structure_layout(&graph);
    let (lines, labels) = lines_and_labels(&graph, &layout);
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let again = routing::place_labels(&layout.nodes, &lines, &refs, (layout.width, layout.height));
    assert_eq!(
        again, p.placements,
        "the same model must place its labels identically"
    );
}
/// The size model is the one the collision test trusts, so it must never claim a box is narrower
/// than the browser paints. Measured in Chromium at 11 px system-ui over the corpus' own labels:
/// 4.30-6.75 units per character, and an ink height of 14.06.
#[test]
fn the_label_size_model_is_never_narrower_than_the_browser() {
    for (text, per_char) in [
        ("p2", 6.75),
        ("Refine", 6.43),
        ("Satisfy", 6.43),
        ("liquid waste", 5.68),
        ("electrical System", 5.68),
        ("Coffee beans in", 5.68),
        ("coffee Machine Production", 5.68),
    ] {
        let (w, h) = edge_label_size(text);
        let painted = text.chars().count() as f64 * per_char;
        assert!(
            w >= painted,
            "'{text}' is estimated at {w} units but paints at {painted}"
        );
        assert!(h >= 14.06, "'{text}' is estimated at {h} units tall");
    }
}

/// The specific crowding that started this: one box fanning out through a shared trunk. Eleven
/// labels used to land on the trunk line, four of them on top of each other.
#[test]
fn a_fan_out_through_one_trunk_does_not_stack_its_labels() {
    let p = placed();
    let mut seen: Vec<(f64, f64)> = Vec::new();
    for (_, label, p) in p.named() {
        let key = (p.x, p.y);
        assert!(
            !seen.contains(&key),
            "two labels share one position: '{label}' at ({}, {})",
            p.x,
            p.y
        );
        seen.push(key);
    }
}
