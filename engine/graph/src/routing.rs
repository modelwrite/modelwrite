// SPDX-License-Identifier: AGPL-3.0-or-later
//! Deterministic orthogonal (Manhattan) edge routing.
//!
//! An engineering diagram never lets an arrow wander through a box. Edges leave a node at the
//! centre of a side (left/right/top/bottom), travel in right angles, and arrive at the centre of
//! the facing side. The turning segments are placed in the free channels between node boxes so a
//! route never runs through another node; where the direct route is blocked, the edge detours
//! around the obstacle. Every choice here is a pure function of the boxes, so the same model
//! routes byte-identically.

use crate::layout::NodeBox;

/// The side of a node an edge leaves from or arrives at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// The centre of the first gap in `[lo, hi]` between the (sorted, merged) blocking intervals;
/// falls back to the midpoint when no gap is wide enough.
fn first_gap(lo: f64, hi: f64, intervals: &[(f64, f64)]) -> f64 {
    let mut sorted = intervals.to_vec();
    sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for &(a, b) in &sorted {
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
