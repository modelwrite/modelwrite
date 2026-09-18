// SPDX-License-Identifier: AGPL-3.0-or-later
//! Analytics and data fusion: answers that cross a single model's boundary, fusing
//! models with external structured and unstructured sources. The ruling that shapes
//! everything here: A FUSED ANSWER MUST BE TRACEABLE TO ITS SOURCES, AND ITS
//! CONFIDENCE MUST BE ITS WEAKEST LINK. Analytics reads and produces findings; it
//! never writes to a model.

pub mod dataset;
pub mod source;

pub use dataset::{CsvError, Dataset};
pub use source::{Registry, RegistryError, Source, SourceKind, TrustLevel};
