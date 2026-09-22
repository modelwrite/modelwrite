// SPDX-License-Identifier: AGPL-3.0-or-later
//! Deterministic orthogonal (Manhattan) edge routing.
//!
//! An engineering diagram never lets an arrow wander through a box. Edges leave a node at the
//! centre of a side (left/right/top/bottom), travel in right angles, and arrive at the centre of
//! the facing side. The turning segments are placed in the free channels between node boxes so a
//! route never runs through another node; where the direct route is blocked, the edge detours
//! around the obstacle. Every choice here is a pure function of the boxes, so the same model
//! routes byte-identically.

use std::collections::HashMap;

use crate::layout::{edge_label_size, NodeBox};

/// A point in canvas units. Named so the placement signatures - which nest points two and three
/// deep in slices, segments and result tuples - stay readable rather than becoming a wall of
/// parentheses.
pub type Point = (f64, f64);
/// One straight run of a drawn line, from `0` to `1`.
pub type Seg = (Point, Point);

/// The side of a node an edge leaves from or arrives at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

/// The smallest gap that counts as a routable channel between boxes.
const MIN_CHANNEL: f64 = 8.0;
/// The clearance a detour lane keeps from the obstacle it goes around.
const LANE_GAP: f64 = 12.0;
/// The length of the perpendicular stub that lands an arrowhead on a top/bottom anchor.
const STUB: f64 = 12.0;
/// The tolerance that counts a segment as touching (rather than crossing) a box.
const EPS: f64 = 0.5;

/// The two side anchors for an edge. In a columnar (left-to-right) layout an edge between two
/// columns leaves/arrives horizontally; only an edge within one column (the boxes' x-ranges
/// overlap) leaves/arrives vertically. The sign of the difference picks the direction, so the
/// choice is deterministic.
pub fn sides(source: &NodeBox, target: &NodeBox) -> (Side, Side) {
    let dx = target.center_x() - source.center_x();
    let dy = target.center_y() - source.center_y();
    let same_column = source.x < target.x + target.width && target.x < source.x + source.width;
    if same_column {
        if dy >= 0.0 {
            (Side::Bottom, Side::Top)
        } else {
            (Side::Top, Side::Bottom)
        }
    } else if dx >= 0.0 {
        (Side::Right, Side::Left)
    } else {
        (Side::Left, Side::Right)
    }
}

/// The centre of a side: the defined anchor an edge leaves from or lands on.
pub fn anchor(b: &NodeBox, side: Side) -> (f64, f64) {
    match side {
        Side::Left => (b.x, b.center_y()),
        Side::Right => (b.x + b.width, b.center_y()),
        Side::Top => (b.center_x(), b.y),
        Side::Bottom => (b.center_x(), b.y + b.height),
    }
}

/// Route an edge from `source` to `target` as an orthogonal polyline whose first point is the
/// source side anchor and whose last point is the target side anchor. `all_boxes` is every
/// placed box; source and target are excluded from the obstacle set automatically.
pub fn route(source: &NodeBox, target: &NodeBox, all_boxes: &[&NodeBox]) -> Vec<(f64, f64)> {
    route_offset(source, target, all_boxes, 0.0)
}

/// As [route], but spreads the anchors `offset` units along the side so parallel edges between the
/// same two boxes do not overlap.
pub fn route_offset(
    source: &NodeBox,
    target: &NodeBox,
    all_boxes: &[&NodeBox],
    offset: f64,
) -> Vec<(f64, f64)> {
    let obstacles: Vec<&NodeBox> = all_boxes
        .iter()
        .copied()
        .filter(|b| b.id != source.id && b.id != target.id)
        .collect();
    let (ss, ts) = sides(source, target);
    let (sx, sy) = spread(anchor(source, ss), ss, offset);
    let (tx, ty) = spread(anchor(target, ts), ts, offset);
    match (ss, ts) {
        (Side::Right, Side::Left) | (Side::Left, Side::Right) => {
            horizontal_route(sx, sy, tx, ty, &obstacles)
        }
        (Side::Bottom, Side::Top) | (Side::Top, Side::Bottom) => {
            vertical_route(sx, sy, tx, ty, source, target, &obstacles)
        }
        // sides() always returns facing sides; this arm is unreachable but keeps the match total.
        _ => vec![(sx, sy), (tx, ty)],
    }
}

/// Shift an anchor along its side, perpendicular to the direction of travel.
fn spread(p: (f64, f64), side: Side, offset: f64) -> (f64, f64) {
    match side {
        Side::Left | Side::Right => (p.0, p.1 + offset),
        Side::Top | Side::Bottom => (p.0 + offset, p.1),
    }
}

/// A horizontal-dominant route: leave a left/right anchor, turn in a free channel, arrive at the
/// facing left/right anchor.
fn horizontal_route(sx: f64, sy: f64, tx: f64, ty: f64, obstacles: &[&NodeBox]) -> Vec<(f64, f64)> {
    let direct = if (sy - ty).abs() < 0.5 {
        vec![(sx, sy), (tx, ty)]
    } else {
        let (lo, hi) = order(sx, tx);
        let (y_lo, y_hi) = order(sy, ty);
        let x = channel_x(lo, hi, y_lo, y_hi, obstacles);
        vec![(sx, sy), (x, sy), (x, ty), (tx, ty)]
    };
    if path_clear(&direct, obstacles) {
        return direct;
    }
    detour_horizontal(sx, sy, tx, ty, obstacles)
}

/// A vertical-dominant route (same column): leave a top/bottom anchor, travel in a free channel
/// beside the column, arrive at the facing top/bottom anchor with a perpendicular stub.
fn vertical_route(
    sx: f64,
    sy: f64,
    tx: f64,
    ty: f64,
    source: &NodeBox,
    target: &NodeBox,
    obstacles: &[&NodeBox],
) -> Vec<(f64, f64)> {
    let dir = if ty >= sy { 1.0 } else { -1.0 };
    let stub_y = ty - dir * STUB;
    let direct = if (sx - tx).abs() < 0.5 {
        vec![(sx, sy), (tx, ty)]
    } else {
        let (x_lo, x_hi) = order(sx, tx);
        let (v_lo, v_hi) = order(sy, stub_y);
        let x = channel_x(x_lo, x_hi, v_lo, v_hi, obstacles);
        vec![(sx, sy), (x, sy), (x, stub_y), (tx, stub_y), (tx, ty)]
    };
    if path_clear(&direct, obstacles) {
        return direct;
    }
    detour_vertical(sx, sy, tx, ty, source, target, obstacles)
}

