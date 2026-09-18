// SPDX-License-Identifier: AGPL-3.0-or-later
use serde::{Deserialize, Serialize};

/// How faithfully one mapping carries a source construct into OKF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MappingVerdict {
    /// The target expresses the same thing; the migration is mechanical.
    Exact,
    /// The target expresses something close, and the difference matters.
    Lossy,
    /// The target cannot express it at all.
    Unmappable,
}

/// One entry in a binding's mapping matrix: a named subject, its verdict, and
/// the reason. Nothing is ever dropped silently; a construct the binding could
/// not carry is a `Lossy` or `Unmappable` entry here, with its subject named.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mapping {
    pub subject: String,
    pub verdict: MappingVerdict,
    pub note: String,
}

/// The record of everything a binding could not carry, produced during import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LossReport {
    pub binding: crate::BindingInfo,
    pub mappings: Vec<Mapping>,
    pub artifact_hash: String,
}

impl LossReport {
    /// True when nothing was lost or left unmapped: every mapping is `Exact`.
    pub fn is_lossless(&self) -> bool {
        self.mappings
            .iter()
            .all(|m| m.verdict == MappingVerdict::Exact)
    }

    /// The entries a human must decide on: everything that is not `Exact`.
    pub fn blocking(&self) -> Vec<&Mapping> {
        self.mappings
            .iter()
            .filter(|m| m.verdict != MappingVerdict::Exact)
            .collect()
    }

    /// The recognised declarations: entries recorded with an `Exact` verdict
    /// whose note begins with [`DECLARATION_MARKER`]. A declaration names a
    /// construct the reader understood that carries no model content (a profile
    /// application, a package import, an annotation or root metadata). Not
    /// carrying it is not a loss, so it never appears in [`Self::blocking`].
    pub fn declarations(&self) -> Vec<&Mapping> {
        self.mappings
            .iter()
            .filter(|m| {
                m.verdict == MappingVerdict::Exact && m.note.starts_with(DECLARATION_MARKER)
            })
            .collect()
    }

    /// The content losses: model content the binding could not express, each an
    /// `Unmappable` entry naming what did not survive.
    pub fn content_losses(&self) -> Vec<&Mapping> {
        self.mappings
            .iter()
            .filter(|m| m.verdict == MappingVerdict::Unmappable)
            .collect()
    }

    /// The lossy entries: content carried imperfectly (a dropped id, name or
    /// flattened package). These are still blocking decisions, but the content
    /// itself survived.
    pub fn lossy(&self) -> Vec<&Mapping> {
        self.mappings
            .iter()
            .filter(|m| m.verdict == MappingVerdict::Lossy)
            .collect()
    }
}

/// The note prefix marking a mapping as a recognised declaration: a construct
/// the reader understood that carries no model content (a profile application,
/// a package import, an annotation or root metadata). A declaration is not a
/// loss, so it never appears in [`LossReport::blocking`].
pub const DECLARATION_MARKER: &str = "declaration:";

/// The verdict a human reads first on an import summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportVerdict {
    /// No model content was lost. (Lossy id/name/flattening drops may remain.)
    Lossless,
    /// Some model content could not be carried into OKF.
    ContentLosses,
}

/// The human-first summary of one import: what came in, what was recognised as
/// declaration, and what model content did not survive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportSummary {
    /// Model content imported (for the XMI binding, the blocks in `structure`).
    pub elements_imported: u64,
    /// Declarations recognised and deliberately not carried (they carry no content).
    pub declarations_recognised: u64,
    /// Model content that did not survive (Unmappable entries).
    pub content_losses: u64,
    /// Content carried imperfectly: dropped ids, names or flattened packages (Lossy entries).
    pub lossy: u64,
    /// The verdict: is any model content lost?
    pub verdict: ImportVerdict,
    /// True when the document contains no model elements at all.
    pub no_model_content: bool,
}

impl ImportSummary {
    /// Build a summary from the raw import tallies.
    pub fn new(
        elements_imported: u64,
        declarations_recognised: u64,
        content_losses: u64,
        lossy: u64,
    ) -> Self {
        let verdict = if content_losses == 0 {
            ImportVerdict::Lossless
        } else {
            ImportVerdict::ContentLosses
        };
        let no_model_content = elements_imported == 0;
        ImportSummary {
            elements_imported,
            declarations_recognised,
            content_losses,
            lossy,
            verdict,
            no_model_content,
        }
    }

    /// The sentence a human reads first.
    pub fn statement(&self) -> String {
        if self.no_model_content {
            if self.declarations_recognised > 0 {
                format!(
                    "This document declares profiles and imports but contains no elements — {} declaration(s) recognised, no content lost.",
                    self.declarations_recognised
                )
            } else {
                "This document contains no elements.".to_string()
            }
        } else if self.content_losses == 0 {
            format!(
                "{} element(s) imported with no content lost ({} declaration(s) recognised).",
                self.elements_imported, self.declarations_recognised
            )
        } else {
            format!(
                "{} element(s) imported; {} content loss(es) named below; {} declaration(s) recognised.",
                self.elements_imported, self.content_losses, self.declarations_recognised
            )
        }
    }

    /// The full human-first summary, with the statement leading and the detailed
    /// list left to follow it.
    pub fn render(&self) -> String {
        let verdict = match self.verdict {
            ImportVerdict::Lossless => "LOSSLESS — no model content was lost",
            ImportVerdict::ContentLosses => "CONTENT LOSSES — some model content did not survive",
        };
        format!(
            "{}
  elements imported       : {}
  declarations recognised : {}
  content losses          : {}
  lossy id/name drops     : {}
  verdict                 : {}",
            self.statement(),
            self.elements_imported,
            self.declarations_recognised,
            self.content_losses,
            self.lossy,
            verdict,
        )
    }
}
