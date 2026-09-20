// SPDX-License-Identifier: AGPL-3.0-or-later
//! The SysML v2 textual-notation binding: reads the stated subset into OKF.
//!
//! This is a VIEWER, not a round-trippable binding: it reads .sysml text into
//! OKF but does not write .sysml back out. Its metadata says so
//! (Direction::ImportOnly) and its export method refuses rather than pretend.
//!
//! The subset is declared as data in [crate::model::mapping_table]. Every
//! construct outside that subset is reported as an Unmappable mapping naming the
//! construct and its qualified name, never dropped in silence. A malformed
//! document is a clean BindingError::Import, never a panic.
//!
//! The reader is a hand-written recursive-descent parser, not a parser
//! generator. The subset is small enough that a hand-written parser is simpler
//! and more honest than a parser-generator dependency, and it keeps the engine
//! floor (rust-version 1.75) untouched: no new dependency is introduced.

pub mod model;

use std::collections::HashMap;

use binding::{
    artifact_hash, Binding, BindingError, BindingInfo, Direction, LossReport, Mapping,
    MappingVerdict,
};
use okf::types::{
    Attribute, Element, Graph, GraphEdge, GraphNode, OkfRoot, Requirement, StateMachine, Summary,
};

/// The binding identity the gate and the workbench use to select this reader.
pub const BINDING_ID: &str = "sysml-v2-textual";
/// The SysML v2 version this binding reads.
pub const BINDING_VERSION: &str = "1.0";

/// The SysML v2 textual-notation binding.
#[derive(Debug, Clone, Default)]
pub struct SysmlV2Binding;

impl SysmlV2Binding {
    pub fn new() -> Self {
        SysmlV2Binding
    }
}

impl Binding for SysmlV2Binding {
    fn info(&self) -> BindingInfo {
        BindingInfo {
            id: BINDING_ID.to_string(),
            version: BINDING_VERSION.to_string(),
            direction: Direction::ImportOnly,
            description: "SysML v2 textual notation (.sysml): part/attribute/item definitions, requirements, satisfy traceability and documentation"
                .to_string(),
        }
    }

    fn mapping_table(&self) -> Vec<Mapping> {
        model::mapping_table()
    }

    fn import(&self, source: &[u8]) -> Result<(OkfRoot, LossReport), BindingError> {
        let text = std::str::from_utf8(source).map_err(|e| {
            BindingError::Import(format!("SysML v2 source is not valid UTF-8: {e}"))
        })?;
        let importer = Importer::new(self.info(), artifact_hash(source));
        importer.run(text)
    }

    fn export(&self, _root: &OkfRoot) -> Result<Vec<u8>, BindingError> {
        // This binding is a viewer: it reads the textual notation but does not
        // write it back. Declaring Direction::ImportOnly in info() is the
        // metadata half of that promise; this error is the behavioural half.
        Err(BindingError::Export(
            "sysml-v2-textual is a viewer (ImportOnly): it reads the textual notation into OKF but cannot write it back"
                .to_string(),
        ))
    }
}

/// A feature of a definition (an attribute, part, item, reference or asserted
/// constraint). Carried as an OKF Attribute, plus - for part/item/ref - a graph
/// edge to the feature's type.
struct Feature {
    name: String,
    /// The clean qualified type name, used to resolve the graph edge target.
    type_ref: String,
    /// The multiplicity text (e.g. "1..*"), folded into the attribute's type.
    mult: String,
    default: String,
    /// "part" or "reference" when this feature also emits a graph edge.
    edge: Option<&'static str>,
    /// The OKF aggregation: "composite" for part/item, "none" otherwise.
    aggregation: String,
}

/// A carried definition (part def / attribute def / item def).
struct Definition {
    id: String,
    name: String,
    stereotype: String,
    /// The specialization base's qualified name, if any.
    base: String,
    features: Vec<Feature>,
    documentation: String,
}

/// A carried requirement.
struct Req {
    id: String,
    name: String,
    req_id: String,
    documentation: String,
    attributes: Vec<Attribute>,
    /// (type_ref, redefines, subject_name)
    subjects: Vec<(String, bool, String)>,
}

/// A satisfy relationship, with both endpoints as written (resolved later).
struct Satisfy {
    req: String,
    subject: String,
}

struct Importer {
    info: BindingInfo,
    artifact_hash: String,
    src: Vec<char>,
    pos: usize,
    project: String,
    /// The current package path, outermost first.
    path: Vec<String>,
    definitions: Vec<Definition>,
    requirements: Vec<Req>,
    satisfies: Vec<Satisfy>,
    losses: Vec<Mapping>,
}

fn import_err(msg: impl Into<String>) -> BindingError {
    BindingError::Import(msg.into())
}

impl Importer {
    fn new(info: BindingInfo, artifact_hash: String) -> Self {
        Importer {
            info,
            artifact_hash,
            src: Vec::new(),
            pos: 0,
            project: String::new(),
            path: Vec::new(),
            definitions: Vec::new(),
            requirements: Vec::new(),
            satisfies: Vec::new(),
            losses: Vec::new(),
        }
    }