/// Detour a horizontal-dominant route around every box between source and target: run the
/// crossing leg in a clear lane above or below all of them, with vertical stubs on the node
/// edges. The lane sits outside the full vertical extent of the span, so it is always clear.
fn detour_horizontal(
    sx: f64,
    sy: f64,
    tx: f64,
    ty: f64,
    obstacles: &[&NodeBox],
) -> Vec<(f64, f64)> {
    let (lo, hi) = order(sx, tx);
    let in_span: Vec<&NodeBox> = obstacles
        .iter()
        .copied()
        .filter(|b| b.x < hi && b.x + b.width > lo)
        .collect();
    let top = in_span.iter().map(|b| b.y).fold(f64::INFINITY, f64::min);
    let bottom = in_span
        .iter()
        .map(|b| b.y + b.height)
        .fold(f64::NEG_INFINITY, f64::max);
    let mid = (sy + ty) / 2.0;
    let above = top - LANE_GAP;
    let below = bottom + LANE_GAP;
    let lanes = if (above - mid).abs() <= (below - mid).abs() {
        [above, below]
    } else {
        [below, above]
    };
    let mut fallback: Option<Vec<(f64, f64)>> = None;
    for lane in lanes {
        let (v_lo, v_hi) = order(sy, lane);
        let x = channel_x(lo, hi, v_lo, v_hi, obstacles);
        let pts = vec![(sx, sy), (x, sy), (x, lane), (tx, lane), (tx, ty)];
        if path_clear(&pts, obstacles) {
            return pts;
        }
        if fallback.is_none() {
            fallback = Some(pts);
        }
    }
    // Neither lane is reachable the way the detour starts: leave the row band first and try again.
    for lane in lanes {
        if let Some(pts) = escape_route(sx, sy, tx, ty, lane, obstacles) {
            return pts;
        }
    }
    fallback.unwrap_or_else(|| vec![(sx, sy), (tx, ty)])
}

/// Detour a vertical-dominant route around a same-column obstacle: jog out of the column to a
/// free channel on the nearer side, travel there, jog back and land with a perpendicular stub.
fn detour_vertical(
    sx: f64,
    sy: f64,
    tx: f64,
    ty: f64,
    source: &NodeBox,
    target: &NodeBox,
    obstacles: &[&NodeBox],
) -> Vec<(f64, f64)> {
    let dir = if ty >= sy { 1.0 } else { -1.0 };
    let stub_y = ty - dir * STUB;
    let col_left = source.x.min(target.x);
    let col_right = (source.x + source.width).max(target.x + target.width);
    let left_channel = col_left - LANE_GAP;
    let right_channel = col_right + LANE_GAP;
    let mid = (sx + tx) / 2.0;
    let channels = if (left_channel - mid).abs() <= (right_channel - mid).abs() {
        [left_channel, right_channel]
    } else {
        [right_channel, left_channel]
    };
    let mut fallback: Option<Vec<(f64, f64)>> = None;
    for x in channels {
        let pts = vec![(sx, sy), (x, sy), (x, stub_y), (tx, stub_y), (tx, ty)];
        if path_clear(&pts, obstacles) {
            return pts;
        }
        if fallback.is_none() {
            fallback = Some(pts);
        }
    }
    fallback.unwrap_or_else(|| vec![(sx, sy), (tx, ty)])
}

/// Whether every segment of a polyline avoids every obstacle.
fn path_clear(pts: &[(f64, f64)], obstacles: &[&NodeBox]) -> bool {
    pts.windows(2).all(|w| seg_clear(w[0], w[1], obstacles))
}

fn seg_clear(a: (f64, f64), b: (f64, f64), obstacles: &[&NodeBox]) -> bool {
    obstacles.iter().all(|o| !seg_hits(a, b, o))
}

/// Whether an axis-aligned segment passes through the interior of a box.
fn seg_hits(a: (f64, f64), b: (f64, f64), o: &NodeBox) -> bool {
    if (a.0 - b.0).abs() < 1e-6 {
        let seg_x = a.0;
        let (ylo, yhi) = order(a.1, b.1);
        seg_x > o.x + EPS
            && seg_x < o.x + o.width - EPS
            && yhi > o.y + EPS
            && ylo < o.y + o.height - EPS
    } else {
        let seg_y = a.1;
        let (xlo, xhi) = order(a.0, b.0);
        seg_y > o.y + EPS
            && seg_y < o.y + o.height - EPS
            && xhi > o.x + EPS
            && xlo < o.x + o.width - EPS
    }
}

fn order(a: f64, b: f64) -> (f64, f64) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// The x of the vertical turning segment, in the first free column channel of `[lo, hi]` whose
/// boxes do not overlap the y-span `[y_lo, y_hi]`.
fn channel_x(lo: f64, hi: f64, y_lo: f64, y_hi: f64, obstacles: &[&NodeBox]) -> f64 {
    let blockers: Vec<(f64, f64)> = obstacles
        .iter()
        .filter(|b| b.y < y_hi && b.y + b.height > y_lo)
        .filter(|b| b.x < hi && b.x + b.width > lo)
        .map(|b| (b.x, b.x + b.width))
        .collect();
    first_gap(lo, hi, &blockers)
}

/// Merge overlapping spans in place, ascending by their start.
fn merge_spans(spans: &mut Vec<(f64, f64)>) {
    spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for &(a, b) in spans.iter() {
        if let Some(last) = merged.last_mut() {
            if a <= last.1 {
                if b > last.1 {
                    last.1 = b;
                }
                continue;
            }
        }
        merged.push((a, b));
    }
    *spans = merged;
}

/// The centre of the first gap in `[lo, hi]` between the (sorted, merged) blocking intervals;
/// falls back to the midpoint when no gap is wide enough.
fn first_gap(lo: f64, hi: f64, intervals: &[(f64, f64)]) -> f64 {
    let mut merged = intervals.to_vec();
    merge_spans(&mut merged);
    let mut cursor = lo;
    for &(a, b) in &merged {
        if a - cursor > MIN_CHANNEL {
            return (cursor + a) / 2.0;
        }
        cursor = cursor.max(b);
    }
    if hi - cursor > MIN_CHANNEL {
        return (cursor + hi) / 2.0;
    }
    (lo + hi) / 2.0
}

/// The x-ranges of the boxes that overlap the rectangle `[lo, hi] x [y_lo, y_hi]`, merged.
fn blocked_spans(
    lo: f64,
    hi: f64,
    y_lo: f64,
    y_hi: f64,
    obstacles: &[&NodeBox],
) -> Vec<(f64, f64)> {
    let mut spans: Vec<(f64, f64)> = obstacles
        .iter()
        .filter(|b| b.y < y_hi && b.y + b.height > y_lo)
        .filter(|b| b.x < hi && b.x + b.width > lo)
        .map(|b| (b.x, b.x + b.width))
        .collect();
    merge_spans(&mut spans);
    spans
}

/// The centre of every free vertical channel in `[lo, hi]`: an x that no box overlapping the
/// y-span `[y_lo, y_hi]` covers, with at least [MIN_CHANNEL] of clearance. Ascending.
fn free_channels(lo: f64, hi: f64, y_lo: f64, y_hi: f64, obstacles: &[&NodeBox]) -> Vec<f64> {
    let merged = blocked_spans(lo, hi, y_lo, y_hi, obstacles);
    let mut channels = Vec::new();
    let mut cursor = lo;
    for &(a, b) in &merged {
        if a - cursor > MIN_CHANNEL {
            channels.push((cursor + a) / 2.0);
        }
        cursor = cursor.max(b);
    }
    if hi - cursor > MIN_CHANNEL {
        channels.push((cursor + hi) / 2.0);
    }
    channels
}

/// How many channels the escape route will try at each end. The nearest one is almost always the
/// answer; a handful keeps the search bounded on a wide drawing.
const ESCAPE_CHANNELS: usize = 4;

