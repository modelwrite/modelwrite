// SPDX-License-Identifier: AGPL-3.0-or-later
//! The metric-definitions completeness contract: a test fails when an engine
//! metric has no definition row, and when a definition has no engine metric
//! behind it (rule 7: nothing is invented).

use analytics::metrics::{metric_definitions, ENGINE_METRIC_IDS};
use std::collections::HashSet;

#[test]
fn every_engine_metric_has_a_definition_row() {
    let definitions = metric_definitions();
    let ids: HashSet<&str> = definitions.iter().map(|d| d.id.as_str()).collect();
    for id in ENGINE_METRIC_IDS {
        assert!(
            ids.contains(id),
            "engine metric {} has no definition row",
            id
        );
    }
}

#[test]
fn no_definition_without_an_engine_metric() {
    let engine: HashSet<&str> = ENGINE_METRIC_IDS.iter().copied().collect();
    for definition in metric_definitions() {
        assert!(
            engine.contains(definition.id.as_str()),
            "definition {} has no engine metric behind it; rule 7: nothing is invented",
            definition.id
        );
    }
}

#[test]
fn every_definition_is_complete() {
    for definition in metric_definitions() {
        assert!(
            !definition.name.is_empty(),
            "{} has an empty name",
            definition.id
        );
        assert!(
            !definition.description.is_empty(),
            "{} has an empty description",
            definition.id
        );
    }
}
