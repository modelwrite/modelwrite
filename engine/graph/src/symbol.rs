// SPDX-License-Identifier: AGPL-3.0-or-later
//! The symbol boundary: one place that turns a model element's declaration into SVG geometry.
//!
//! The renderer must not care where a shape came from. Every node is reduced to a
//! [Symbol] by [symbol], and the renderer draws whatever that returns. Two vocabularies live
//! behind the boundary today:
//!
//! * **Kind glyphs** - hand-drawn monochrome marks for the nine kinds the platform models
//!   (block, requirement, activity, signal, interface, actor, state, stateMachine, usecase),
//!   plus a neutral default for anything unrecognised. Drawn with `currentColor` so they
//!   take the token palette. An unknown kind gets the neutral default - never nothing, never a
//!   broken image.
//! * **MIL-STD-2525D / APP-6D frames** - for a node that *declares* a symbol (a stereotype
//!   carrying a numeric 20-character SIDC). The frame is the standard's affiliation
//!   (friend/hostile/neutral/unknown) crossed with the battle dimension (land / sea surface /
//!   subsurface / air / space), filled with the standard's affiliation colour. The SIDC is
//!   never guessed: a declaration this build does not know is returned as
//!   [Symbol::Unmappable] and must be reported, never silently drawn as a plain box.
//!
//! **Standard version implemented: MIL-STD-2525D / APP-6D (numeric 20-character SIDC).**
//! All frame and glyph geometry here is hand-drawn; `milsymbol` v3.0.4 (MIT) is the reference
//! implementation, cited in NOTICE, but no third-party path data is vendored into this crate.

use okf::types::Graph;

/// A monochrome glyph: primary SVG path data plus an optional secondary accent path, authored
/// against a square viewBox of side [Glyph::view]. Both paths draw in `currentColor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyph {
    /// Primary filled path.
    pub path: &'static str,
    /// Optional secondary filled path (same colour).
    pub accent: Option<&'static str>,
    /// Side length of the square viewBox the path data is authored against.
    pub view: u32,
}

/// A glyph resolves to a kind mark, a mapped 2525 symbol, or a declared-but-unmappable symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Symbol {
    /// A kind glyph (or the neutral default for an unknown kind).
    Kind(Glyph),
    /// A declared MIL-STD-2525D symbol this build can draw.
    MilStd2525 {
        affiliation: Affiliation,
        dimension: Dimension,
    },
    /// A declared symbol this build cannot draw. The node still renders (kind glyph + label),
    /// but the declaration must be reported - never silently drawn as a plain box.
    Unmappable {
        /// The raw declared symbol string.
        declared: String,
    },
}

/// The 2525 standard identity, folded into the four affiliations the platform draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Affiliation {
    Friend,
    Hostile,
    Neutral,
    Unknown,
}

/// The 2525 battle dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    Land,
    SeaSurface,
    Subsurface,
    Air,
    Space,
}

/// A parsed SIDC: affiliation and dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sidc {
    pub affiliation: Affiliation,
    pub dimension: Dimension,
}

/// The side length of the square viewBox every frame and platform glyph is authored against.
pub const FRAME_VIEW: u32 = 100;

/// The standard affiliation fill colours (2525 colour mode).
const FRIEND_FILL: &str = "#3d8bfd";
const HOSTILE_FILL: &str = "#ef4444";
const NEUTRAL_FILL: &str = "#22c55e";
const UNKNOWN_FILL: &str = "#f59e0b";

/// A 2525 frame: its outline path plus how to fill/stroke it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub path: &'static str,
    pub fill: &'static str,
    /// Whether the frame stroke is dashed (the standard's "unknown" convention).
    pub dash: bool,
}

// ---------------------------------------------------------------------------
// Kind glyphs (hand-drawn, 24x24 viewBox, currentColor).
// ---------------------------------------------------------------------------

/// Neutral default for an unknown kind: a plain filled square.
pub const NEUTRAL_GLYPH: Glyph = Glyph {
    path: "M5 5 H19 V19 H5 Z",
    accent: None,
    view: 24,
};

/// A block: an isometric cube silhouette.
pub const BLOCK_GLYPH: Glyph = Glyph {
    path: "M12 3 L20 7 L20 17 L12 21 L4 17 L4 7 Z",
    accent: None,
    view: 24,
};

/// A requirement: a document with a folded top-right corner.
pub const REQUIREMENT_GLYPH: Glyph = Glyph {
    path: "M5 3 H14 L19 8 V21 H5 Z",
    accent: None,
    view: 24,
};

/// An activity: a play triangle.
pub const ACTIVITY_GLYPH: Glyph = Glyph {
    path: "M9 6 L19 12 L9 18 Z",
    accent: None,
    view: 24,
};

/// A signal: a lightning bolt.
pub const SIGNAL_GLYPH: Glyph = Glyph {
    path: "M13 3 L6 13 H10 L9 21 L18 9 H13 Z",
    accent: None,
    view: 24,
};