/// A route that leaves its starting row band instead of running through what is in it.
///
/// The lane detours start with a leg that runs straight out of the source's side at the anchor's
/// y, so a box in that row band blocks every one of them however clear the lanes are - which is
/// exactly what happens as soon as a rank is placed as more than one column, and the source's own
/// rank-mate sits beside it. This variant turns FIRST: out to the nearest free channel between the
/// source and the target, along the lane, down through the nearest free channel at the target's
/// end, and in to the anchor. Every segment is axis-aligned, the anchors are unchanged, and the
/// whole path is verified clear of every box before it is returned - so it can only ever replace a
/// route that would have crossed one.
fn escape_route(
    sx: f64,
    sy: f64,
    tx: f64,
    ty: f64,
    lane: f64,
    obstacles: &[&NodeBox],
) -> Option<Vec<(f64, f64)>> {
    let (lo, hi) = order(sx, tx);
    let (sy_lo, sy_hi) = order(sy, lane);
    let (ty_lo, ty_hi) = order(ty, lane);
    let mut out = free_channels(lo, hi, sy_lo, sy_hi, obstacles);
    let mut back = free_channels(lo, hi, ty_lo, ty_hi, obstacles);
    // Nearest the source first, then nearest the target, ties by position: deterministic.
    out.sort_by(|a, b| {
        (a - sx)
            .abs()
            .partial_cmp(&(b - sx).abs())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
    });
    back.sort_by(|a, b| {
        (a - tx)
            .abs()
            .partial_cmp(&(b - tx).abs())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
    });
    for &x0 in out.iter().take(ESCAPE_CHANNELS) {
        for &x1 in back.iter().take(ESCAPE_CHANNELS) {
            let pts = vec![
                (sx, sy),
                (x0, sy),
                (x0, lane),
                (x1, lane),
                (x1, ty),
                (tx, ty),
            ];
            if path_clear(&pts, obstacles) {
                return Some(pts);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Graph-level routing: deterministic anchors, shared fan-out trunks, separate
// lanes for non-flow edges, and crossing jumps.
// ---------------------------------------------------------------------------

/// The perpendicular spread between adjacent anchors that share one side of a node.
const ANCHOR_SPREAD: f64 = 10.0;
/// The clearance an anchor keeps from a side's corners.
const SIDE_MARGIN: f64 = 6.0;
/// The distance from a source side to the shared trunk line.
const TRUNK_GAP: f64 = 12.0;
/// The perpendicular separation between adjacent lanes.
const LANE_SPACING: f64 = 10.0;
/// The clearance a shared trunk must keep from a node box edge (larger than EPS so a trunk can
/// never graze a box that a later tolerance would call a crossing).
const TRUNK_CLEAR: f64 = 4.0;

/// Which visual lane an edge travels in. Flow edges take the primary trunk; dependency
/// (Satisfy/…) edges take their own band so they never read as "flows to"; everything else
/// (containment, association, generalization) shares the middle band.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Lane {
    Flow,
    Dependency,
    Other,
}

impl Lane {
    /// The band index (0 = nearest the source), used to offset the shared trunk per lane.
    pub fn rank(self) -> usize {
        match self {
            Lane::Flow => 0,
            Lane::Other => 1,
            Lane::Dependency => 2,
        }
    }

    /// Map a renderer edge group ("flow" / "dependency" / …) onto a lane.
    pub fn from_group(group: &str) -> Lane {
        match group {
            "flow" => Lane::Flow,
            "dependency" => Lane::Dependency,
            _ => Lane::Other,
        }
    }
}

/// One edge as the graph-level router sees it: endpoints plus the lane it travels in and a
/// unique, caller-assigned ordinal (the draw order) used as the final tie-break.
pub struct EdgeRequest {
    pub source: String,
    pub target: String,
    pub lane: Lane,
    pub index: usize,
}

/// A crossing jump: a small arc drawn on the edge at segment `seg` (the polyline segment it lies
/// on) so a crossing never reads as a junction.
#[derive(Debug, Clone, PartialEq)]
pub struct Jump {
    pub seg: usize,
    pub x: f64,
    pub y: f64,
}

/// The result of routing one edge: an orthogonal polyline and any crossing jumps on it.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutedEdge {
    pub points: Vec<(f64, f64)>,
    pub jumps: Vec<Jump>,
}

/// Anchor entries grouped by node side: (edge index, is-source-anchor, perpendicular sort key).
type AnchorGroup = Vec<(usize, bool, f64)>;
type AnchorGroups<'a> = HashMap<(&'a str, Side), AnchorGroup>;
/// Horizontal trunk groups keyed by (source id, side, target column).
type TrunkGroups<'a> = HashMap<(&'a str, Side, i64), Vec<usize>>;

/// Route every edge at once so anchors can be spread across a side, fan-out edges can share a
/// trunk, dependency edges get their own lane, and crossings can be marked. Pure function of the
/// node boxes and the (deterministically ordered) requests: the same graph routes identically.
///
/// `edges` are node-to-node and non-self-loop; the caller resolves dangling endpoints and
/// self-loops itself. Each request's `index` is the caller's draw order (ascending), which
/// decides which edge carries the jump at a crossing.
pub fn route_graph(nodes: &[NodeBox], edges: &[EdgeRequest]) -> Vec<RoutedEdge> {
    let n = edges.len();
    let by_id: HashMap<&str, &NodeBox> = nodes.iter().map(|b| (b.id.as_str(), b)).collect();

    // Per-edge facing sides (a placeholder for any endpoint the caller failed to place).
    let mut edge_sides: Vec<(Side, Side)> = vec![(Side::Right, Side::Left); n];
    for i in 0..n {
        let e = &edges[i];
        if let (Some(s), Some(t)) = (by_id.get(e.source.as_str()), by_id.get(e.target.as_str())) {
            edge_sides[i] = sides(s, t);
        }
    }

    let (src_off, tgt_off) = assign_anchors(edges, &edge_sides, &by_id);
    let trunks = compute_trunks(nodes, edges, &edge_sides, &src_off, &tgt_off, &by_id);

    let mut routes: Vec<Vec<(f64, f64)>> = vec![Vec::new(); n];
    for i in 0..n {
        let e = &edges[i];
        let (Some(s), Some(t)) = (by_id.get(e.source.as_str()), by_id.get(e.target.as_str()))
        else {
            continue;
        };
        let (ss, ts) = edge_sides[i];
        let (sx, sy) = spread(anchor(s, ss), ss, src_off[i]);
        let (tx, ty) = spread(anchor(t, ts), ts, tgt_off[i]);
        let obstacles: Vec<&NodeBox> = nodes
            .iter()
            .filter(|b| b.id != s.id && b.id != t.id)
            .collect();
        let pts = match (ss, ts) {
            (Side::Right, Side::Left) | (Side::Left, Side::Right) => {
                route_horizontal(sx, sy, tx, ty, trunks[i], &obstacles)
            }
            (Side::Bottom, Side::Top) | (Side::Top, Side::Bottom) => {
                vertical_route(sx, sy, tx, ty, s, t, &obstacles)
            }
            _ => vec![(sx, sy), (tx, ty)],
        };
        routes[i] = pts;
    }

    // Draw order for jump assignment: the caller's index is the draw position, so a
    // later-drawn edge (higher index) carries the jump at a crossing.
    let mut draw_order: Vec<usize> = (0..n).collect();
    draw_order.sort_by_key(|&i| edges[i].index);
    let jumps = find_jumps(&routes, &draw_order);
    (0..n)
        .map(|i| RoutedEdge {
            points: routes[i].clone(),
            jumps: jumps[i].clone(),
        })
        .collect()
}

