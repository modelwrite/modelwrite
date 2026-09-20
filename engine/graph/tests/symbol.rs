// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tests for the symbol boundary: kind glyphs, the neutral default, SIDC parsing, the 2525
//! frame geometry, and the unmappable-symbol finding.

use graph::symbol::{self, Affiliation, Dimension, Symbol, KIND_GLYPHS, NEUTRAL_GLYPH};

fn strs(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn every_known_kind_has_a_distinct_glyph() {
    // The table is the single source of truth for what the renderer can draw.
    let kinds: Vec<&str> = KIND_GLYPHS.iter().map(|(k, _)| *k).collect();
    assert_eq!(
        kinds,
        vec![
            "block",
            "requirement",
            "activity",
            "signal",
            "interface",
            "actor",
            "state",
            "stateMachine",
            "usecase"
        ],
        "the platform's nine kinds must all have a glyph"
    );
    for (kind, glyph) in KIND_GLYPHS {
        assert!(!glyph.path.is_empty(), "{kind} glyph must have a path");
        assert_eq!(glyph.view, 24, "{kind} glyph is authored at 24px");
    }
}

#[test]
fn symbol_returns_a_glyph_for_every_known_kind() {
    for (kind, glyph) in KIND_GLYPHS {
        assert_eq!(
            symbol::symbol(kind, &[]),
            Symbol::Kind(*glyph),
            "{kind} must resolve to its glyph"
        );
    }
}

#[test]
fn unknown_kind_degrades_to_the_neutral_default() {
    assert_eq!(symbol::symbol("bogus", &[]), Symbol::Kind(NEUTRAL_GLYPH));
    assert_eq!(symbol::symbol("", &[]), Symbol::Kind(NEUTRAL_GLYPH));
    assert!(!NEUTRAL_GLYPH.path.is_empty(), "the default is never empty");
}

#[test]
fn kind_glyph_is_a_pure_function() {
    let a = symbol::kind_glyph("signal");
    let b = symbol::kind_glyph("signal");
    assert_eq!(a, b, "the same kind always returns the same glyph");
    assert_eq!(a.path, b.path);
}

#[test]
fn declared_sidc_recognises_bare_and_prefixed_stereotypes() {
    let bare = "10310000012000000000";
    let prefixed = strs(&["sidc:10310000012000000000"]);
    let symbol_prefixed = strs(&["symbol:10310000012000000000"]);
    let other = strs(&["Requirement", "trace"]);
    assert_eq!(symbol::declared_sidc(&prefixed), Some(bare.to_string()));
    assert_eq!(
        symbol::declared_sidc(&symbol_prefixed),
        Some(bare.to_string())
    );
    assert_eq!(symbol::declared_sidc(&other), None);
    assert_eq!(
        symbol::declared_sidc(&strs(&["10310000012000000000"])),
        Some(bare.to_string()),
        "a bare SIDC stereotype is also a declaration"
    );
}

#[test]
fn parse_sidc_maps_affiliation_and_dimension() {
    // Official 2525D layout: idx 2 = standard identity, idx 3-4 = symbol set.
    let friend_land = symbol::parse_sidc("10310000012000000000").unwrap();
    assert_eq!(friend_land.affiliation, Affiliation::Friend);
    assert_eq!(friend_land.dimension, Dimension::Land);

    let hostile_air = symbol::parse_sidc("10601000011000000000").unwrap();
    assert_eq!(hostile_air.affiliation, Affiliation::Hostile);
    assert_eq!(hostile_air.dimension, Dimension::Air);

    let neutral_subsurface = symbol::parse_sidc("10406000011000000000").unwrap();
    assert_eq!(neutral_subsurface.affiliation, Affiliation::Neutral);
    assert_eq!(neutral_subsurface.dimension, Dimension::Subsurface);

    let unknown_sea = symbol::parse_sidc("10105000011000000000").unwrap();
    assert_eq!(unknown_sea.affiliation, Affiliation::Unknown);
    assert_eq!(unknown_sea.dimension, Dimension::SeaSurface);

    let friend_space = symbol::parse_sidc("10326000011000000000").unwrap();
    assert_eq!(friend_space.affiliation, Affiliation::Friend);
    assert_eq!(friend_space.dimension, Dimension::Space);
}

#[test]
fn parse_sidc_rejects_unknown_codes_instead_of_guessing() {
    assert!(
        symbol::parse_sidc("10399000012000000000").is_none(),
        "unknown symbol set"
    );
    assert!(
        symbol::parse_sidc("10910000012000000000").is_none(),
        "unknown identity"
    );
    assert!(symbol::parse_sidc("short").is_none(), "not a 20-char SIDC");
    assert!(symbol::parse_sidc("").is_none());
}

#[test]
fn declared_but_unknown_symbol_is_unmappable_not_silent() {
    let unknown = strs(&["sidc:10399000012000000000"]);
    assert_eq!(
        symbol::symbol("block", &unknown),
        Symbol::Unmappable {
            declared: "10399000012000000000".to_string()
        },
        "a declared symbol this build does not know must be reported, not boxed"
    );
}

#[test]
fn every_affiliation_dimension_pair_has_a_frame() {
    let affiliations = [
        Affiliation::Friend,
        Affiliation::Hostile,
        Affiliation::Neutral,
        Affiliation::Unknown,
    ];
    let dimensions = [
        Dimension::Land,
        Dimension::SeaSurface,
        Dimension::Subsurface,
        Dimension::Air,
        Dimension::Space,
    ];
    for &aff in &affiliations {
        for &dim in &dimensions {
            let frame = symbol::frame(aff, dim);
            assert!(!frame.path.is_empty(), "{aff:?} x {dim:?} frame");
            assert!(!frame.fill.is_empty(), "{aff:?} x {dim:?} fill");
            assert_eq!(
                frame.dash,
                aff == Affiliation::Unknown,
                "only the unknown affiliation uses a dashed frame"
            );
        }
    }
}

#[test]
fn frames_distinguish_affiliation_by_shape_in_the_rectangular_family() {
    // The standard's monochrome convention: friend/neutral/hostile/unknown differ by shape.
    let friend = symbol::frame(Affiliation::Friend, Dimension::Land).path;
    let hostile = symbol::frame(Affiliation::Hostile, Dimension::Land).path;
    let neutral = symbol::frame(Affiliation::Neutral, Dimension::Land).path;
    let unknown = symbol::frame(Affiliation::Unknown, Dimension::Land).path;
    let shapes = [friend, hostile, neutral, unknown];
    for (i, a) in shapes.iter().enumerate() {
        for (j, b) in shapes.iter().enumerate() {
            if i != j {
                assert_ne!(a, b, "land frame shapes must differ by affiliation");
            }
        }
    }
}

#[test]
fn every_dimension_has_a_platform_glyph() {
    for dim in [
        Dimension::Land,
        Dimension::SeaSurface,
        Dimension::Subsurface,
        Dimension::Air,
        Dimension::Space,
    ] {
        assert!(!symbol::platform_glyph(dim).is_empty(), "{dim:?} glyph");
    }
}