    fn run(mut self, text: &str) -> Result<(OkfRoot, LossReport), BindingError> {
        self.src = text.chars().collect();
        self.pos = 0;
        loop {
            self.skip_trivia()?;
            if self.at_eof() {
                break;
            }
            if self.starts_with("package") {
                self.consume("package");
                self.parse_package()?;
            } else if self.peek_char() == Some('#') {
                let kw = self.read_hash_word();
                self.report_and_skip_statement(
                    &kw,
                    Some("top-level extension declaration; outside the subset"),
                )?;
            } else {
                let kw = self.read_word().unwrap_or_default();
                self.report_and_skip_statement(
                    &kw,
                    Some("top-level statement; expected a package"),
                )?;
            }
        }
        if self.project.is_empty() {
            return Err(import_err("SysML v2 document has no package"));
        }
        self.build()
    }

    // --- lexing helpers -----------------------------------------------------

    fn at_eof(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn peek_char(&self) -> Option<char> {
        self.src.get(self.pos).copied()
    }

    fn peek_n(&self, n: usize) -> Option<char> {
        self.src.get(self.pos + n).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek_char();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn starts_with(&self, s: &str) -> bool {
        let mut chars = s.chars();
        let mut i = self.pos;
        loop {
            match chars.next() {
                None => return true,
                Some(c) => match self.src.get(i) {
                    Some(&d) if d == c => i += 1,
                    _ => return false,
                },
            }
        }
    }

    fn consume(&mut self, s: &str) {
        for _ in s.chars() {
            self.bump();
        }
    }

    fn skip_ws_only(&mut self) {
        while let Some(c) = self.peek_char() {
            if c.is_whitespace() {
                self.bump();
            } else {
                break;
            }
        }
    }

    fn skip_trivia(&mut self) -> Result<(), BindingError> {
        loop {
            match self.peek_char() {
                Some(c) if c.is_whitespace() => {
                    self.bump();
                }
                Some('/') if self.peek_n(1) == Some('/') => {
                    while let Some(c) = self.peek_char() {
                        self.bump();
                        if c == '\n' {
                            break;
                        }
                    }
                }
                Some('/') if self.peek_n(1) == Some('*') => {
                    let _ = self.read_block_comment()?;
                }
                _ => return Ok(()),
            }
        }
    }

    /// Reads a block comment, returning the inner text (decoration not yet
    /// stripped). Handles both the '/* ... */' form and the '/** ... **/' doc form.
    fn read_block_comment(&mut self) -> Result<String, BindingError> {
        self.bump(); // '/'
        self.bump(); // '*'
        let doc_form = self.peek_char() == Some('*');
        if doc_form {
            self.bump(); // the leading '*' of '/**'
        }
        let mut out = String::new();
        loop {
            match self.peek_char() {
                None => return Err(import_err("unterminated block comment")),
                Some('*') => {
                    if self.peek_n(1) == Some('/') {
                        self.bump();
                        self.bump();
                        break;
                    }
                    if doc_form && self.peek_n(1) == Some('*') && self.peek_n(2) == Some('/') {
                        self.bump();
                        self.bump();
                        self.bump();
                        break;
                    }
                    out.push('*');
                    self.bump();
                }
                Some(c) => {
                    out.push(c);
                    self.bump();
                }
            }
        }
        Ok(out)
    }

    fn read_word(&mut self) -> Option<String> {
        let c = self.peek_char()?;
        if !(c.is_alphabetic() || c == '_') {
            return None;
        }
        let mut s = String::new();
        while let Some(c) = self.peek_char() {
            if c.is_alphanumeric() || c == '_' {
                s.push(c);
                self.bump();
            } else {
                break;
            }
        }
        Some(s)
    }

    fn peek_word(&self) -> Option<String> {
        let c = self.peek_char()?;
        if !(c.is_alphabetic() || c == '_') {
            return None;
        }
        let mut s = String::new();
        let mut i = self.pos;
        while let Some(&c) = self.src.get(i) {
            if c.is_alphanumeric() || c == '_' {
                s.push(c);
                i += 1;
            } else {
                break;
            }
        }
        Some(s)
    }

    fn read_quoted(&mut self) -> Result<String, BindingError> {
        let quote = self.bump().unwrap();
        let mut s = String::new();
        loop {
            match self.peek_char() {
                None => return Err(import_err("unterminated string literal")),
                Some(c) if c == quote => {
                    self.bump();
                    break;
                }
                Some('\\') => {
                    self.bump();
                    if let Some(c) = self.bump() {
                        s.push(c);
                    }
                }
                Some(c) => {
                    s.push(c);
                    self.bump();
                }
            }
        }
        Ok(s)
    }

    fn read_hash_word(&mut self) -> String {
        let mut s = String::new();
        s.push('#');
        self.bump(); // '#'
        while let Some(c) = self.peek_char() {
            if c.is_alphanumeric() || c == '_' {
                s.push(c);
                self.bump();
            } else {
                break;
            }
        }
        s
    }

    /// Reads a name: a quoted string or a bare identifier.
    fn read_name(&mut self) -> Option<String> {
        self.skip_ws_only();
        match self.peek_char() {
            Some('\'') | Some('"') => self.read_quoted().ok(),
            Some(c) if c.is_alphabetic() || c == '_' => self.read_word(),
            _ => None,
        }
    }

    /// Reads a '::'-qualified and/or '.'-dotted name (a type or reference,
    /// e.g. 'Drone_SystemArchitecture::drone.battery').
    fn read_qualified_name(&mut self) -> Option<String> {
        let mut out = self.read_name()?;
        loop {
            self.skip_ws_only();
            let sep = if self.starts_with("::") {
                "::"
            } else if self.starts_with(".") {
                "."
            } else {
                break;
            };
            self.consume(sep);
            self.skip_ws_only();
            let seg = self.read_name()?;
            out.push_str(sep);
            out.push_str(&seg);
        }
        Some(out)
    }

    /// Reads raw source until one of the terminator characters (at brace depth
    /// zero), trimming surrounding whitespace.
    fn read_raw_until(&mut self, terms: &[char]) -> String {
        let mut out = String::new();
        while let Some(c) = self.peek_char() {
            if terms.contains(&c) {
                break;
            }
            out.push(c);
            self.bump();
        }
        out.trim().to_string()
    }

    /// Reads the inner text of a balanced block, e.g. the body of a constraint.
    fn read_balanced(&mut self, open: char, close: char) -> Result<String, BindingError> {
        self.bump(); // open
        let mut depth = 1usize;
        let mut out = String::new();
        loop {
            match self.peek_char() {
                None => return Err(import_err("unterminated block")),
                Some(c) if c == open => {
                    depth += 1;
                    out.push(c);
                    self.bump();
                }
                Some(c) if c == close => {
                    depth -= 1;
                    if depth == 0 {
                        self.bump();
                        break;
                    }
                    out.push(c);
                    self.bump();
                }
                Some(c) => {
                    out.push(c);
                    self.bump();
                }
            }
        }
        Ok(out)
    }

    fn qualify(&self, name: &str) -> String {
        if self.path.is_empty() {
            name.to_string()
        } else {
            format!("{}::{}", self.path.join("::"), name)
        }
    }

    // --- parsing ------------------------------------------------------------

    fn parse_package(&mut self) -> Result<(), BindingError> {
        self.skip_trivia()?;
        let name = self
            .read_name()
            .ok_or_else(|| import_err("package without a name"))?;
        let is_root = self.path.is_empty();
        if is_root {
            self.project = name.clone();
        }
        self.path.push(name.clone());
        let qname = self.path.join("::");
        self.skip_trivia()?;
        if self.peek_char() == Some(';') {
            self.bump();
            self.record_package(&qname, is_root);
            self.path.pop();
            return Ok(());
        }
        if self.peek_char() == Some('{') {
            self.bump();
            self.parse_members()?;
            self.skip_trivia()?;
            if self.peek_char() == Some('}') {
                self.bump();
            }
            self.record_package(&qname, is_root);
            self.path.pop();
            return Ok(());
        }
        self.path.pop();
        Err(import_err(format!(
            "package {name}: body must be {{ ... }} or ;"
        )))
    }

    fn record_package(&mut self, qname: &str, is_root: bool) {
        if !is_root {
            self.losses.push(Mapping {
                subject: format!("package {qname}"),
                verdict: MappingVerdict::Lossy,
                note: "OKF has no package/namespace concept; members are promoted to the top-level structure and the package itself is dropped".to_string(),
            });
        }
    }

    fn parse_members(&mut self) -> Result<(), BindingError> {
        loop {
            self.skip_trivia()?;
            if self.at_eof() {
                return Err(import_err(
                    "unexpected end of document inside a package body",
                ));
            }
            if self.peek_char() == Some('}') {
                return Ok(());
            }
            self.parse_member()?;
        }
    }

    fn parse_member(&mut self) -> Result<(), BindingError> {
        self.skip_trivia()?;
        if self.peek_char() == Some('#') {
            let kw = self.read_hash_word();
            self.report_and_skip_statement(
                &kw,
                Some("extension (SYSMOD/custom) declaration; outside the subset"),
            )?;
            return Ok(());
        }
        let mut modifiers: Vec<String> = Vec::new();
        loop {
            self.skip_trivia()?;
            match self.peek_word() {
                Some(w) if w == "abstract" || w == "individual" => {
                    self.read_word();
                    modifiers.push(w);
                }
                _ => break,
            }
        }
        self.skip_trivia()?;
        let kw = self
            .read_word()
            .ok_or_else(|| import_err("expected a declaration"))?;
        match kw.as_str() {
            "package" => self.parse_package(),
            "part" | "attribute" | "item" => self.parse_def_or_usage(&kw, &modifiers),
            "requirement" => self.parse_requirement(),
            "satisfy" => self.parse_satisfy(),
            "import" => self.parse_import(),
            "doc" => {
                let raw = self.read_doc_comment()?;
                self.losses.push(Mapping {
                    subject: "doc comment".to_string(),
                    verdict: MappingVerdict::Lossy,
                    note: format!(
                        "package-level documentation not carried: OKF has no package element to hold it ({})",
                        normalize_doc(&raw)
                    ),
                });
                Ok(())
            }
            other => {
                let note = out_of_scope_note(other);
                self.report_and_skip_statement(other, note)
            }
        }
    }

    fn parse_def_or_usage(&mut self, kind: &str, modifiers: &[String]) -> Result<(), BindingError> {
        self.skip_trivia()?;
        if self.starts_with("def") {
            self.consume("def");
            self.parse_definition(kind, modifiers)
        } else {
            let name = self.read_name().unwrap_or_default();
            let subject = if name.is_empty() {
                kind.to_string()
            } else {
                format!("{kind} {name}")
            };
            self.losses.push(Mapping {
                subject,
                verdict: MappingVerdict::Unmappable,
                note: "top-level usage owned by a package; outside the subset — only definitions and requirements are carried".to_string(),
            });
            self.skip_statement()?;
            Ok(())
        }
    }

    fn parse_definition(&mut self, kind: &str, modifiers: &[String]) -> Result<(), BindingError> {
        self.skip_trivia()?;
        let name = self
            .read_name()
            .ok_or_else(|| import_err(format!("{kind} def without a name")))?;
        let id = self.qualify(&name);
        let stereotype = stereotype_for(kind, modifiers);
        self.skip_trivia()?;
        let mut base = String::new();
        if self.starts_with(":>") {
            self.consume(":>");
            self.skip_trivia()?;
            base = self.read_qualified_name().unwrap_or_default();
        } else if self.starts_with("specializes") {
            self.consume("specializes");
            self.skip_trivia()?;
            base = self.read_qualified_name().unwrap_or_default();
        }
        let idx = self.definitions.len();
        self.definitions.push(Definition {
            id: id.clone(),
            name: name.clone(),
            stereotype,
            base,
            features: Vec::new(),
            documentation: String::new(),
        });
        self.skip_trivia()?;
        if self.peek_char() == Some(';') {
            self.bump();
            return Ok(());
        }
        if self.peek_char() == Some('{') {
            self.bump();
            self.parse_definition_body(idx)?;
            return Ok(());
        }
        Err(import_err(format!(
            "{kind} def {name}: body must be {{ ... }} or ;"
        )))
    }

    fn parse_definition_body(&mut self, def_idx: usize) -> Result<(), BindingError> {
        loop {
            self.skip_trivia()?;
            if self.at_eof() {
                return Err(import_err("unterminated definition body"));
            }
            if self.peek_char() == Some('}') {
                self.bump();
                return Ok(());
            }
            self.parse_feature_member(def_idx)?;
        }
    }

    fn parse_feature_member(&mut self, def_idx: usize) -> Result<(), BindingError> {
        self.skip_trivia()?;
        let mut modifiers: Vec<String> = Vec::new();
        loop {
            match self.peek_word() {
                Some(w) if w == "in" || w == "out" => {
                    self.read_word();
                    modifiers.push(w);
                    self.skip_trivia()?;
                }
                _ => break,
            }
        }
        self.skip_trivia()?;
        let kw = self
            .read_word()
            .ok_or_else(|| import_err("expected a feature"))?;
        match kw.as_str() {
            "doc" => {
                let raw = self.read_doc_comment()?;
                let normalized = normalize_doc(&raw);
                let existing = self.definitions[def_idx].documentation.clone();
                self.definitions[def_idx].documentation = merge_doc(&existing, &normalized);
                Ok(())
            }
            "attribute" | "part" | "item" => self.parse_feature(def_idx, &kw, &modifiers),
            "ref" => {
                self.skip_trivia()?;
                if let Some(w) = self.peek_word() {
                    if w == "item" || w == "individual" {
                        self.read_word();
                    }
                }
                self.parse_feature(def_idx, "ref", &modifiers)
            }
            "assert" => self.parse_constraint_into(def_idx),
            other => {
                let note = out_of_scope_note(other);
                self.report_and_skip_statement(other, note)
            }
        }
    }

    fn parse_feature(
        &mut self,
        def_idx: usize,
        kind: &str,
        modifiers: &[String],
    ) -> Result<(), BindingError> {
        let direction = modifiers
            .iter()
            .find(|m| **m == "in" || **m == "out")
            .cloned();
        self.skip_trivia()?;
        if self.starts_with("redefines") {
            self.consume("redefines");
        }
        self.skip_trivia()?;
        let name = self
            .read_name()
            .ok_or_else(|| import_err(format!("{kind} without a name")))?;
        self.skip_trivia()?;
        let mut mult_after_name = String::new();
        if self.peek_char() == Some('[') {
            mult_after_name = self.read_mult()?;
        }
        self.skip_trivia()?;
        let mut type_ref = String::new();
        let mut mult_after_type = String::new();
        if self.starts_with(":") {
            self.consume(":");
            self.skip_trivia()?;
            type_ref = self.read_qualified_name().unwrap_or_default();
            self.skip_trivia()?;
            if self.peek_char() == Some('[') {
                mult_after_type = self.read_mult()?;
            }
        }
        self.skip_trivia()?;
        let mut default = String::new();
        if self.starts_with("=") {
            self.consume("=");
            self.skip_trivia()?;
            default = collapse_ws(&self.read_raw_until(&[';']));
        }
        self.skip_trivia()?;
        if self.peek_char() == Some(';') {
            self.bump();
        }
        let mult = if !mult_after_type.is_empty() {
            mult_after_type
        } else {
            mult_after_name
        };
        let (edge, aggregation) = match kind {
            "part" | "item" => (Some("part"), "composite"),
            "ref" => (Some("reference"), "none"),
            _ => (None, "none"),
        };
        self.definitions[def_idx].features.push(Feature {
            name: name.clone(),
            type_ref: type_ref.clone(),
            mult,
            default,
            edge,
            aggregation: aggregation.to_string(),
        });
        if let Some(d) = direction {
            self.losses.push(Mapping {
                subject: format!("{d} {name}"),
                verdict: MappingVerdict::Lossy,
                note: format!("direction '{d}' dropped: OKF Attribute has no direction slot"),
            });
        }
        Ok(())
    }

    fn read_mult(&mut self) -> Result<String, BindingError> {
        let raw = self.read_balanced('[', ']')?;
        Ok(raw.chars().filter(|c| !c.is_whitespace()).collect())
    }

    fn parse_constraint_into(&mut self, def_idx: usize) -> Result<(), BindingError> {
        self.skip_trivia()?;
        if self.starts_with("constraint") {
            self.consume("constraint");
        }
        self.skip_trivia()?;
        let body = self.read_balanced('{', '}')?;
        let text = collapse_ws(&body);
        self.definitions[def_idx].features.push(Feature {
            name: "constraint".to_string(),
            type_ref: "ConstraintUsage".to_string(),
            mult: String::new(),
            default: text,
            edge: None,
            aggregation: "none".to_string(),
        });
        Ok(())
    }

    fn parse_requirement(&mut self) -> Result<(), BindingError> {
        self.skip_trivia()?;
        let mut req_id = String::new();
        if self.peek_char() == Some('<') {
            self.bump();
            self.skip_ws_only();
            // The id is a quoted name, e.g. <'REQ-42'>; fall back to raw text
            // for an unquoted id.
            req_id = match self.peek_char() {
                Some('\'') | Some('"') => self.read_quoted().unwrap_or_default(),
                _ => self.read_raw_until(&['>']).to_string(),
            };
            self.skip_ws_only();
            if self.peek_char() == Some('>') {
                self.bump();
            }
        }
        self.skip_trivia()?;
        let name = self
            .read_name()
            .ok_or_else(|| import_err("requirement without a name"))?;
        let id = self.qualify(&name);
        let idx = self.requirements.len();
        self.requirements.push(Req {
            id,
            name: name.clone(),
            req_id,
            documentation: String::new(),
            attributes: Vec::new(),
            subjects: Vec::new(),
        });
        self.skip_trivia()?;
        if self.peek_char() == Some(';') {
            self.bump();
            return Ok(());
        }
        if self.peek_char() == Some('{') {
            self.bump();
            self.parse_requirement_body(idx)?;
            return Ok(());
        }
        Err(import_err(format!(
            "requirement {name}: body must be {{ ... }} or ;"
        )))
    }

    fn parse_requirement_body(&mut self, req_idx: usize) -> Result<(), BindingError> {
        loop {
            self.skip_trivia()?;
            if self.at_eof() {
                return Err(import_err("unterminated requirement body"));
            }
            if self.peek_char() == Some('}') {
                self.bump();
                return Ok(());
            }
            self.parse_req_member(req_idx)?;
        }
    }

    fn parse_req_member(&mut self, req_idx: usize) -> Result<(), BindingError> {
        self.skip_trivia()?;
        let kw = self
            .read_word()
            .ok_or_else(|| import_err("expected a requirement member"))?;
        match kw.as_str() {
            "doc" => {
                let raw = self.read_doc_comment()?;
                let normalized = normalize_doc(&raw);
                let existing = self.requirements[req_idx].documentation.clone();
                self.requirements[req_idx].documentation = merge_doc(&existing, &normalized);
                Ok(())
            }
            "subject" => self.parse_subject(req_idx),
            "assert" => self.parse_constraint_into_req(req_idx),
            other => {
                let note = out_of_scope_note(other);
                self.report_and_skip_statement(other, note)
            }
        }
    }

    fn parse_subject(&mut self, req_idx: usize) -> Result<(), BindingError> {
        self.skip_trivia()?;
        let name = self
            .read_name()
            .ok_or_else(|| import_err("subject without a name"))?;
        self.skip_trivia()?;
        let mut redefines = false;
        if self.starts_with(":>>") {
            self.consume(":>>");
            redefines = true;
        } else if self.starts_with(":>") {
            self.consume(":>");
            redefines = true;
        } else if self.starts_with(":") {
            self.consume(":");
        } else {
            return Err(import_err(format!("subject {name}: missing ':' or ':>>'")));
        }
        self.skip_trivia()?;
        let type_ref = self.read_qualified_name().unwrap_or_default();
        self.skip_trivia()?;
        if self.peek_char() == Some(';') {
            self.bump();
        }
        self.requirements[req_idx]
            .subjects
            .push((type_ref, redefines, name));
        Ok(())
    }

    fn parse_constraint_into_req(&mut self, req_idx: usize) -> Result<(), BindingError> {
        self.skip_trivia()?;
        if self.starts_with("constraint") {
            self.consume("constraint");
        }
        self.skip_trivia()?;
        let body = self.read_balanced('{', '}')?;
        let text = collapse_ws(&body);
        self.requirements[req_idx].attributes.push(Attribute {
            name: "constraint".to_string(),
            attr_type: "ConstraintUsage".to_string(),
            aggregation: String::new(),
            default: text,
        });
        Ok(())
    }

    fn parse_satisfy(&mut self) -> Result<(), BindingError> {
        self.skip_trivia()?;
        let req = self
            .read_qualified_name()
            .ok_or_else(|| import_err("satisfy without a requirement"))?;
        self.skip_trivia()?;
        if self.starts_with("by") {
            self.consume("by");
        } else {
            return Err(import_err("satisfy without 'by'"));
        }
        self.skip_trivia()?;
        let subject = self
            .read_qualified_name()
            .ok_or_else(|| import_err("satisfy without a subject"))?;
        self.skip_trivia()?;
        if self.peek_char() == Some(';') {
            self.bump();
        }
        self.satisfies.push(Satisfy { req, subject });
        Ok(())
    }

    fn parse_import(&mut self) -> Result<(), BindingError> {
        self.skip_trivia()?;
        let path = self.read_raw_until(&[';']).to_string();
        if self.peek_char() == Some(';') {
            self.bump();
        }
        self.losses.push(Mapping {
            subject: format!("import {path}"),
            verdict: MappingVerdict::Exact,
            note: "declaration: import recognised and not carried — it carries no model content"
                .to_string(),
        });
        Ok(())
    }

    fn read_doc_comment(&mut self) -> Result<String, BindingError> {
        self.skip_ws_only();
        if self.starts_with("/*") {
            Ok(self.read_block_comment()?)
        } else {
            Err(import_err("'doc' keyword not followed by a block comment"))
        }
    }

    fn report_and_skip_statement(
        &mut self,
        kw: &str,
        note: Option<&str>,
    ) -> Result<(), BindingError> {
        self.skip_trivia()?;
        let name = self.read_name().unwrap_or_default();
        let subject = if name.is_empty() {
            kw.to_string()
        } else {
            format!("{kw} {name}")
        };
        let note = note
            .map(|n| n.to_string())
            .unwrap_or_else(|| "outside the sysml-v2-textual subset; not carried".to_string());
        self.losses.push(Mapping {
            subject,
            verdict: MappingVerdict::Unmappable,
            note,
        });
        self.skip_statement()
    }

    fn skip_statement(&mut self) -> Result<(), BindingError> {
        self.skip_trivia()?;
        let mut depth = 0usize;
        let mut saw_block = false;
        loop {
            match self.peek_char() {
                None => return Ok(()),
                Some('{') => {
                    depth += 1;
                    saw_block = true;
                    self.bump();
                }
                Some('}') => {
                    if depth == 0 {
                        return Ok(());
                    }
                    depth -= 1;
                    self.bump();
                    if depth == 0 && saw_block {
                        return Ok(());
                    }
                }
                Some(';') if depth == 0 => {
                    self.bump();
                    return Ok(());
                }
                Some('\'') | Some('"') => {
                    let _ = self.read_quoted()?;
                }
                Some('/') if self.peek_n(1) == Some('/') => {
                    while let Some(c) = self.peek_char() {
                        self.bump();
                        if c == '\n' {
                            break;
                        }
                    }
                }
                Some('/') if self.peek_n(1) == Some('*') => {
                    let _ = self.read_block_comment()?;
                }
                Some(_) => {
                    self.bump();
                }
            }
        }
    }

    // --- assembly -----------------------------------------------------------

    fn build(mut self) -> Result<(OkfRoot, LossReport), BindingError> {
        let mut id_to_name: HashMap<String, String> = HashMap::new();
        let mut short_to_ids: HashMap<String, Vec<String>> = HashMap::new();
        for d in &self.definitions {
            id_to_name.insert(d.id.clone(), d.name.clone());
            short_to_ids
                .entry(d.name.clone())
                .or_default()
                .push(d.id.clone());
        }
        for r in &self.requirements {
            id_to_name.insert(r.id.clone(), r.name.clone());
            short_to_ids
                .entry(r.name.clone())
                .or_default()
                .push(r.id.clone());
        }

        let mut structure: Vec<Element> = Vec::new();
        let mut requirements: Vec<Requirement> = Vec::new();
        let mut graph_nodes: Vec<GraphNode> = Vec::new();
        let mut graph_edges: Vec<GraphEdge> = Vec::new();

        for d in &self.definitions {
            let attributes = d
                .features
                .iter()
                .map(|f| Attribute {
                    name: f.name.clone(),
                    attr_type: fold_type(&f.type_ref, &f.mult),
                    aggregation: f.aggregation.clone(),
                    default: f.default.clone(),
                })
                .collect();
            structure.push(Element {
                id: d.id.clone(),
                name: d.name.clone(),
                kind: "block".to_string(),
                stereotypes: vec![d.stereotype.clone()],
                attributes,
                documentation: d.documentation.clone(),
            });
            graph_nodes.push(GraphNode {
                id: d.id.clone(),
                kind: "block".to_string(),
                name: d.name.clone(),
                stereotypes: vec![d.stereotype.clone()],
            });

            if !d.base.is_empty() {
                match resolve(&d.base, &id_to_name, &short_to_ids) {
                    Some(t) => graph_edges.push(GraphEdge {
                        source: d.id.clone(),
                        target: t,
                        kind: "generalization".to_string(),
                        label: String::new(),
                    }),
                    None => self.losses.push(Mapping {
                        subject: format!("{} :> {}", d.id, d.base),
                        verdict: MappingVerdict::Unmappable,
                        note: "specialization base does not resolve to a carried element"
                            .to_string(),
                    }),
                }
            }
            for f in &d.features {
                if let Some(edge_kind) = f.edge {
                    if f.type_ref.is_empty() {
                        continue;
                    }
                    match resolve(&f.type_ref, &id_to_name, &short_to_ids) {
                        Some(t) => graph_edges.push(GraphEdge {
                            source: d.id.clone(),
                            target: t,
                            kind: edge_kind.to_string(),
                            label: String::new(),
                        }),
                        None => self.losses.push(Mapping {
                            subject: format!("{edge_kind} {}", f.name),
                            verdict: MappingVerdict::Lossy,
                            note: format!(
                                "{edge_kind} '{}' type '{}' is not a carried element in this document; no {edge_kind} edge emitted (the attribute still carries the type)",
                                f.name, f.type_ref
                            ),
                        }),
                    }
                }
            }
        }

        for r in &self.requirements {
            requirements.push(Requirement {
                id: r.id.clone(),
                name: r.name.clone(),
                kind: "requirement".to_string(),
                stereotypes: vec!["requirement".to_string()],
                attributes: r.attributes.clone(),
                documentation: r.documentation.clone(),
                req_id: r.req_id.clone(),
                req_text: String::new(),
            });
            graph_nodes.push(GraphNode {
                id: r.id.clone(),
                kind: "requirement".to_string(),
                name: r.name.clone(),
                stereotypes: vec!["requirement".to_string()],
            });
            for (type_ref, redefines, subj_name) in &r.subjects {
                match resolve(type_ref, &id_to_name, &short_to_ids) {
                    Some(t) => {
                        graph_edges.push(GraphEdge {
                            source: r.id.clone(),
                            target: t,
                            kind: "subject".to_string(),
                            label: subj_name.clone(),
                        });
                        if *redefines {
                            self.losses.push(Mapping {
                                subject: format!("subject {subj_name} :>> {type_ref}"),
                                verdict: MappingVerdict::Lossy,
                                note: "redefinition subject carried as a subject edge; redefinition semantics not reconstructed".to_string(),
                            });
                        }
                    }
                    None => self.losses.push(Mapping {
                        subject: format!("subject {subj_name} : {type_ref}"),
                        verdict: MappingVerdict::Unmappable,
                        note: if *redefines {
                            "redefinition subject type does not resolve to a carried element"
                                .to_string()
                        } else {
                            "subject type does not resolve to a carried element".to_string()
                        },
                    }),
                }
            }
        }

        for s in &self.satisfies {
            let req_res = resolve(&s.req, &id_to_name, &short_to_ids);
            let subj_res = resolve(&s.subject, &id_to_name, &short_to_ids);
            match (req_res, subj_res) {
                (Some(r), Some(su)) => graph_edges.push(GraphEdge {
                    source: su,
                    target: r,
                    kind: "dependency".to_string(),
                    label: "satisfy".to_string(),
                }),
                _ => self.losses.push(Mapping {
                    subject: format!("satisfy {} by {}", s.req, s.subject),
                    verdict: MappingVerdict::Unmappable,
                    note: "satisfy relationship: an endpoint is outside the carried subset"
                        .to_string(),
                }),
            }
        }

        let summary = Summary {
            blocks: structure.len() as u64,
            requirements: requirements.len() as u64,
            interfaces: 0,
            signals: 0,
            activities: 0,
            graph_nodes: graph_nodes.len() as u64,
            graph_edges: graph_edges.len() as u64,
        };

        let root = OkfRoot {
            okf: "1.0".to_string(),
            project: self.project.clone(),
            exported_at: String::new(),
            summary,
            structure,
            interfaces: Vec::new(),
            signals: Vec::new(),
            requirements,
            state_machine: Some(StateMachine {
                name: "stateMachine".to_string(),
                regions: Vec::new(),
            }),
            activities: Vec::new(),
            graph: Some(Graph {
                nodes: graph_nodes,
                edges: graph_edges,
            }),
            provenance: None,
            references: Vec::new(),
        };

        let report = LossReport {
            binding: self.info,
            mappings: self.losses,
            artifact_hash: self.artifact_hash,
        };
        Ok((root, report))
    }
}

/// Resolve a name (qualified or short) to a carried element id.
fn resolve(
    name: &str,
    id_to_name: &HashMap<String, String>,
    short_to_ids: &HashMap<String, Vec<String>>,
) -> Option<String> {
    if id_to_name.contains_key(name) {
        return Some(name.to_string());
    }
    if name.contains("::") {
        let suffix = format!("::{name}");
        let matches: Vec<&String> = id_to_name
            .keys()
            .filter(|id| id.ends_with(&suffix))
            .collect();
        if matches.len() == 1 {
            return Some(matches[0].clone());
        }
        return None;
    }
    if let Some(ids) = short_to_ids.get(name) {
        if ids.len() == 1 {
            return Some(ids[0].clone());
        }
    }
    None
}

fn fold_type(type_ref: &str, mult: &str) -> String {
    match (type_ref.is_empty(), mult.is_empty()) {
        (true, true) => String::new(),
        (false, true) => type_ref.to_string(),
        (true, false) => format!("[{mult}]"),
        (false, false) => format!("{type_ref}[{mult}]"),
    }
}

fn stereotype_for(kind: &str, modifiers: &[String]) -> String {
    let has = |m: &str| modifiers.iter().any(|x| x == m);
    let base = match kind {
        "part" => "partDef",
        "attribute" => "attributeDef",
        "item" => "itemDef",
        _ => "def",
    };
    let capitalized = {
        let mut c = base.chars();
        let first = c.next().unwrap().to_uppercase().collect::<String>();
        first + c.as_str()
    };
    if has("abstract") {
        format!("abstract{capitalized}")
    } else if has("individual") {
        format!("individual{capitalized}")
    } else {
        base.to_string()
    }
}

fn merge_doc(a: &str, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => String::new(),
        (true, false) => b.to_string(),
        (false, true) => a.to_string(),
        (false, false) => format!("{a} {b}"),
    }
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::new();
    let mut in_ws = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !in_ws {
                out.push(' ');
                in_ws = true;
            }
        } else {
            out.push(c);
            in_ws = false;
        }
    }
    out.trim().to_string()
}