/// An interface: a ball (lollipop head) with a short stem (the ball-and-socket convention).
pub const INTERFACE_GLYPH: Glyph = Glyph {
    path: "M8 7 a4 4 0 1 0 8 0 a4 4 0 1 0 -8 0",
    accent: Some("M11 10 H13 V18 H11 Z"),
    view: 24,
};

/// An actor: a head and shoulders.
pub const ACTOR_GLYPH: Glyph = Glyph {
    path: "M8.5 9 a3.5 3.5 0 1 0 7 0 a3.5 3.5 0 1 0 -7 0 M5 20 c 0 -4.5 3.1 -7.5 7 -7.5 c 3.9 0 7 3 7 7.5 Z",
    accent: None,
    view: 24,
};

/// A state: a filled circle (the state node of a state chart).
pub const STATE_GLYPH: Glyph = Glyph {
    path: "M4 12 a8 8 0 1 0 16 0 a8 8 0 1 0 -16 0",
    accent: None,
    view: 24,
};

/// A state machine: two states joined by a transition arrow.
pub const STATE_MACHINE_GLYPH: Glyph = Glyph {
    path: "M5.5 12 a2.5 2.5 0 1 0 5 0 a2.5 2.5 0 1 0 -5 0 M13.5 12 a2.5 2.5 0 1 0 5 0 a2.5 2.5 0 1 0 -5 0",
    accent: Some("M10.5 10 L14 12 L10.5 14 Z"),
    view: 24,
};

/// A use case: an ellipse.
pub const USECASE_GLYPH: Glyph = Glyph {
    path:
        "M12 6.5 c 4.97 0 9 2.46 9 5.5 s -4.03 5.5 -9 5.5 s -9 -2.46 -9 -5.5 s 4.03 -5.5 9 -5.5 Z",
    accent: None,
    view: 24,
};

/// The explicit (kind) -> glyph table. Stereotype refinement for the bundled set is not yet
/// used (every kind has one glyph); the table is the single place to extend that.
pub const KIND_GLYPHS: &[(&str, Glyph)] = &[
    ("block", BLOCK_GLYPH),
    ("requirement", REQUIREMENT_GLYPH),
    ("activity", ACTIVITY_GLYPH),
    ("signal", SIGNAL_GLYPH),
    ("interface", INTERFACE_GLYPH),
    ("actor", ACTOR_GLYPH),
    ("state", STATE_GLYPH),
    ("stateMachine", STATE_MACHINE_GLYPH),
    ("usecase", USECASE_GLYPH),
];

/// The glyph for a kind, or the neutral default when the kind is unknown.
pub fn kind_glyph(kind: &str) -> Glyph {
    KIND_GLYPHS
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, g)| *g)
        .unwrap_or(NEUTRAL_GLYPH)
}

// ---------------------------------------------------------------------------
// SIDC declaration and parsing.
// ---------------------------------------------------------------------------

/// Whether a string is a well-formed numeric 20-character SIDC.
fn is_sidc(code: &str) -> bool {
    code.len() == 20 && code.bytes().all(|b| b.is_ascii_digit())
}

/// The first stereotype that declares a symbol, with any `sidc:` / `symbol:` prefix stripped.
///
/// A model declares its symbol rather than leaving the platform to guess it: either a bare
/// numeric 20-character SIDC, or one prefixed `sidc:` or `symbol:`.
pub fn declared_sidc(stereotypes: &[String]) -> Option<String> {
    for s in stereotypes {
        let code = s
            .strip_prefix("sidc:")
            .or_else(|| s.strip_prefix("symbol:"))
            .unwrap_or(s.as_str());
        if is_sidc(code) {
            return Some(code.to_string());
        }
    }
    None
}

/// Parse a numeric 20-character MIL-STD-2525D SIDC into its affiliation and dimension.
///
/// Field positions (0-based, official 2525D layout): 0 version, 1 context, 2 standard
/// identity, 3-4 symbol set. Anything outside the supported set yields `None`, which the
/// caller reports as unmappable rather than guessing.
pub fn parse_sidc(code: &str) -> Option<Sidc> {
    if !is_sidc(code) {
        return None;
    }
    let bytes = code.as_bytes();
    let affiliation = match bytes[2] {
        b'0' | b'1' => Affiliation::Unknown, // pending / unknown
        b'2' | b'3' => Affiliation::Friend,  // assumed friend / friend
        b'4' => Affiliation::Neutral,
        b'5' | b'6' => Affiliation::Hostile, // suspect / hostile
        _ => return None,
    };
    let dimension = match &code[3..5] {
        "01" => Dimension::Air,
        "05" => Dimension::SeaSurface,
        "06" => Dimension::Subsurface,
        "10" | "11" => Dimension::Land,
        "26" => Dimension::Space,
        _ => return None,
    };
    Some(Sidc {
        affiliation,
        dimension,
    })
}

