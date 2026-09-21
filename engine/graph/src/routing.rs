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

use crate::layout::NodeBox;

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