/// The perpendicular coordinate along which edges sharing a side are ordered (y for the
/// left/right sides, x for the top/bottom sides), so parallel edges stay unshuffled.
fn perp_center(b: &NodeBox, side: Side) -> f64 {
    match side {
        Side::Left | Side::Right => b.center_y(),
        Side::Top | Side::Bottom => b.center_x(),
    }
}

/// Spread every edge touching a node side across that side, ordered by the far endpoint's
/// perpendicular position so fan-out and fan-in edges never cross. Returns the perpendicular
/// offset for each edge's source anchor and target anchor.
fn assign_anchors(
    edges: &[EdgeRequest],
    edge_sides: &[(Side, Side)],
    by_id: &HashMap<&str, &NodeBox>,
) -> (Vec<f64>, Vec<f64>) {
    let n = edges.len();
    let mut src_off = vec![0.0f64; n];
    let mut tgt_off = vec![0.0f64; n];
    let mut groups: AnchorGroups<'_> = HashMap::new();
    for i in 0..n {
        let e = &edges[i];
        let (Some(s), Some(t)) = (by_id.get(e.source.as_str()), by_id.get(e.target.as_str()))
        else {
            continue;
        };
        let (ss, ts) = edge_sides[i];
        groups
            .entry((s.id.as_str(), ss))
            .or_default()
            .push((i, true, perp_center(t, ss)));
        groups
            .entry((t.id.as_str(), ts))
            .or_default()
            .push((i, false, perp_center(s, ts)));
    }
    for ((node_id, side), mut entries) in groups {
        entries.sort_by(|a, b| {
            a.2.partial_cmp(&b.2)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        let Some(b) = by_id.get(node_id) else {
            continue;
        };
        let avail = match side {
            Side::Left | Side::Right => b.height,
            Side::Top | Side::Bottom => b.width,
        };
        let count = entries.len();
        let step = if count > 1 {
            ((avail - 2.0 * SIDE_MARGIN) / (count as f64 - 1.0)).min(ANCHOR_SPREAD)
        } else {
            0.0
        };
        for (rank, &(idx, is_src, _)) in entries.iter().enumerate() {
            let off = (rank as f64 - (count as f64 - 1.0) / 2.0) * step;
            if is_src {
                src_off[idx] = off;
            } else {
                tgt_off[idx] = off;
            }
        }
    }
    (src_off, tgt_off)
}

/// Compute a shared trunk x for every horizontal edge, grouped by (source, side, target column).
/// Each lane in a group gets its own trunk offset; a trunk is used only when its vertical line is
/// clear of every node box over the group's y-span (otherwise the edge falls back to a channel).
fn compute_trunks(
    nodes: &[NodeBox],
    edges: &[EdgeRequest],
    edge_sides: &[(Side, Side)],
    src_off: &[f64],
    tgt_off: &[f64],
    by_id: &HashMap<&str, &NodeBox>,
) -> Vec<Option<f64>> {
    let n = edges.len();
    let mut trunks: Vec<Option<f64>> = vec![None; n];
    let mut groups: TrunkGroups<'_> = HashMap::new();
    for i in 0..n {
        let e = &edges[i];
        let (Some(s), Some(t)) = (by_id.get(e.source.as_str()), by_id.get(e.target.as_str()))
        else {
            continue;
        };
        let (ss, _ts) = edge_sides[i];
        if !matches!(ss, Side::Right | Side::Left) {
            continue;
        }
        let col = (t.x * 1000.0).round() as i64;
        groups.entry((s.id.as_str(), ss, col)).or_default().push(i);
    }
    for ((node_id, ss, _col), mut idxs) in groups {
        idxs.sort_unstable();
        let Some(s) = by_id.get(node_id) else {
            continue;
        };
        let mut y_lo = f64::INFINITY;
        let mut y_hi = f64::NEG_INFINITY;
        for &i in &idxs {
            let e = &edges[i];
            let t = by_id.get(e.target.as_str()).expect("placed target");
            let ts = edge_sides[i].1;
            let (_sx, sy) = spread(anchor(s, ss), ss, src_off[i]);
            let (_tx, ty) = spread(anchor(t, ts), ts, tgt_off[i]);
            y_lo = y_lo.min(sy.min(ty));
            y_hi = y_hi.max(sy.max(ty));
        }
        let mut lanes: Vec<Lane> = idxs.iter().map(|&i| edges[i].lane).collect();
        lanes.sort_unstable();
        lanes.dedup();
        for lane in lanes {
            let base = match ss {
                Side::Right => s.x + s.width + TRUNK_GAP,
                Side::Left => s.x - TRUNK_GAP,
                _ => continue,
            };
            let x = match ss {
                Side::Right => base + lane.rank() as f64 * LANE_SPACING,
                Side::Left => base - lane.rank() as f64 * LANE_SPACING,
                _ => continue,
            };
            if x_clear(x, y_lo, y_hi, nodes) {
                for &i in &idxs {
                    if edges[i].lane == lane {
                        trunks[i] = Some(x);
                    }
                }
            }
        }
    }
    trunks
}

/// Whether the vertical line x = `x`, over the y-span [`y_lo`, `y_hi`], passes through no box.
fn x_clear(x: f64, y_lo: f64, y_hi: f64, boxes: &[NodeBox]) -> bool {
    boxes.iter().all(|b| {
        !(x > b.x - TRUNK_CLEAR
            && x < b.x + b.width + TRUNK_CLEAR
            && y_hi > b.y + EPS
            && y_lo < b.y + b.height - EPS)
    })
}

/// A horizontal route that uses a shared trunk x when one is available and clear, and otherwise
/// falls back to the single-edge channel router (which detours around obstacles).
fn route_horizontal(
    sx: f64,
    sy: f64,
    tx: f64,
    ty: f64,
    trunk: Option<f64>,
    obstacles: &[&NodeBox],
) -> Vec<(f64, f64)> {
    if let Some(x) = trunk {
        let pts = fixed_x_route(sx, sy, tx, ty, x);
        if path_clear(&pts, obstacles) {
            return pts;
        }
    }
    horizontal_route(sx, sy, tx, ty, obstacles)
}

/// A horizontal-dominant route with a fixed turning x (the shared trunk).
fn fixed_x_route(sx: f64, sy: f64, tx: f64, ty: f64, x: f64) -> Vec<(f64, f64)> {
    if (sy - ty).abs() < 0.5 {
        vec![(sx, sy), (tx, ty)]
    } else {
        vec![(sx, sy), (x, sy), (x, ty), (tx, ty)]
    }
}

/// Find every proper crossing between two edges and mark the later-drawn edge with a jump. The
/// `order` is the draw order (ascending index); the later edge carries the arc so the reader can
/// always tell a crossing from a junction.
fn find_jumps(routes: &[Vec<(f64, f64)>], order: &[usize]) -> Vec<Vec<Jump>> {
    let n = routes.len();
    let mut jumps: Vec<Vec<Jump>> = vec![Vec::new(); n];
    for a in 0..order.len() {
        let ia = order[a];
        if routes[ia].len() < 2 {
            continue;
        }
        for &ib in order.iter().skip(a + 1) {
            if routes[ib].len() < 2 {
                continue;
            }
            for sa in 0..routes[ia].len() - 1 {
                let seg_a = (routes[ia][sa], routes[ia][sa + 1]);
                for sb in 0..routes[ib].len() - 1 {
                    let seg_b = (routes[ib][sb], routes[ib][sb + 1]);
                    if let Some((px, py)) = seg_cross(seg_a.0, seg_a.1, seg_b.0, seg_b.1) {
                        jumps[ib].push(Jump {
                            seg: sb,
                            x: px,
                            y: py,
                        });
                    }
                }
            }
        }
    }
    for jlist in jumps.iter_mut() {
        jlist.sort_by(|x, y| {
            x.seg
                .cmp(&y.seg)
                .then_with(|| x.x.partial_cmp(&y.x).unwrap_or(std::cmp::Ordering::Equal))
                .then_with(|| x.y.partial_cmp(&y.y).unwrap_or(std::cmp::Ordering::Equal))
        });
        jlist.dedup_by(|x, y| {
            x.seg == y.seg && (x.x - y.x).abs() < 1e-6 && (x.y - y.y).abs() < 1e-6
        });
    }
    jumps
}

/// The proper crossing point of two axis-aligned segments, or None when they are parallel, touch
/// only at an endpoint, or overlap collinearly.
fn seg_cross(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> Option<(f64, f64)> {
    let a_horizontal = (a.1 - b.1).abs() < 1e-6;
    let c_horizontal = (c.1 - d.1).abs() < 1e-6;
    if a_horizontal == c_horizontal {
        return None;
    }
    if a_horizontal {
        let (xlo, xhi) = order(a.0, b.0);
        let (ylo, yhi) = order(c.1, d.1);
        let x = c.0;
        let y = a.1;
        if x > xlo + 1e-6 && x < xhi - 1e-6 && y > ylo + 1e-6 && y < yhi - 1e-6 {
            Some((x, y))
        } else {
            None
        }
    } else {
        let (xlo, xhi) = order(c.0, d.0);
        let (ylo, yhi) = order(a.1, b.1);
        let x = a.0;
        let y = c.1;
        if x > xlo + 1e-6 && x < xhi - 1e-6 && y > ylo + 1e-6 && y < yhi - 1e-6 {
            Some((x, y))
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Label placement: where the name of a relationship can actually be read.
// ---------------------------------------------------------------------------

/// The standoff a label keeps from the line it names, so no stroke strikes through the text.
const LABEL_LINE_GAP: f64 = 3.0;
/// The standoff two labels keep from each other.
const LABEL_LABEL_GAP: f64 = 2.0;
/// The standoff a label keeps from a node box.
const LABEL_BOX_GAP: f64 = 2.0;
/// The standoff a label keeps from the edge of the drawing.
const LABEL_EDGE_INSET: f64 = 2.0;
/// The clearance a leader keeps from a box or a label it passes.
const LEADER_BOX_GAP: f64 = 1.0;
/// How far along its own line the search walks, as a fraction of each segment. The middle of the
/// longest segment is tried first and the walk fans out symmetrically, so a label lands where its
/// line has the most room and the choice never depends on floating-point noise.
const LABEL_STATIONS: [f64; 9] = [0.5, 0.4, 0.6, 0.3, 0.7, 0.2, 0.8, 0.1, 0.9];
/// The step of the ring search that finds room for a label that cannot sit beside its own line,
/// and how many rings it will walk. Eight units is a fifth of a node box's height: fine enough to
/// find a pocket, coarse enough to stay cheap on a hundred-label drawing. A hundred and sixty
/// rings reach 1280 units in every direction; past that a pocket is so far from the line that the
/// geometry-aligned search below - which is bounded and rests labels against the boxes they fit
/// between - is the better answer anyway.
const LEADER_STEP: f64 = 16.0;
const LEADER_RINGS: usize = 80;

/// Where the renderer should draw an edge label, and how a reader gets from that label to the line
/// it names.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelPlacement {
    /// The centre of the label's box.
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// An orthogonal leader from the label to its line, drawn when the label could not sit beside
    /// the line itself. Empty when the label sits directly against it.
    pub leader: Vec<(f64, f64)>,
}

impl LabelPlacement {
    pub fn left(&self) -> f64 {
        self.x - self.width / 2.0
    }
    pub fn right(&self) -> f64 {
        self.x + self.width / 2.0
    }
    pub fn top(&self) -> f64 {
        self.y - self.height / 2.0
    }
    pub fn bottom(&self) -> f64 {
        self.y + self.height / 2.0
    }
}

/// An axis-aligned rectangle: a label being placed, a node box it must miss, or a label already
/// placed.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    fn centered(cx: f64, cy: f64, w: f64, h: f64) -> Rect {
        Rect {
            x: cx - w / 2.0,
            y: cy - h / 2.0,
            w,
            h,
        }
    }

    fn of(p: &LabelPlacement) -> Rect {
        Rect {
            x: p.left(),
            y: p.top(),
            w: p.width,
            h: p.height,
        }
    }

    /// A copy grown by \`by\` on every side, for a test that needs clearance rather than mere
    /// non-overlap.
    fn inflated(&self, by: f64) -> Rect {
        Rect {
            x: self.x - by,
            y: self.y - by,
            w: self.w + 2.0 * by,
            h: self.h + 2.0 * by,
        }
    }

    /// Whether two rectangles come closer than \`gap\`. Touching is clear: the test is strict, so a
    /// label exactly \`gap\` from a box is not an overlap.
    fn close_to(&self, o: &Rect, gap: f64) -> bool {
        self.x - gap < o.x + o.w
            && o.x < self.x + self.w + gap
            && self.y - gap < o.y + o.h
            && o.y < self.y + self.h + gap
    }

    fn inside(&self, bounds: (f64, f64)) -> bool {
        self.x >= LABEL_EDGE_INSET
            && self.y >= LABEL_EDGE_INSET
            && self.x + self.w <= bounds.0 - LABEL_EDGE_INSET
            && self.y + self.h <= bounds.1 - LABEL_EDGE_INSET
    }

    fn contains(&self, p: (f64, f64)) -> bool {
        p.0 > self.x && p.0 < self.x + self.w && p.1 > self.y && p.1 < self.y + self.h
    }
}

/// Whether a straight segment passes through the interior of a rectangle (Liang-Barsky). Exact
/// rather than a bounding-box approximation, because the renderer draws lines that are not
/// axis-aligned - the sampled curve of a self-loop, and a run to a dangling marker - and a
/// bounding box around one of those would refuse a label a hundred units away from it. A segment
/// that only grazes the border is not a hit: the text is not struck through.
fn seg_hits_rect(a: (f64, f64), b: (f64, f64), r: &Rect) -> bool {
    let d = (b.0 - a.0, b.1 - a.1);
    let p = [-d.0, d.0, -d.1, d.1];
    let q = [a.0 - r.x, r.x + r.w - a.0, a.1 - r.y, r.y + r.h - a.1];
    let mut t0 = 0.0f64;
    let mut t1 = 1.0f64;
    for i in 0..4 {
        if p[i].abs() < 1e-12 {
            // Parallel to this edge: outside it unless the segment lies on the border, which is a
            // graze rather than a crossing.
            if q[i] <= 0.0 {
                return false;
            }
        } else {
            let t = q[i] / p[i];
            if p[i] < 0.0 {
                if t > t1 {
                    return false;
                }
                t0 = t0.max(t);
            } else {
                if t < t0 {
                    return false;
                }
                t1 = t1.min(t);
            }
        }
    }
    t0 < t1
}

/// The segments of a drawn polyline.
fn segments_of(pts: &[(f64, f64)]) -> Vec<Seg> {
    pts.windows(2).map(|w| (w[0], w[1])).collect()
}

/// A segment's length. Routes are axis-aligned, so the Manhattan length is the true length and it
/// avoids a square root in the inner loop.
fn seg_len(s: &Seg) -> f64 {
    (s.0 .0 - s.1 .0).abs() + (s.0 .1 - s.1 .1).abs()
}

/// The place on a line its own name would go if nothing else were in the way: the middle of the
/// longest segment, ties to the earliest.
fn ideal_point(own: &[Seg]) -> (f64, f64) {
    let mut best: Option<(f64, Seg)> = None;
    for s in own {
        let len = seg_len(s);
        if best.map_or(true, |(bl, _)| len > bl + 1e-9) {
            best = Some((len, *s));
        }
    }
    match best {
        Some((_, s)) => ((s.0 .0 + s.1 .0) / 2.0, (s.0 .1 + s.1 .1) / 2.0),
        None => (0.0, 0.0),
    }
}

/// The points of one square ring around \`c\`, walked clockwise from the top-left corner. The order
/// is fixed, so a ring search always resolves the same way.
fn ring_points(c: (f64, f64), r: f64) -> Vec<(f64, f64)> {
    if r <= 0.0 {
        return vec![c];
    }
    let steps = (2.0 * r / LEADER_STEP).round().max(1.0) as usize;
    let mut pts = Vec::with_capacity(4 * steps + 1);
    let at = |i: usize| -r + 2.0 * r * (i as f64) / (steps as f64);
    for i in 0..=steps {
        pts.push((c.0 + at(i), c.1 - r));
    }
    for i in 1..=steps {
        pts.push((c.0 + r, c.1 + at(i)));
    }
    for i in 1..=steps {
        pts.push((c.0 - at(i), c.1 + r));
    }
    for i in 1..=steps {
        pts.push((c.0 - r, c.1 - at(i)));
    }
    pts
}

/// The point of a line closest to \`p\`, clamped to each segment so it always lands on the line.
fn nearest_on_line(p: (f64, f64), own: &[Seg]) -> Option<(f64, f64)> {
    let mut best: Option<(f64, (f64, f64))> = None;
    for s in own {
        let (a, b) = *s;
        let cand = if (a.0 - b.0).abs() <= (a.1 - b.1).abs() {
            let (lo, hi) = order(a.1, b.1);
            (a.0, p.1.clamp(lo, hi))
        } else {
            let (lo, hi) = order(a.0, b.0);
            (p.0.clamp(lo, hi), a.1)
        };
        let d = (cand.0 - p.0).powi(2) + (cand.1 - p.1).powi(2);
        if best.map_or(true, |(bd, _)| d < bd - 1e-9) {
            best = Some((d, cand));
        }
    }
    best.map(|(_, cand)| cand)
}

/// The centre of every free vertical channel in `[lo, hi]` across the band `[y_lo, y_hi]`: an x
/// that no box covers, with at least [MIN_CHANNEL] of clearance. Ascending. This is the same rule
/// the edge router turns by, applied to the leader, so a leader turns in a channel an edge could
/// have used rather than cutting through a box.
fn leader_channels_x(lo: f64, hi: f64, y_lo: f64, y_hi: f64, boxes: &[Rect]) -> Vec<f64> {
    let mut spans: Vec<(f64, f64)> = boxes
        .iter()
        .filter(|b| b.y < y_hi + LEADER_BOX_GAP && b.y + b.h > y_lo - LEADER_BOX_GAP)
        .filter(|b| b.x < hi && b.x + b.w > lo)
        .map(|b| (b.x - LEADER_BOX_GAP, b.x + b.w + LEADER_BOX_GAP))
        .collect();
    merge_spans(&mut spans);
    channels_in(lo, hi, &spans)
}

/// The same, for a horizontal channel: a y that no box covers across `[x_lo, x_hi]`.
fn leader_channels_y(lo: f64, hi: f64, x_lo: f64, x_hi: f64, boxes: &[Rect]) -> Vec<f64> {
    let mut spans: Vec<(f64, f64)> = boxes
        .iter()
        .filter(|b| b.x < x_hi + LEADER_BOX_GAP && b.x + b.w > x_lo - LEADER_BOX_GAP)
        .filter(|b| b.y < hi && b.y + b.h > lo)
        .map(|b| (b.y - LEADER_BOX_GAP, b.y + b.h + LEADER_BOX_GAP))
        .collect();
    merge_spans(&mut spans);
    channels_in(lo, hi, &spans)
}

/// The centres of the gaps in `[lo, hi]` left by the merged blocking spans, ascending.
fn channels_in(lo: f64, hi: f64, merged: &[(f64, f64)]) -> Vec<f64> {
    let mut out = Vec::new();
    let mut cursor = lo;
    for &(a, b) in merged {
        if a - cursor > MIN_CHANNEL {
            out.push((cursor + a) / 2.0);
        }
        cursor = cursor.max(b);
    }
    if hi - cursor > MIN_CHANNEL {
        out.push((cursor + hi) / 2.0);
    }
    out
}

/// How many channel turns the leader router will try at each end. The nearest few are almost
/// always the answer, and a handful keeps the search bounded on a wide drawing.
const LEADER_CHANNELS: usize = 3;

/// Candidate orthogonal leaders from a label's box to its line, in a fixed order. Every candidate
/// starts on the box's own boundary - so a leader never strikes through its own label - and ends on
/// the line it names.
///
/// A plain L (out of the box, one elbow, onto the line) is tried first, from each of the box's four
/// sides. Where an L would cross a box the candidate leaves the box, turns into a free channel, runs
/// there, and turns again - the same move the edge router makes, so a leader reads as a thin edge
/// rather than as a line struck through the drawing.
fn leader_candidates(rect: &Rect, own: &[Seg], boxes: &[Rect]) -> Vec<Vec<(f64, f64)>> {
    let c = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
    let Some(target) = nearest_on_line(c, own) else {
        return Vec::new();
    };
    if rect.contains(target) {
        return Vec::new();
    }
    let left = rect.x;
    let right = rect.x + rect.w;
    let top = rect.y;
    let bottom = rect.y + rect.h;
    let exits = [
        (right, target.1.clamp(top, bottom)),
        (left, target.1.clamp(top, bottom)),
        (target.0.clamp(left, right), bottom),
        (target.0.clamp(left, right), top),
    ];
    let mut out: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut push = |pts: Vec<(f64, f64)>| {
        let mut pts = pts;
        pts.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6);
        if pts.len() < 2 || out.contains(&pts) {
            return;
        }
        out.push(pts);
    };
    for ex in exits {
        push(vec![ex, (target.0, ex.1), target]);
        push(vec![ex, (ex.0, target.1), target]);
        if (ex.1 - target.1).abs() > 1e-6 {
            let (lo, hi) = order(ex.0, target.0);
            let (y0, y1) = order(ex.1, target.1);
            for x in leader_channels_x(lo, hi, y0, y1, boxes)
                .into_iter()
                .take(LEADER_CHANNELS)
            {
                push(vec![ex, (x, ex.1), (x, target.1), target]);
            }
        }
        if (ex.0 - target.0).abs() > 1e-6 {
            let (lo, hi) = order(ex.1, target.1);
            let (x0, x1) = order(ex.0, target.0);
            for y in leader_channels_y(lo, hi, x0, x1, boxes)
                .into_iter()
                .take(LEADER_CHANNELS)
            {
                push(vec![ex, (ex.0, y), (target.0, y), target]);
            }
        }
    }
    out
}

/// The length of a leader polyline.
fn polyline_len(pts: &[(f64, f64)]) -> f64 {
    pts.windows(2).map(|w| seg_len(&(w[0], w[1]))).sum()
}

/// Whether a leader crosses no box and no label already placed.
fn leader_clear(leader: &[(f64, f64)], boxes: &[Rect], placed: &[Option<LabelPlacement>]) -> bool {
    leader.windows(2).all(|w| {
        !boxes
            .iter()
            .any(|b| seg_hits_rect(w[0], w[1], &b.inflated(LEADER_BOX_GAP)))
            && !placed
                .iter()
                .flatten()
                .any(|p| seg_hits_rect(w[0], w[1], &Rect::of(p).inflated(LEADER_BOX_GAP)))
    })
}

/// The shortest leader that gets from a label's box to its line, without running back through the
/// label's own text.
///
/// With `require_clear` the leader must also cross no box and no other label - the good answer,
/// and the one every ordinary placement gets. Without it, a leader that crosses something is
/// accepted when nothing clean exists: the labels are drawn BEFORE the node boxes, so such a
/// leader passes behind a box rather than through the text, and a label whose leader is partly
/// hidden still names its relationship while a dropped label names nothing at all. Ties go to the
/// candidate the enumeration produced first, so the choice is stable either way.
fn best_leader(
    rect: &Rect,
    own: &[Seg],
    boxes: &[Rect],
    placed: &[Option<LabelPlacement>],
    require_clear: bool,
) -> Option<Vec<(f64, f64)>> {
    let mut clean: Option<(f64, Vec<Point>)> = None;
    let mut any: Option<(f64, Vec<Point>)> = None;
    for leader in leader_candidates(rect, own, boxes) {
        if leader.iter().skip(1).any(|p| rect.contains(*p)) {
            continue;
        }
        let len = polyline_len(&leader);
        if leader_clear(&leader, boxes, placed)
            && clean.as_ref().map_or(true, |(bl, _)| len < bl - 1e-9)
        {
            clean = Some((len, leader.clone()));
        }
        if any.as_ref().map_or(true, |(bl, _)| len < bl - 1e-9) {
            any = Some((len, leader));
        }
    }
    if require_clear {
        clean.map(|(_, leader)| leader)
    } else {
        clean.or(any).map(|(_, leader)| leader)
    }
}
/// The test every candidate placement must pass: inside the drawing, off every box, off every
/// label already placed, and off every drawn line except the one segment it sits beside.
fn clear_for(
    r: &Rect,
    skip: Option<usize>,
    all: &[Seg],
    boxes: &[Rect],
    placed: &[Option<LabelPlacement>],
    bounds: (f64, f64),
) -> bool {
    r.inside(bounds)
        && !boxes.iter().any(|b| r.close_to(b, LABEL_BOX_GAP))
        && !placed
            .iter()
            .flatten()
            .any(|p| r.close_to(&Rect::of(p), LABEL_LABEL_GAP))
        && !all
            .iter()
            .enumerate()
            .any(|(k, s)| Some(k) != skip && seg_hits_rect(s.0, s.1, r))
}

/// The best placement whose box rests against the geometry of the drawing: against a box edge or
/// the drawing edge, in either axis.
///
/// Why the geometry and not a grid: a label that fits a corridor with a unit to spare has a window
/// of feasible positions narrower than any grid step worth walking, so a grid search would report
/// "no room" about a place that plainly has room. Every feasible position can be slid - in x and
/// in y, without leaving the feasible set - until it rests against the drawing edge or the edge of
/// some box in both axes, so the candidates here are aligned to the geometry itself. Among the
/// clear ones the winner is the one nearest its own line, ties to the first the sweep reaches.
#[allow(clippy::too_many_arguments)]
fn sweep_aligned(
    own: &[Seg],
    all: &[Seg],
    boxes: &[Rect],
    placed: &[Option<LabelPlacement>],
    bounds: (f64, f64),
    w: f64,
    h: f64,
    ideal: (f64, f64),
    require_clear_leader: bool,
) -> Option<(Point, Vec<Point>)> {
    let mut xs: Vec<f64> = vec![
        LABEL_EDGE_INSET + w / 2.0,
        bounds.0 - LABEL_EDGE_INSET - w / 2.0,
    ];
    let mut ys: Vec<f64> = vec![
        LABEL_EDGE_INSET + h / 2.0,
        bounds.1 - LABEL_EDGE_INSET - h / 2.0,
    ];
    for b in boxes {
        xs.push(b.x - LABEL_BOX_GAP - w / 2.0);
        xs.push(b.x + b.w + LABEL_BOX_GAP + w / 2.0);
        ys.push(b.y - LABEL_BOX_GAP - h / 2.0);
        ys.push(b.y + b.h + LABEL_BOX_GAP + h / 2.0);
    }
    let by_value = |a: &f64, b: &f64| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal);
    xs.sort_by(by_value);
    xs.dedup();
    ys.sort_by(by_value);
    ys.dedup();

    let mut best: Option<(f64, Point, Vec<Point>)> = None;
    for &y in &ys {
        for &x in &xs {
            let r = Rect::centered(x, y, w, h);
            if !clear_for(&r, None, all, boxes, placed, bounds) {
                continue;
            }
            let Some(leader) = best_leader(&r, own, boxes, placed, require_clear_leader) else {
                continue;
            };
            let near = nearest_on_line((x, y), own).unwrap_or(ideal);
            let d = (x - near.0).powi(2) + (y - near.1).powi(2);
            if best.as_ref().map_or(true, |(bd, _, _)| d < bd - 1e-9) {
                best = Some((d, (x, y), leader));
            }
        }
    }
    best.map(|(_, c, leader)| (c, leader))
}

/// Place one label: beside its own line where that works, otherwise nearby and led to it.
#[allow(clippy::too_many_arguments)]
fn place_one(
    own: &[Seg],
    first: usize,
    all: &[Seg],
    boxes: &[Rect],
    placed: &[Option<LabelPlacement>],
    bounds: (f64, f64),
    w: f64,
    h: f64,
) -> Option<LabelPlacement> {
    // 1. Along the line: longest segment first, middle station first, both sides of the line.
    let mut by_len: Vec<usize> = (0..own.len()).collect();
    by_len.sort_by(|&a, &b| {
        seg_len(&own[b])
            .partial_cmp(&seg_len(&own[a]))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.cmp(&b))
    });
    for &k in &by_len {
        let (a, b) = own[k];
        let horizontal = (a.1 - b.1).abs() < 1e-6;
        for &f in &LABEL_STATIONS {
            let (sx, sy) = (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f);
            for side in [1.0f64, -1.0f64] {
                let (cx, cy) = if horizontal {
                    (sx, sy - side * (h / 2.0 + LABEL_LINE_GAP))
                } else {
                    (sx - side * (w / 2.0 + LABEL_LINE_GAP), sy)
                };
                let r = Rect::centered(cx, cy, w, h);
                if clear_for(&r, Some(first + k), all, boxes, placed, bounds) {
                    return Some(LabelPlacement {
                        x: cx,
                        y: cy,
                        width: w,
                        height: h,
                        leader: Vec::new(),
                    });
                }
            }
        }
    }

    // 2. Off the line, with a leader: the nearest clear pocket to the line, found in rings, and the
    // shortest orthogonal leader from the label's edge back to the line that crosses nothing.
    //
    // The ring is walked outward from the line and each ring is walked from the point nearest the
    // line outward, so the first pocket that works is the one the reader's eye reaches first.
    let ideal = ideal_point(own);
    let mut chosen: Option<(Point, Vec<Point>)> = None;
    'rings: for ring in 0..=LEADER_RINGS {
        let radius = ring as f64 * LEADER_STEP;
        let mut points = ring_points(ideal, radius);
        points.sort_by(|a, b| {
            let da = (a.0 - ideal.0).powi(2) + (a.1 - ideal.1).powi(2);
            let db = (b.0 - ideal.0).powi(2) + (b.1 - ideal.1).powi(2);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        });
        for c in points {
            let r = Rect::centered(c.0, c.1, w, h);
            if !clear_for(&r, None, all, boxes, placed, bounds) {
                continue;
            }
            if let Some(leader) = best_leader(&r, own, boxes, placed, true) {
                chosen = Some((c, leader));
                break 'rings;
            }
        }
    }

    // 3. The positions where a label rests against a box edge or the drawing edge. A uniform grid
    //    can miss a corridor that fits the label with a unit to spare, because the window that fits
    //    is narrower than the grid step - the coffee machine has exactly such a corridor. These
    //    candidates are aligned to the geometry itself, so a corridor that fits at all contains one,
    //    and this bounds the search when the rings come up empty on a crowded drawing.
    if chosen.is_none() {
        chosen = sweep_aligned(own, all, boxes, placed, bounds, w, h, ideal, true);
    }

    // 4. A clear place, led however it can be led. A name too wide for any free channel - the
    //    corpus has a 306-unit one - may have no box position whose leader is also clean. The label
    //    itself must still land on nothing; the leader may pass behind a box, because the labels are
    //    drawn before the boxes and a partly hidden leader still names its relationship, while a
    //    dropped label names nothing at all.
    if chosen.is_none() {
        chosen = sweep_aligned(own, all, boxes, placed, bounds, w, h, ideal, false);
    }

    let ((cx, cy), leader) = chosen?;
    Some(LabelPlacement {
        x: cx,
        y: cy,
        width: w,
        height: h,
        leader,
    })
}
/// Place every edge label so a reader can find it and read it: never under a box, never under
/// another label, and never across a line that is not the one it names.
///
/// \`lines[i]\` is the polyline the renderer draws for edge \`i\` and \`labels[i]\` is the text it
/// carries; an empty label draws nothing. \`bounds\` is the drawing area - the caption band is not
/// part of it.
///
/// The method, least invasive first, per label:
///
/// 1. ALONG THE LINE. Every segment of the edge, longest first, is walked with stations - the
///    middle first, then fanning out - and both sides of the line are tried at each one. A label
///    that lands clear of every box, every label already placed and every other line is put there.
///    This is the repair for the crowding that made this function exist. The old rule dropped
///    every label at the midpoint of the chord between its two anchors; for a fan-out that shares
///    a trunk - the coffee machine's eleven containment edges out of one box - that midpoint IS
///    the trunk line, so eleven names landed on one vertical line, on top of each other, and under
///    each other's boxes.
/// 2. OFF THE LINE, WITH A LEADER. When no station on the line is clear - a 44-unit channel asked
///    to hold a 164-unit name - search outward from the line in rings, take the nearest clear
///    pocket, and lead from the label's edge to the line with an orthogonal leader. That is
///    standard engineering-drawing practice, and it is what keeps the name readable instead of
///    dropping it.
/// 3. WHERE IT RESTS AGAINST THE GEOMETRY. The rings walk a grid, and a corridor that fits a wide
///    label with a unit to spare can be narrower than the grid step. Every feasible place can be
///    slid until it rests against a box edge or the drawing edge, so those positions are tried too,
///    nearest the line first.
/// 4. A CLEAR PLACE, LED HOWEVER IT CAN BE LED. A name too wide for any free channel may have no
///    clear place whose leader is also clean. The label itself must still land on nothing; its
///    leader may then pass behind a box, which still names the relationship - unlike a dropped
///    label, which names nothing.
/// 5. NOTHING. \`None\` means no clear position exists anywhere in the drawing at all. The caller
///    must not draw the label - and must not quietly lose the relationship either: the renderer
///    collects every name it could not place and states it on the page.
///
/// Labels are placed biggest first, ties by index, so the largest names - which need the largest
/// clear rectangle - are not left with the leavings. Every iteration is over an ordered slice and
/// every tie is broken explicitly, so the placement is a pure function of the geometry: the same
/// model always exports byte-identically.
pub fn place_labels(
    nodes: &[NodeBox],
    lines: &[Vec<(f64, f64)>],
    labels: &[&str],
    bounds: (f64, f64),
) -> Vec<Option<LabelPlacement>> {
    let n = labels.len().min(lines.len());
    let boxes: Vec<Rect> = nodes
        .iter()
        .map(|b| Rect {
            x: b.x,
            y: b.y,
            w: b.width,
            h: b.height,
        })
        .collect();
    // Every drawn segment, once, flattened, with the flat index of each line's first segment so a
    // station can exclude exactly the one segment it sits beside.
    let segs: Vec<Vec<Seg>> = lines[..n].iter().map(|p| segments_of(p)).collect();
    let mut all: Vec<Seg> = Vec::new();
    let mut first: Vec<usize> = Vec::with_capacity(n);
    for list in &segs {
        first.push(all.len());
        all.extend_from_slice(list);
    }

    let mut out: Vec<Option<LabelPlacement>> = vec![None; n];
    let mut order: Vec<usize> = (0..n).filter(|&i| !labels[i].is_empty()).collect();
    order.sort_by(|&a, &b| {
        let (wa, _) = edge_label_size(labels[a]);
        let (wb, _) = edge_label_size(labels[b]);
        wb.partial_cmp(&wa)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.cmp(&b))
    });

    for &i in &order {
        let (w, h) = edge_label_size(labels[i]);
        if let Some(p) = place_one(&segs[i], first[i], &all, &boxes, &out, bounds, w, h) {
            out[i] = Some(p);
        }
    }
    out
}