/// Normalise a doc comment: strip the per-line '*' decoration and collapse
/// whitespace to single spaces.
fn normalize_doc(raw: &str) -> String {
    let mut out = String::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        let stripped = trimmed.trim_start_matches('*').trim();
        if !stripped.is_empty() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(stripped);
        }
    }
    out
}

fn out_of_scope_note(kw: &str) -> Option<&'static str> {
    match kw {
        "calc" => Some("calculation (calc def / calc); outside the subset — not carried"),
        "state" => Some("state definition; outside the subset — not carried"),
        "transition" => Some("state transition; outside the subset — not carried"),
        "action" => Some("action definition; outside the subset — not carried"),
        "actor" => Some("actor; outside the subset — not carried"),
        "usecase" => Some("use case; outside the subset — not carried"),
        "association" => Some("association; outside the subset — not carried"),
        "boundary" => Some("use-case boundary; outside the subset — not carried"),
        "include" => Some("use-case include; outside the subset — not carried"),
        "enum" => Some("enumeration; outside the subset — not carried"),
        "variation" => Some("variation point; outside the subset — not carried"),
        "variant" => Some("variant; outside the subset — not carried"),
        "timeslice" => Some("time slice; outside the subset — not carried"),
        "metadata" => Some("metadata; outside the subset — not carried"),
        _ => None,
    }
}