/// Resolve a node's declaration to a [Symbol]: a declared SIDC wins over the kind glyph; an
/// unknown declared symbol is [Symbol::Unmappable]; otherwise the kind glyph (neutral default
/// for an unknown kind) is returned.
pub fn symbol(kind: &str, stereotypes: &[String]) -> Symbol {
    if let Some(declared) = declared_sidc(stereotypes) {
        return match parse_sidc(&declared) {
            Some(sidc) => Symbol::MilStd2525 {
                affiliation: sidc.affiliation,
                dimension: sidc.dimension,
            },
            None => Symbol::Unmappable { declared },
        };
    }
    Symbol::Kind(kind_glyph(kind))
}

// ---------------------------------------------------------------------------
// 2525 frame geometry.
// ---------------------------------------------------------------------------

/// The frame outline for an affiliation, per the standard's monochrome shape convention.
fn frame_path(affiliation: Affiliation, dimension: Dimension) -> &'static str {
    match dimension {
        Dimension::Air => match affiliation {
            Affiliation::Friend => "M20 78 C20 42 40 20 50 20 C60 20 80 42 80 78 Z",
            Affiliation::Hostile => "M20 78 L20 42 L50 12 L80 42 L80 78 Z",
            Affiliation::Neutral => "M26 30 H74 V78 H26 Z",
            Affiliation::Unknown => UNKNOWN_FRAME,
        },
        Dimension::Space => SPACE_FRAME,
        // Land, sea surface and subsurface share the rectangular frame family.
        _ => match affiliation {
            Affiliation::Friend => "M18 26 H82 V74 H18 Z",
            Affiliation::Hostile => "M50 12 L88 50 L50 88 L12 50 Z",
            Affiliation::Neutral => "M28 28 H72 V72 H28 Z",
            Affiliation::Unknown => UNKNOWN_FRAME,
        },
    }
}

/// The "unknown" frame: a square whose sides bow outward (the standard's unknown convention).
const UNKNOWN_FRAME: &str =
    "M30 30 C30 8 70 8 70 30 C92 30 92 70 70 70 C70 92 30 92 30 70 C8 70 8 30 30 30 Z";

/// The space frame: an orbit oval, shared across affiliations (affiliation carried by fill).
const SPACE_FRAME: &str =
    "M50 16 C70 16 84 30 84 50 C84 70 70 84 50 84 C30 84 16 70 16 50 C16 30 30 16 50 16 Z";

fn affiliation_fill(affiliation: Affiliation) -> &'static str {
    match affiliation {
        Affiliation::Friend => FRIEND_FILL,
        Affiliation::Hostile => HOSTILE_FILL,
        Affiliation::Neutral => NEUTRAL_FILL,
        Affiliation::Unknown => UNKNOWN_FILL,
    }
}

/// The 2525 frame for a mapped affiliation x dimension pair.
pub fn frame(affiliation: Affiliation, dimension: Dimension) -> Frame {
    Frame {
        path: frame_path(affiliation, dimension),
        fill: affiliation_fill(affiliation),
        dash: affiliation == Affiliation::Unknown,
    }
}

/// The generic platform glyph drawn inside a frame, as a dark silhouette in the frame's
/// 100x100 viewBox. Generic and hand-drawn - a wheeled vehicle, a ship hull, an aircraft
/// planform, and so on - never a real product's likeness.
pub fn platform_glyph(dimension: Dimension) -> &'static str {
    match dimension {
        Dimension::Land => {
            "M30 54 H70 V62 H30 Z M29 66 a5 5 0 1 0 10 0 a5 5 0 1 0 -10 0 M61 66 a5 5 0 1 0 10 0 a5 5 0 1 0 -10 0"
        }
        Dimension::SeaSurface => "M28 58 H72 L64 74 H36 Z M44 42 H56 V58 H44 Z",
        Dimension::Subsurface => "M26 56 Q50 48 74 56 V64 Q50 72 26 64 Z M42 40 H50 V52 H42 Z",
        Dimension::Air => "M50 16 L78 70 L62 76 L50 66 L38 76 L22 70 Z",
        Dimension::Space => "M44 44 H56 V62 H44 Z M24 50 H44 V58 H24 Z M56 50 H76 V58 H56 Z",
    }
}

// ---------------------------------------------------------------------------
// Unmappable findings.
// ---------------------------------------------------------------------------

/// A node whose declared symbol this build cannot draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub node_id: String,
    pub declared: String,
}

/// Every node that declares a symbol this build cannot draw, sorted by node id so the report is
/// deterministic. The renderer must surface these - a diagram must never imply a symbol it did
/// not draw.
pub fn unmappable_findings(graph: &Graph) -> Vec<Finding> {
    let mut out = Vec::new();
    for node in &graph.nodes {
        if let Symbol::Unmappable { declared } = symbol(&node.kind, &node.stereotypes) {
            out.push(Finding {
                node_id: node.id.clone(),
                declared,
            });
        }
    }
    out.sort_by(|a, b| a.node_id.cmp(&b.node_id));
    out
}
