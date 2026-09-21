// SPDX-License-Identifier: AGPL-3.0-or-later
//! The drop-zone front door (E1): drop a model, and be told what it is in plain language.
//!
//! The projects page used to open on a list and an empty "New project" box, which asks the
//! person this product is for - an engineer with a 36 MB Cameo export and a review next
//! month - to do the hard parts themselves: know which binding to pick, create a project,
//! upload, read a 50,784-row loss report and decide what to accept. This module is the front
//! door instead: one drop target, no format choice, no project to create first.
//!
//! The flow is detect -> create -> retain -> import -> say what happened:
//!
//! 1. the dropped bytes are sniffed (and the file name read) to pick a binding, so nobody has
//!    to know that a MagicDraw export is "sysml-v1-xmi@2.4";
//! 2. a project is created named after the file (or reused when it already exists);
//! 3. the artifact is retained content-addressed, streamed to a staged file first so a large
//!    vendor export is never held in memory twice;
//! 4. the import runs through the SAME `crate::binding_api::import_core` the JSON endpoint and
//!    the import page call, and an acceptance through the SAME
//!    `crate::binding_api::accept_import_core`, so the front door can never become a weaker or
//!    different path to the data;
//! 5. the result page states, in plain language, what the ENGINE measured.
//!
//! ## The rule this module exists to keep
//!
//! **The LLM authors and interprets; the engine measures.** Every number and every count in
//! the plain-language summary is read from an engine value: the binding's own loss report, the
//! engine's round-trip diff, or the graph and coverage the imported document produced. Nothing
//! here counts anything the engine did not count, and nothing is estimated. The sentences are
//! TEMPLATES over an `OnboardFacts` value, so the same facts render with or without a
//! configured reasoner - the public showcase runs in open mode with no assist, and it must show
//! exactly the numbers a deployment with a reasoner would show.
//!
//! The page is server-rendered and the drop zone is a plain multipart file upload, so the whole
//! flow completes with JavaScript disabled; the inline script only adds drag-and-drop on top of
//! the same file input.

use std::collections::BTreeMap;
use std::io::Write as _;

use axum::extract::{Multipart, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use maud::{html, Markup, PreEscaped};

use binding::{BindingInfo, Direction, LossReport, Mapping};
use graph::{graph_stats, requirement_coverage};
use okf::types::OkfRoot;

use crate::api::{
    create_project_core, load_model, map_store_error, validate_name, verify_actor, ApiState,
};
use crate::auth::{identity as resolve_identity, Identity, Permission};
use crate::binding_api::{accept_import_core, import_core, ImportCore, ImportOutcome};
use crate::binding_registry;
use crate::error::ApiError;
use crate::store::{Commit, Store};
use crate::ui::import as import_ui;
use crate::ui::layout;

/// The branch an onboarded model lands on, matching the model page's default.
const ONBOARD_BRANCH: &str = "main";
/// How many loss classes the plain-language summary names. The requirement is the five
/// biggest; the count is a presentation limit, never a number this module measures.
const MAX_LOSS_CLASSES: usize = 5;
/// How much of a dropped artifact is held in memory for format detection. The markers that
/// decide the format all appear in the document's own header, so a bounded read is enough and
/// a 36 MB export is never buffered just to be identified.
const SNIFF_BYTES: usize = 64 * 1024;

// ---------------------------------------------------------------------------
// Detection: what is this file, and therefore which binding reads it.

/// The format a dropped artifact was detected as, decided from the BYTES first and the file
/// name only as a tie-break. The label is what the result page says in words; the binding is
/// what the registry resolves for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Format {
    /// A SysML v1 / UML XMI export: MagicDraw, Cameo, any XMI 2.x writer.
    Xmi,
    /// A Cameo/MagicDraw `.mdzip` container, which is a zip holding the XMI export.
    Cameo,
    /// SysML v2 textual notation.
    SysmlV2,
    /// Nothing this platform reads, with what was seen instead.
    Unknown(String),
}

impl Format {
    /// The plain-language name of the detected format, for the summary.
    pub fn label(&self) -> &'static str {
        match self {
            Format::Xmi => {
                "a SysML v1 XMI export (the form MagicDraw, Cameo and most UML/SysML tools write)"
            }
            Format::Cameo => "a Cameo/MagicDraw .mdzip container",
            Format::SysmlV2 => "SysML v2 textual notation",
            Format::Unknown(_) => "not a format this server reads",
        }
    }

    /// The binding selector id@version the registry resolves for this format, or None when the
    /// format is not one this platform reads.
    fn binding(&self) -> Option<String> {
        match self {
            Format::Xmi | Format::Cameo => Some(format!(
                "{}@{}",
                binding_xmi::BINDING_ID,
                binding_xmi::BINDING_VERSION
            )),
            Format::SysmlV2 => Some(format!(
                "{}@{}",
                binding_sysmlv2::BINDING_ID,
                binding_sysmlv2::BINDING_VERSION
            )),
            Format::Unknown(_) => None,
        }
    }
}

/// Whether the head of an artifact carries the markers of an XMI document. Checked on the
/// bytes, not the extension: a MagicDraw export renamed to .xml is still an XMI export, and a
/// .xmi file that is not XMI must not be claimed as one.
fn looks_like_xmi(head: &[u8]) -> bool {
    let text = String::from_utf8_lossy(head);
    text.contains("<xmi:XMI")
        || text.contains("xmlns:uml=")
        || text.contains("http://www.omg.org/spec/XMI")
}

/// Whether the head of an artifact carries the markers of SysML v2 textual notation. The
/// keywords are the notation's own declaration words; a .sysml name alone is enough to try the
/// reader, which refuses a file it cannot read rather than guessing.
fn looks_like_sysmlv2(head: &[u8], file_name: &str) -> bool {
    let text = String::from_utf8_lossy(head);
    const KEYWORDS: &[&str] = &[
        "package ",
        "part def ",
        "attribute def ",
        "item def ",
        "action def ",
        "port def ",
        "requirement def ",
        "requirement ",
    ];
    if KEYWORDS.iter().any(|keyword| text.contains(keyword)) {
        return true;
    }
    file_name.to_ascii_lowercase().ends_with(".sysml")
}

/// Detect the format of a dropped artifact from its head bytes and its file name. The order
/// matters: a zip is a container (never XMI text), XMI markers beat the SysML v2 keywords a
/// .sysml-named but XMI-bodied file would otherwise trip, and anything unrecognised is reported
/// as such rather than guessed at.
pub fn detect(head: &[u8], file_name: &str) -> Format {
    if is_zip(head) {
        return Format::Cameo;
    }
    if looks_like_xmi(head) {
        return Format::Xmi;
    }
    if looks_like_sysmlv2(head, file_name) {
        return Format::SysmlV2;
    }
    let lower = file_name.to_ascii_lowercase();
    let extension = lower.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
    match extension {
        "xmi" | "uml" => Format::Unknown(
            "the file name says XMI but the bytes do not carry an XMI document".to_string(),
        ),
        "mdzip" => Format::Unknown(
            "the file name says .mdzip but the bytes are not a zip container".to_string(),
        ),
        "" => Format::Unknown("the file has no name and no recognised content".to_string()),
        other => Format::Unknown(format!(
            "the .{} file is not a format this server reads",
            other
        )),
    }
}

/// The zip local-file-header signature every container starts with.
fn is_zip(head: &[u8]) -> bool {
    head.starts_with(b"PK\x03\x04")
}

// ---------------------------------------------------------------------------
// The Cameo container: a zip holding the XMI export.

/// One entry located in a zip container, with where its bytes are and how they are stored.
struct ZipEntry {
    name: String,
    method: u16,
    compressed_size: usize,
    uncompressed_size: usize,
    data_offset: usize,
}

fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
    let slice = bytes.get(at..at + 2)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// Find the End Of Central Directory record by scanning back from the end of the container.
/// The record is at most 22 bytes plus a 64 KiB comment, so the search is bounded.
fn end_of_central_directory(bytes: &[u8]) -> Option<usize> {
    const SIGNATURE: u32 = 0x0605_4b50;
    let len = bytes.len();
    let start = len.saturating_sub(22 + 65_535);
    (start..len.saturating_sub(3)).rev().find(|&at| {
        read_u32(bytes, at) == Some(SIGNATURE)
            && read_u16(bytes, at + 20).map(|comment| at + 22 + comment as usize) == Some(len)
    })
}

/// Read a zip container's central directory. This is the authority on what the container
/// holds: the local headers can lie about sizes when the writer used a data descriptor, the
/// central directory cannot.
fn zip_entries(bytes: &[u8]) -> Result<Vec<ZipEntry>, String> {
    const CENTRAL: u32 = 0x0201_4b50;
    let eocd = end_of_central_directory(bytes)
        .ok_or_else(|| "the .mdzip container has no end-of-central-directory record".to_string())?;
    let count = read_u16(bytes, eocd + 10).ok_or("the container is truncated")? as usize;
    let mut cursor = read_u32(bytes, eocd + 16).ok_or("the container is truncated")? as usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        if read_u32(bytes, cursor) != Some(CENTRAL) {
            return Err("the container's central directory is malformed".to_string());
        }
        let method = read_u16(bytes, cursor + 10).ok_or("the container is truncated")?;
        let compressed_size = read_u32(bytes, cursor + 20).ok_or("truncated")? as usize;
        let uncompressed_size = read_u32(bytes, cursor + 24).ok_or("truncated")? as usize;
        let name_len = read_u16(bytes, cursor + 28).ok_or("truncated")? as usize;
        let extra_len = read_u16(bytes, cursor + 30).ok_or("truncated")? as usize;
        let comment_len = read_u16(bytes, cursor + 32).ok_or("truncated")? as usize;
        let local = read_u32(bytes, cursor + 42).ok_or("truncated")? as usize;
        let name_bytes = bytes
            .get(cursor + 46..cursor + 46 + name_len)
            .ok_or("the container is truncated")?;
        let name = String::from_utf8_lossy(name_bytes).to_string();
        // The local header carries its own name and extra lengths, which are what sit between
        // the header and the entry's bytes.
        let local_name_len = read_u16(bytes, local + 26).ok_or("truncated")? as usize;
        let local_extra_len = read_u16(bytes, local + 28).ok_or("truncated")? as usize;
        let data_offset = local + 30 + local_name_len + local_extra_len;
        entries.push(ZipEntry {
            name,
            method,
            compressed_size,
            uncompressed_size,
            data_offset,
        });
        cursor += 46 + name_len + extra_len + comment_len;
    }
    Ok(entries)
}

// ---------------------------------------------------------------------------
// DEFLATE (RFC 1951): unpacking what a real Cameo container holds.

/// A reader of the DEFLATE bit stream. Bits arrive least-significant-bit first, which is the
/// format's own convention and the opposite of how the bytes are written down.
struct BitReader<'a> {
    data: &'a [u8],
    byte: usize,
    bit: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        BitReader {
            data,
            byte: 0,
            bit: 0,
        }
    }

    /// One bit, least-significant first.
    fn bit(&mut self) -> Result<u32, String> {
        if self.byte >= self.data.len() {
            return Err("the deflate stream ended in the middle of a code".to_string());
        }
        let value = ((self.data[self.byte] >> self.bit) & 1) as u32;
        self.bit += 1;
        if self.bit == 8 {
            self.bit = 0;
            self.byte += 1;
        }
        Ok(value)
    }

    /// `count` bits, least-significant first, assembled into an integer.
    fn bits(&mut self, count: u32) -> Result<u32, String> {
        let mut value = 0u32;
        for position in 0..count {
            value |= self.bit()? << position;
        }
        Ok(value)
    }

    /// Skip to the next byte boundary, which a stored block requires.
    fn align(&mut self) {
        if self.bit != 0 {
            self.bit = 0;
            self.byte += 1;
        }
    }
}

/// A canonical Huffman decoding table: how many codes exist at each length, and the symbols in
/// canonical order, exactly as RFC 1951 section 3.2.2 builds them.
struct Huffman {
    counts: [u32; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    /// Build the table from a list of code lengths, one per symbol. A length of zero means the
    /// symbol is not coded, which is normal and not an error.
    fn new(lengths: &[u8]) -> Self {
        let mut counts = [0u32; 16];
        for &length in lengths {
            counts[length as usize] += 1;
        }
        counts[0] = 0;
        let mut offsets = [0u32; 16];
        let mut total = 0u32;
        for length in 1..16 {
            offsets[length] = total;
            total += counts[length];
        }
        let mut symbols = vec![0u16; total as usize];
        for (symbol, &length) in lengths.iter().enumerate() {
            if length != 0 {
                symbols[offsets[length as usize] as usize] = symbol as u16;
                offsets[length as usize] += 1;
            }
        }
        Huffman { counts, symbols }
    }

    /// Decode one symbol, one bit at a time. This is the simple canonical decode of RFC 1951:
    /// walk the lengths in order, and the first length whose code range contains the bits read
    /// so far names the symbol.
    fn decode(&self, reader: &mut BitReader<'_>) -> Result<u16, String> {
        let mut code = 0u32;
        let mut first = 0u32;
        let mut index = 0u32;
        for length in 1..16 {
            code |= reader.bit()?;
            let count = self.counts[length];
            if code >= first && code - first < count {
                return Ok(self.symbols[(index + (code - first)) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err("the deflate stream carries a code outside its Huffman table".to_string())
    }
}

/// The base length of each length code 257..=285.
const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
/// The extra bits each length code carries.
const LENGTH_EXTRA: [u32; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
/// The base distance of each distance code 0..=29.
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
/// The extra bits each distance code carries.
const DIST_EXTRA: [u32; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// The two tables a fixed-Huffman block uses (RFC 1951 section 3.2.6).
fn fixed_tables() -> (Huffman, Huffman) {
    let mut literal_lengths = vec![0u8; 288];
    for (symbol, length) in literal_lengths.iter_mut().enumerate() {
        *length = match symbol {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    let distance_lengths = vec![5u8; 30];
    (
        Huffman::new(&literal_lengths),
        Huffman::new(&distance_lengths),
    )
}

/// The two tables a dynamic-Huffman block declares in its own header.
fn dynamic_tables(reader: &mut BitReader<'_>) -> Result<(Huffman, Huffman), String> {
    // The order the code-length code lengths are written in (RFC 1951 section 3.2.7).
    const ORDER: [usize; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];
    let literal_count = reader.bits(5)? as usize + 257;
    let distance_count = reader.bits(5)? as usize + 1;
    let code_length_count = reader.bits(4)? as usize + 4;
    if literal_count > 286 || distance_count > 30 {
        return Err("the deflate header declares more codes than the format allows".to_string());
    }
    let mut code_lengths = [0u8; 19];
    for slot in 0..code_length_count {
        code_lengths[ORDER[slot]] = reader.bits(3)? as u8;
    }
    let code_length_table = Huffman::new(&code_lengths);
    let mut lengths = vec![0u8; literal_count + distance_count];
    let mut index = 0usize;
    while index < lengths.len() {
        let symbol = code_length_table.decode(reader)?;
        match symbol {
            0..=15 => {
                lengths[index] = symbol as u8;
                index += 1;
            }
            16 => {
                if index == 0 {
                    return Err("a deflate repeat has no previous length to repeat".to_string());
                }
                let previous = lengths[index - 1];
                let repeat = 3 + reader.bits(2)? as usize;
                if index + repeat > lengths.len() {
                    return Err("a deflate repeat runs past the code list".to_string());
                }
                for _ in 0..repeat {
                    lengths[index] = previous;
                    index += 1;
                }
            }
            17 => {
                let repeat = 3 + reader.bits(3)? as usize;
                if index + repeat > lengths.len() {
                    return Err("a deflate repeat runs past the code list".to_string());
                }
                index += repeat;
            }
            18 => {
                let repeat = 11 + reader.bits(7)? as usize;
                if index + repeat > lengths.len() {
                    return Err("a deflate repeat runs past the code list".to_string());
                }
                index += repeat;
            }
            _ => return Err("the deflate stream uses an invalid code-length symbol".to_string()),
        }
    }
    Ok((
        Huffman::new(&lengths[..literal_count]),
        Huffman::new(&lengths[literal_count..]),
    ))
}

/// Decode one Huffman block's symbols into the output, resolving back-references.
fn inflate_huffman_block(
    reader: &mut BitReader<'_>,
    literal: &Huffman,
    distance: &Huffman,
    out: &mut Vec<u8>,
    limit: usize,
) -> Result<(), String> {
    loop {
        let symbol = literal.decode(reader)?;
        if symbol < 256 {
            out.push(symbol as u8);
        } else if symbol == 256 {
            return Ok(());
        } else {
            let slot = symbol as usize - 257;
            if slot >= LENGTH_BASE.len() {
                return Err("the deflate stream uses an invalid length code".to_string());
            }
            let length = LENGTH_BASE[slot] as usize + reader.bits(LENGTH_EXTRA[slot])? as usize;
            let distance_slot = distance.decode(reader)? as usize;
            if distance_slot >= DIST_BASE.len() {
                return Err("the deflate stream uses an invalid distance code".to_string());
            }
            let back = DIST_BASE[distance_slot] as usize
                + reader.bits(DIST_EXTRA[distance_slot])? as usize;
            if back > out.len() {
                return Err("the deflate stream refers before the start of its output".to_string());
            }
            // A match may overlap the bytes it is producing (a run), so the copy is byte by
            // byte from a moving start rather than a bulk copy.
            let start = out.len() - back;
            for offset in 0..length {
                let byte = out[start + offset];
                out.push(byte);
            }
        }
        if out.len() > limit {
            return Err(format!(
                "the deflate stream expands past the {} bytes its container declares",
                limit
            ));
        }
    }
}

/// Unpack one raw DEFLATE stream (RFC 1951), expecting exactly `expected` bytes. The bound is
/// enforced while decoding, not after, so a container that claims to be small cannot be used to
/// make this server expand without limit.
fn inflate(data: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    let mut reader = BitReader::new(data);
    let mut out: Vec<u8> = Vec::with_capacity(expected);
    loop {
        let final_block = reader.bit()? == 1;
        match reader.bits(2)? {
            0 => {
                reader.align();
                let length = reader.bits(16)? as usize;
                let complement = reader.bits(16)? as usize;
                if length ^ 0xFFFF != complement {
                    return Err("a stored deflate block declares a broken length".to_string());
                }
                for _ in 0..length {
                    out.push(reader.bits(8)? as u8);
                }
            }
            1 => {
                let (literal, distance) = fixed_tables();
                inflate_huffman_block(&mut reader, &literal, &distance, &mut out, expected)?;
            }
            2 => {
                let (literal, distance) = dynamic_tables(&mut reader)?;
                inflate_huffman_block(&mut reader, &literal, &distance, &mut out, expected)?;
            }
            _ => return Err("the deflate stream declares an invalid block type".to_string()),
        }
        if final_block {
            break;
        }
        if out.len() > expected {
            return Err("the deflate stream expands past what its container declares".to_string());
        }
    }
    if out.len() != expected {
        return Err(format!(
            "the deflate stream expands to {} bytes and its container declares {}",
            out.len(),
            expected
        ));
    }
    Ok(out)
}

/// Pull the XMI export out of a Cameo .mdzip container. A container that holds no XMI entry, or
/// holds one this build cannot unpack, is refused with a sentence naming what was found rather
/// than a generic error: the person is told what to do next.
pub fn extract_cameo(container: &[u8]) -> Result<Vec<u8>, String> {
    let entries = zip_entries(container)?;
    let xmi = entries
        .iter()
        .find(|entry| {
            let name = entry.name.to_ascii_lowercase();
            name.ends_with(".xmi") || name.ends_with(".uml")
        })
        .or_else(|| entries.iter().find(|entry| !entry.name.ends_with('/')))
        .ok_or_else(|| "the .mdzip container holds no model entry".to_string())?;
    let end = xmi
        .data_offset
        .checked_add(xmi.compressed_size)
        .ok_or_else(|| "the .mdzip container is malformed".to_string())?;
    let stored = container
        .get(xmi.data_offset..end)
        .ok_or_else(|| "the .mdzip container is truncated".to_string())?;
    match xmi.method {
        0 => {
            if stored.len() != xmi.uncompressed_size {
                return Err(format!(
                    "the .mdzip entry {} is damaged: it declares {} bytes and stores {}",
                    xmi.name,
                    xmi.uncompressed_size,
                    stored.len()
                ));
            }
            Ok(stored.to_vec())
        }
        // Method 8 is DEFLATE, which is what a real Cameo container uses. Unpacking it here
        // rather than taking a compression dependency keeps the container support in the layer
        // that owns the drop, and the decoder is the standard RFC 1951 one.
        8 => inflate(stored, xmi.uncompressed_size).map_err(|detail| {
            format!(
                "the .mdzip entry {} could not be unpacked: {}",
                xmi.name, detail
            )
        }),
        method => Err(format!(
            "the .mdzip entry {} uses compression method {}, which this build cannot unpack",
            xmi.name, method
        )),
    }
}

// ---------------------------------------------------------------------------
// The facts: every number the summary speaks, each one from the engine.

/// One class of loss, counted from the binding's own loss report. The name is the engine's own
/// word for the construct (the front of each entry's subject), never a category this module
/// invented; the example is one real entry from the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LossClass {
    pub name: String,
    pub count: u64,
    pub example: String,
}

/// What the engine measured about the imported model, for the "check it" half of the flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthFacts {
    pub nodes: u64,
    pub relationships: u64,
    pub components: u64,
    pub isolated: u64,
    pub requirements: u64,
    pub covered: u64,
    pub uncovered: u64,
}

/// Every number the drop-zone summary speaks.
///
/// Each field is read from an engine value - the binding's loss report, the engine's own
/// round-trip diff, the graph stats and the coverage report - and the sentences in
/// [summary_sentences] are templates over this value. A reasoner may be asked to phrase the
/// same facts, but it is handed these numbers and computes nothing; the numbers render whether
/// or not a reasoner exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardFacts {
    pub format_label: String,
    pub binding: String,
    pub viewer: bool,
    pub blocks: u64,
    pub relationships: u64,
    pub requirements: u64,
    pub graph_nodes: u64,
    pub artifact_bytes: u64,
    pub blocking: u64,
    pub unmappable: u64,
    pub lossy: u64,
    pub declarations: u64,
    pub round_trip: Option<bool>,
    pub classes: Vec<LossClass>,
    pub health: Option<HealthFacts>,
}

/// The engine values the facts are read from. A struct rather than eight positional arguments
/// so the call site names which measurement is which.
pub struct FactsInput<'a> {
    pub format_label: &'a str,
    pub binding: &'a str,
    pub direction: Direction,
    pub root: &'a OkfRoot,
    pub loss_report: &'a LossReport,
    pub round_trip: Option<bool>,
    pub artifact_bytes: u64,
    pub health: Option<HealthFacts>,
}

impl OnboardFacts {
    /// Read every fact from the engine's own values. Nothing is estimated and nothing is
    /// counted twice: the element counts come from the imported document, the loss counts from
    /// the binding's report, and the round-trip verdict from the engine's diff.
    pub fn from_engine(input: FactsInput<'_>) -> Self {
        let root = input.root;
        let report = input.loss_report;
        let graph_edges = root
            .graph
            .as_ref()
            .map(|graph| graph.edges.len() as u64)
            .unwrap_or(root.summary.graph_edges);
        OnboardFacts {
            format_label: input.format_label.to_string(),
            binding: input.binding.to_string(),
            viewer: input.direction == Direction::ImportOnly,
            blocks: root.structure.len() as u64,
            relationships: graph_edges,
            requirements: root.requirements.len() as u64,
            graph_nodes: root
                .graph
                .as_ref()
                .map(|graph| graph.nodes.len() as u64)
                .unwrap_or(root.summary.graph_nodes),
            artifact_bytes: input.artifact_bytes,
            blocking: report.blocking().len() as u64,
            unmappable: report.content_losses().len() as u64,
            lossy: report.lossy().len() as u64,
            declarations: report.declarations().len() as u64,
            round_trip: input.round_trip,
            classes: loss_classes(report, MAX_LOSS_CLASSES),
            health: input.health,
        }
    }
}

/// The engine's own name for a loss class: the construct at the front of the entry's subject.
/// For the XMI reader that is the XMI element name (uml:Class, uml:Property, ...), which is
/// exactly "the construct types I do not map yet"; nothing is re-categorised here.
fn loss_class_name(mapping: &Mapping) -> String {
    mapping
        .subject
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string()
}

/// The biggest classes of loss, counted from the binding's own loss report. Ties are broken by
/// name so the same report always produces the same list.
pub fn loss_classes(report: &LossReport, limit: usize) -> Vec<LossClass> {
    let mut by_name: BTreeMap<String, (u64, String)> = BTreeMap::new();
    for mapping in report.blocking() {
        let name = loss_class_name(mapping);
        let entry = by_name
            .entry(name)
            .or_insert_with(|| (0, mapping.subject.clone()));
        entry.0 += 1;
    }
    let mut classes: Vec<LossClass> = by_name
        .into_iter()
        .map(|(name, (count, example))| LossClass {
            name,
            count,
            example,
        })
        .collect();
    classes.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    classes.truncate(limit);
    classes
}

/// Read the health facts from the engine's graph analysis. Both calls require a graph, and a
/// document that carries none has nothing to say topologically, so it yields None rather than a
/// panic.
pub fn health_facts(root: &OkfRoot) -> Option<HealthFacts> {
    let graph = root.graph.as_ref()?;
    let stats = graph_stats(root);
    let coverage = requirement_coverage(root);
    Some(HealthFacts {
        nodes: stats.node_count as u64,
        relationships: graph.edges.len() as u64,
        components: stats.component_count as u64,
        isolated: stats.isolated.len() as u64,
        requirements: coverage.total as u64,
        covered: coverage.covered as u64,
        uncovered: coverage.uncovered.len() as u64,
    })
}

fn plural(n: u64, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 {
        one
    } else {
        many
    }
}

/// The plain-language summary: template composition over the engine's own numbers.
///
/// Read this as a contract. Every value interpolated below comes from [OnboardFacts], and every
/// field of that value is read from an engine computation. There is no arithmetic here beyond
/// selecting a word, so a sentence cannot state a number the engine did not produce.
pub fn summary_sentences(facts: &OnboardFacts) -> Vec<String> {
    let mut out = Vec::new();

    out.push(format!(
        "This is {} — read by the {} binding.",
        facts.format_label, facts.binding
    ));

    out.push(format!(
        "It carried {} {}, {} {} and {} {}.",
        facts.blocks,
        plural(facts.blocks, "block", "blocks"),
        facts.relationships,
        plural(facts.relationships, "relationship", "relationships"),
        facts.requirements,
        plural(facts.requirements, "requirement", "requirements"),
    ));

    out.push(match facts.round_trip {
        Some(true) => "It round-trips exactly: the engine wrote the imported model back to the \
                       source form, read it again, and its own diff of the two found nothing \
                       missing."
            .to_string(),
        Some(false) => "The engine round-tripped it, and its own diff says the reader's \
                        write-back is not exact — this is a disagreement between the engine and \
                        the reader, and it is refused rather than hidden."
            .to_string(),
        None => "This reader is a viewer (it reads but cannot write back), so no round trip can \
                 be measured; the loss report is its own account of the read."
            .to_string(),
    });

    if facts.blocking == 0 {
        out.push(
            "Nothing is outside what this reader carries: every entry in its loss report is \
             exact."
                .to_string(),
        );
    } else {
        out.push(format!(
            "{} {} outside what this reader carries: {} it cannot map at all and {} it carries \
             with a named drop.",
            facts.blocking,
            plural(facts.blocking, "thing is", "things are"),
            facts.unmappable,
            facts.lossy,
        ));
    }
    if facts.declarations > 0 {
        out.push(format!(
            "It also recognised {} {} (profiles, package imports, annotations and root metadata) \
             that carry no model content and are therefore not losses.",
            facts.declarations,
            plural(facts.declarations, "declaration", "declarations"),
        ));
    }

    if !facts.classes.is_empty() {
        let heading = if facts.classes.len() >= MAX_LOSS_CLASSES {
            "The five biggest classes of loss are"
        } else {
            "These are the classes of loss"
        };
        let listed = facts
            .classes
            .iter()
            .map(|class| {
                format!(
                    "{} × {} (for example {})",
                    class.count, class.name, class.example
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        out.push(format!("{}: {}.", heading, listed));
    }

    if let Some(health) = &facts.health {
        out.push(coverage_sentence(health));
        out.push(topology_sentence(health));
    }

    out
}

/// The coverage half of the health sentence, from the engine's coverage report.
fn coverage_sentence(health: &HealthFacts) -> String {
    if health.requirements == 0 {
        return "The model declares no requirements.".to_string();
    }
    if health.uncovered == 0 {
        return format!(
            "All {} {} covered by a Satisfy, Refine, Verify or Allocate link.",
            health.requirements,
            plural(health.requirements, "requirement is", "requirements are"),
        );
    }
    if health.covered == 0 {
        return format!(
            "{} {}, none covered — the model carries no traceability link from a block to a \
             requirement.",
            health.requirements,
            plural(health.requirements, "requirement is", "requirements are"),
        );
    }
    format!(
        "{} of {} {} covered; {} uncovered.",
        health.covered,
        health.requirements,
        plural(health.requirements, "requirement is", "requirements are"),
        health.uncovered,
    )
}

/// The topology half of the health sentence, from the engine's graph stats.
fn topology_sentence(health: &HealthFacts) -> String {
    if health.components <= 1 && health.isolated == 0 {
        return format!(
            "All {} {} one connected group.",
            health.nodes,
            plural(health.nodes, "node sits in", "nodes sit in"),
        );
    }
    format!(
        "{} {} in {} disconnected {}; {} of them isolated, with no edges at all.",
        health.nodes,
        plural(health.nodes, "node sits", "nodes sit"),
        health.components,
        plural(health.components, "group", "groups"),
        health.isolated,
    )
}

// ---------------------------------------------------------------------------
// The drop zone markup.

/// The inline enhancement: drag-and-drop onto the section and the chosen file's name beside
/// the button. It is an enhancement and nothing more - the file input inside the same form is
/// what actually carries the artifact, so the flow completes with this script never running.
const DROPZONE_SCRIPT: &str = r#"(function () {
  'use strict';
  var zone = document.getElementById('mw-dropzone');
  if (!zone) { return; }
  var input = zone.querySelector('input[type="file"]');
  if (!input) { return; }
  var name = zone.querySelector('.mw-dropzone-file');
  function show() {
    if (!name) { return; }
    name.textContent = input.files && input.files.length ? input.files[0].name : '';
  }
  input.addEventListener('change', show);
  ['dragenter', 'dragover'].forEach(function (type) {
    zone.addEventListener(type, function (event) {
      event.preventDefault();
      zone.classList.add('mw-dragging');
    });
  });
  ['dragleave', 'drop'].forEach(function (type) {
    zone.addEventListener(type, function (event) {
      event.preventDefault();
      zone.classList.remove('mw-dragging');
    });
  });
  zone.addEventListener('drop', function (event) {
    var files = event.dataTransfer && event.dataTransfer.files;
    if (files && files.length) {
      input.files = files;
      show();
    }
  });
})();
"#;

/// The drop zone: the primary element of the front door. One target, no format dropdown, no
/// project-name box required first - the format is detected from the file and the project is
/// named after it. The binding dropdown stays, behind the expert disclosure, exactly as the
/// design says it should.
pub fn dropzone_markup(bindings: &[BindingInfo], can_write: bool) -> Markup {
    html! {
        section class="mw-dropzone" id="mw-dropzone" {
            h1 { "Drop your SysML model here, or choose a file" }
            p class="mw-dropzone-hint" {
                "XMI — a MagicDraw, Cameo or any UML/SysML v1 export; a Cameo "
                code { ".mdzip" } " container; or SysML v2 textual notation ("
                code { ".sysml" } "). The format is detected from the file itself and the "
                "project is named after it, so there is nothing to choose and no project to "
                "create first."
            }
            form method="post" action="/onboard" enctype="multipart/form-data"
                 class="mw-dropzone-form" {
                label for="mw-dropzone-input" { "Your model file" }
                input type="file" id="mw-dropzone-input" name="artifact"
                      accept=".xmi,.xml,.uml,.mdzip,.sysml,.kerml" required;
                details class="mw-dropzone-advanced" {
                    summary { "Advanced: name the project or pick the binding yourself" }
                    div class="advanced-grid" {
                        p {
                            label for="mw-dropzone-name" { "Project name (optional)" }
                            input type="text" id="mw-dropzone-name" name="name"
                                  placeholder="taken from the file name";
                        }
                        p {
                            label for="mw-dropzone-binding" { "Binding (optional)" }
                            select id="mw-dropzone-binding" name="binding" {
                                option value="" { "detected from the file" }
                                @for binding in bindings {
                                    option value=(format!("{}@{}", binding.id, binding.version)) {
                                        (binding.id.as_str()) "@" (binding.version.as_str())
                                        @if binding.direction == Direction::ImportOnly {
                                            " (viewer: reads, does not write back)"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                p class="mw-dropzone-actions" {
                    @if can_write {
                        button type="submit" { "Onboard this model" }
                    } @else {
                        button type="submit" disabled { "Onboard this model" }
                    }
                    span class="mw-dropzone-file" aria-live="polite" {}
                }
            }
        }
        script { (PreEscaped(DROPZONE_SCRIPT)) }
    }
}

/// GET /onboard - the drop zone as its own page, for a link that goes straight to the front
/// door rather than to the project list.
pub async fn onboard_page(State(state): State<ApiState>, headers: HeaderMap) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    if !identity.may(Permission::Read) {
        return layout::error_page(
            StatusCode::FORBIDDEN,
            Some(&identity.subject),
            mechanism,
            "read permission required",
        );
    }
    let nav = match layout::Nav::load(&state, &identity, None) {
        Ok(nav) => nav,
        Err(error) => {
            return layout::error_page(
                error.status,
                Some(&identity.subject),
                mechanism,
                &error.message,
            )
        }
    };
    let bindings = binding_registry::bindings();
    let body = html! {
        (dropzone_markup(&bindings, identity.may(Permission::Write)))
    };
    layout::html_response(
        StatusCode::OK,
        layout::shell(
            "modelwrite — onboard a model",
            &nav,
            Some(&identity.subject),
            identity.may(Permission::Administer),
            mechanism,
            body,
        ),
    )
}

// ---------------------------------------------------------------------------
// The drop itself.

/// What the drop zone collected before anything was imported.
struct Dropped {
    file_name: String,
    head: Vec<u8>,
    staged: tempfile::NamedTempFile,
    project_override: String,
    binding_override: String,
}

/// The intake result: a staged artifact, or an upload the configured limit refused before any
/// of it was imported.
enum Intake {
    Ready(Box<Dropped>),
    TooLarge { received: u64 },
}

/// POST /onboard - the drop. Detect, create the project, retain the artifact, import, and render
/// the result. The status code follows the outcome - 201 when a model was committed, 422 when
/// blocking losses need an acceptance - and the refusal is a RESULT the page renders, never a
/// bare error.
pub async fn onboard_upload(
    State(state): State<ApiState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    let report = match perform_onboard(&state, &identity, &mut multipart).await {
        Ok(report) => report,
        Err(error) => {
            return layout::error_page(
                error.status,
                Some(&identity.subject),
                mechanism,
                &error.message,
            )
        }
    };
    let nav = match layout::Nav::load(&state, &identity, Some(&report.project)) {
        Ok(nav) => nav,
        Err(error) => {
            return layout::error_page(
                error.status,
                Some(&identity.subject),
                mechanism,
                &error.message,
            )
        }
    };
    let status = if report.commit.is_some() {
        StatusCode::CREATED
    } else {
        StatusCode::UNPROCESSABLE_ENTITY
    };
    layout::html_response(
        status,
        layout::shell(
            &format!("modelwrite — {} — onboarded", report.project),
            &nav,
            Some(&identity.subject),
            identity.may(Permission::Administer),
            mechanism,
            result_page(&identity, &report),
        ),
    )
}

/// Stream the multipart body into a staged file, enforcing the server's body limit while the
/// bytes arrive and keeping only the first [SNIFF_BYTES] for detection. This is the same
/// streaming discipline the import page's upload uses: a 36 MB export is never buffered in full
/// just to be identified.
async fn read_drop(state: &ApiState, multipart: &mut Multipart) -> Result<Intake, ApiError> {
    let mut staged = tempfile::NamedTempFile::new()
        .map_err(|e| ApiError::internal(format!("could not stage the artifact: {}", e)))?;
    let mut head: Vec<u8> = Vec::new();
    let mut file_name = String::new();
    let mut project_override = String::new();
    let mut binding_override = String::new();
    let mut received: u64 = 0;
    let mut got_artifact = false;

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(error) => {
                if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    return Ok(Intake::TooLarge { received });
                }
                return Err(ApiError::bad_request(format!(
                    "could not read the dropped file: {}",
                    error.body_text()
                )));
            }
        };
        let name = field.name().unwrap_or_default().to_string();
        if name == "artifact" {
            got_artifact = true;
            file_name = field.file_name().unwrap_or_default().to_string();
            let mut field = field;
            loop {
                let chunk = match field.chunk().await {
                    Ok(Some(chunk)) => chunk,
                    Ok(None) => break,
                    Err(error) => {
                        if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
                            return Ok(Intake::TooLarge { received });
                        }
                        return Err(ApiError::bad_request(format!(
                            "could not read the dropped file: {}",
                            error.body_text()
                        )));
                    }
                };
                received = received.saturating_add(chunk.len() as u64);
                if received > state.max_body_bytes {
                    return Ok(Intake::TooLarge { received });
                }
                if head.len() < SNIFF_BYTES {
                    let take = (SNIFF_BYTES - head.len()).min(chunk.len());
                    head.extend_from_slice(&chunk[..take]);
                }
                staged.write_all(&chunk).map_err(|e| {
                    ApiError::internal(format!("could not stage the artifact: {}", e))
                })?;
            }
        } else {
            let value = field.text().await.map_err(|e| {
                ApiError::bad_request(format!(
                    "could not read the dropped file: {}",
                    e.body_text()
                ))
            })?;
            match name.as_str() {
                "name" => project_override = value,
                "binding" => binding_override = value,
                _ => {}
            }
        }
    }

    if !got_artifact {
        return Err(ApiError::bad_request(
            "drop a model file: the artifact must be supplied as the artifact file field",
        ));
    }
    if received == 0 {
        return Err(ApiError::bad_request("the dropped file is empty"));
    }
    Ok(Intake::Ready(Box::new(Dropped {
        file_name,
        head,
        staged,
        project_override,
        binding_override,
    })))
}

/// A project name derived from the dropped file's name. The file's stem is the first choice, so
/// dropping coffee-machine.xmi offers a project called coffee-machine rather than asking a
/// question; characters outside the name shape become hyphens and the result is validated by
/// the SAME rule every other project name passes.
pub fn project_name_from_file(file_name: &str) -> Result<String, ApiError> {
    let base = file_name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(file_name)
        .to_string();
    let stem = match base.rsplit_once('.') {
        Some((stem, _extension)) if !stem.is_empty() => stem.to_string(),
        _ => base,
    };
    let mut name = String::with_capacity(stem.len());
    let mut last_hyphen = false;
    for ch in stem.chars() {
        let mapped = if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' {
            ch
        } else {
            '-'
        };
        if mapped == '-' {
            if last_hyphen {
                continue;
            }
            last_hyphen = true;
        } else {
            last_hyphen = false;
        }
        name.push(mapped);
    }
    let name = name.trim_matches('-').to_string();
    let name = if name.len() > 64 {
        name[..64].trim_end_matches('-').to_string()
    } else {
        name
    };
    if name.is_empty() || !name.chars().any(|c| c.is_ascii_alphanumeric()) {
        return Ok("model".to_string());
    }
    validate_name("project name", &name)?;
    Ok(name)
}

/// The whole drop, from the staged bytes to the result report. Permission and scope are decided
/// first, in the same order as every other write path.
async fn perform_onboard(
    state: &ApiState,
    identity: &Identity,
    multipart: &mut Multipart,
) -> Result<OnboardReport, ApiError> {
    if !identity.may(Permission::Write) {
        return Err(ApiError::forbidden("write permission required"));
    }
    let dropped = match read_drop(state, multipart).await? {
        Intake::Ready(dropped) => dropped,
        Intake::TooLarge { received } => {
            return Err(ApiError::unprocessable(
                format!(
                    "the dropped file is larger than this server accepts: {} bytes received, \
                     limit {} bytes (MW_MAX_BODY_BYTES). Nothing was imported and nothing was \
                     stored; raise the limit and drop it again.",
                    received, state.max_body_bytes
                ),
                Vec::new(),
            ))
        }
    };

    // The format is decided from the bytes, never from a dropdown; an explicit binding override
    // is the expert's escape hatch and still goes through the same registry.
    let format = detect(&dropped.head, &dropped.file_name);
    if let Format::Unknown(detail) = &format {
        return Err(ApiError::unprocessable(
            format!(
                "This file was not recognised, so nothing was imported: {}. Drop an XMI export, \
                 a Cameo .mdzip container or a SysML v2 .sysml file.",
                detail
            ),
            Vec::new(),
        ));
    }
    let binding_selector = if dropped.binding_override.trim().is_empty() {
        format
            .binding()
            .ok_or_else(|| ApiError::unprocessable("unrecognised file", Vec::new()))?
    } else {
        dropped.binding_override.trim().to_string()
    };
    let (binding_id, binding_version) = crate::binding_api::parse_binding(&binding_selector)?;

    let project = if dropped.project_override.trim().is_empty() {
        project_name_from_file(&dropped.file_name)?
    } else {
        validate_name("project name", dropped.project_override.trim())?;
        dropped.project_override.trim().to_string()
    };
    if !identity.may_reach(&project) {
        return Err(ApiError::forbidden("project not in scope"));
    }

    // Retain before anything is read: the container first when there is one, then the artifact
    // the binding will actually read. Content addressing means the same model dropped twice
    // costs one blob.
    let (artifact, artifact_hash, container_hash) = match format {
        Format::Cameo => {
            let container = std::fs::read(dropped.staged.path()).map_err(|e| {
                ApiError::internal(format!("could not read the staged container: {}", e))
            })?;
            let payload = extract_cameo(&container).map_err(|detail| {
                ApiError::unprocessable(
                    format!("The .mdzip container could not be unpacked: {}.", detail),
                    Vec::new(),
                )
            })?;
            let container_hash = state
                .store_for(identity)
                .put_blob(&container)
                .map_err(map_store_error)?;
            let payload_hash = state
                .store_for(identity)
                .put_blob(&payload)
                .map_err(map_store_error)?;
            (payload, payload_hash, Some(container_hash))
        }
        _ => {
            let temp_path = dropped.staged.into_temp_path();
            let hash = state
                .store_for(identity)
                .put_blob_file(temp_path.as_ref())
                .map_err(map_store_error)?;
            let bytes = state
                .store_for(identity)
                .blob(&hash)
                .map_err(map_store_error)?
                .ok_or_else(|| {
                    ApiError::internal("the retained artifact could not be read back")
                })?;
            (bytes, hash, None)
        }
    };

    // A project that already exists is reused rather than refused: dropping a second version of
    // the same model is a normal thing to do, and the import commits onto the branch like any
    // other commit. A project that does not exist is created here, named after the file.
    if state
        .store_for(identity)
        .project(&project)
        .map_err(map_store_error)?
        .is_none()
    {
        create_project_core(
            state.store_for(identity).as_ref(),
            &identity.subject,
            state.auth.mechanism(),
            state.auth.authorizer().unwrap_or(""),
            &project,
        )
        .map_err(map_store_error)?;
    }

    let author = identity.subject.clone();
    let message = format!("onboard {}", dropped.file_name);
    verify_actor(&state.auth, identity, None)?;

    let outcome = import_core(
        state.store_for(identity).as_ref(),
        &project,
        &ImportCore {
            binding: &binding_selector,
            branch: ONBOARD_BRANCH,
            author: &author,
            message: &message,
            artifact: &artifact,
            accept_losses: &[],
            holder: None,
            actor: &identity.subject,
            mechanism: state.auth.mechanism(),
            authorizer: state.auth.authorizer().unwrap_or(""),
            acceptance: None,
        },
    )?;

    let artifact_bytes = artifact.len() as u64;
    let mut report = OnboardReport {
        project,
        file_name: dropped.file_name.clone(),
        format_label: format.label().to_string(),
        binding: binding_selector,
        artifact_hash,
        container_hash,
        artifact_bytes,
        branch: ONBOARD_BRANCH.to_string(),
        message,
        loss_report: LossReport {
            binding: binding_infos(&binding_id, &binding_version),
            mappings: Vec::new(),
            artifact_hash: String::new(),
        },
        unaccepted: Vec::new(),
        commit: None,
        facts: None,
    };

    match outcome {
        ImportOutcome::Committed {
            commit,
            artifact_hash,
            loss_report,
            fidelity,
            binding_id,
            binding_version,
            ..
        } => {
            let root = load_model(
                state.store_for(identity).as_ref(),
                &report.project,
                &commit.hash,
            )
            .map_err(map_store_error)?;
            report.artifact_hash = artifact_hash;
            report.binding = format!("{}@{}", binding_id, binding_version);
            report.facts = Some(OnboardFacts::from_engine(FactsInput {
                format_label: &report.format_label,
                binding: &report.binding,
                direction: direction_of(&binding_id, &binding_version),
                round_trip: fidelity.as_ref().map(|measured| measured.diff.equal),
                root: &root,
                loss_report: &loss_report,
                artifact_bytes,
                health: health_facts(&root),
            }));
            report.loss_report = loss_report;
            report.commit = Some(*commit);
        }
        ImportOutcome::Blocking {
            artifact_hash,
            loss_report,
            unaccepted,
            fidelity,
            binding_id,
            binding_version,
        } => {
            // No commit means no stored document to measure, so the retained bytes are read
            // through the SAME binding again. It is deterministic, and it is the only way the
            // refusal page can name the blocks and relationships it is offering to import.
            let root = reread_root(
                state.store_for(identity).as_ref(),
                &artifact_hash,
                &binding_id,
                &binding_version,
            )?;
            report.artifact_hash = artifact_hash;
            report.binding = format!("{}@{}", binding_id, binding_version);
            report.facts = Some(OnboardFacts::from_engine(FactsInput {
                format_label: &report.format_label,
                binding: &report.binding,
                direction: direction_of(&binding_id, &binding_version),
                round_trip: fidelity.as_ref().map(|measured| measured.diff.equal),
                root: &root,
                loss_report: &loss_report,
                artifact_bytes,
                health: None,
            }));
            report.loss_report = loss_report;
            report.unaccepted = unaccepted;
        }
    }
    Ok(report)
}

/// The binding's declared direction, read from the registry rather than assumed, so the summary
/// can say "viewer" only when the binding says so.
fn direction_of(binding_id: &str, binding_version: &str) -> Direction {
    binding_registry::resolve(binding_id, binding_version)
        .map(|binding| binding.info().direction)
        .unwrap_or(Direction::ImportOnly)
}

/// The binding's declared identity, read from the registry.
fn binding_infos(binding_id: &str, binding_version: &str) -> BindingInfo {
    binding_registry::resolve(binding_id, binding_version)
        .map(|binding| binding.info())
        .unwrap_or(BindingInfo {
            id: binding_id.to_string(),
            version: binding_version.to_string(),
            direction: Direction::ImportOnly,
            description: String::new(),
        })
}

/// The retained artifact's own bytes, read back by its content address.
fn retained_bytes(
    state: &ApiState,
    identity: &Identity,
    artifact_hash: &str,
) -> Result<Vec<u8>, ApiError> {
    state
        .store_for(identity)
        .blob(artifact_hash)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::internal("the retained artifact could not be read back"))
}

/// Re-read a retained artifact through the binding that read it, to obtain the imported document
/// the summary counts. Only the refusal path needs this.
fn reread_root(
    store: &dyn Store,
    artifact_hash: &str,
    binding_id: &str,
    binding_version: &str,
) -> Result<OkfRoot, ApiError> {
    let bytes = store
        .blob(artifact_hash)
        .map_err(map_store_error)?
        .ok_or_else(|| ApiError::internal("the retained artifact could not be read back"))?;
    let binding = binding_registry::resolve(binding_id, binding_version).ok_or_else(|| {
        ApiError::internal("the binding that read this artifact is no longer registered")
    })?;
    let (root, _) = binding.import(&bytes).map_err(|error| {
        ApiError::internal(format!(
            "the binding could not re-read the retained artifact: {}",
            error
        ))
    })?;
    Ok(root)
}

/// A project name derived from a dropped file, as the front door offers it. Falls back to
/// "model" when the file's name can yield nothing usable, so a drop never fails on naming.
pub fn derived_project_name(file_name: &str) -> String {
    project_name_from_file(file_name).unwrap_or_else(|_| "model".to_string())
}

// ---------------------------------------------------------------------------
// The result: what the engine found, in words, with the one button.

/// Everything the result page renders, assembled from engine values only.
pub struct OnboardReport {
    pub project: String,
    pub file_name: String,
    pub format_label: String,
    pub binding: String,
    pub artifact_hash: String,
    pub container_hash: Option<String>,
    pub artifact_bytes: u64,
    pub branch: String,
    pub message: String,
    pub loss_report: LossReport,
    pub unaccepted: Vec<Mapping>,
    pub commit: Option<Commit>,
    /// The engine's numbers. None only if the document could not be measured, which the page
    /// states rather than papering over.
    pub facts: Option<OnboardFacts>,
}

impl OnboardReport {
    /// Whether the model was committed.
    pub fn committed(&self) -> bool {
        self.commit.is_some()
    }
}

/// The sentences the page shows, from the engine's numbers. Kept as a function so every caller -
/// the page, a test, a future reasoner handed the same facts - reads the identical prose.
pub fn report_sentences(report: &OnboardReport) -> Vec<String> {
    match &report.facts {
        Some(facts) => summary_sentences(facts),
        None => vec![
            "The engine produced no measurement for this artifact, so there is nothing it can \
             honestly say about the model."
                .to_string(),
        ],
    }
}

/// The engine's own numbers, as a table under the prose. The prose may be rephrased; this table
/// is the measurement, and it renders whether or not anything is configured to rephrase it.
fn facts_table(report: &OnboardReport) -> Markup {
    let Some(facts) = &report.facts else {
        return Markup::default();
    };
    html! {
        table class="onboard-facts" {
            thead {
                tr {
                    th { "measured by the engine" }
                    th class="num" { "count" }
                }
            }
            tbody {
                tr { td { "blocks carried" } td class="num" { (facts.blocks) } }
                tr { td { "relationships carried" } td class="num" { (facts.relationships) } }
                tr { td { "requirements carried" } td class="num" { (facts.requirements) } }
                tr { td { "graph nodes" } td class="num" { (facts.graph_nodes) } }
                tr { td { "blocking losses in the report" } td class="num" { (facts.blocking) } }
                tr { td { "unmappable content losses" } td class="num" { (facts.unmappable) } }
                tr { td { "lossy drops" } td class="num" { (facts.lossy) } }
                tr { td { "declarations recognised" } td class="num" { (facts.declarations) } }
            }
        }
    }
}

/// The loss classes, each with its count and one real example.
fn classes_markup(report: &OnboardReport) -> Markup {
    let Some(facts) = &report.facts else {
        return Markup::default();
    };
    if facts.classes.is_empty() {
        return Markup::default();
    }
    html! {
        ul class="onboard-classes" {
            @for class in &facts.classes {
                li {
                    span class="lc-count" { (class.count) }
                    " × "
                    span class="lc-name" { (class.name.as_str()) }
                    span class="lc-example" { " — for example " (class.example.as_str()) }
                }
            }
        }
    }
}

/// The result page: the plain-language summary first, the engine's table under it, and then
/// exactly one next step - the one button when losses must be accepted, the health view when the
/// model landed.
fn result_page(identity: &Identity, report: &OnboardReport) -> Markup {
    let can_write = identity.may(Permission::Write);
    let sentences = report_sentences(report);
    let committed = report.committed();
    let project = crate::ui::urlencode(&report.project);
    let health_href = match &report.commit {
        Some(commit) => format!(
            "/ui/projects/{}/health?commit={}",
            project,
            crate::ui::urlencode(&commit.hash)
        ),
        None => format!("/ui/projects/{}/health?branch={}", project, ONBOARD_BRANCH),
    };
    let blocking = report.loss_report.blocking();
    let all_identities = blocking
        .iter()
        .map(|mapping| agent::losses::entry_identity(mapping))
        .collect::<Vec<_>>()
        .join("\n");
    let button_label = match &report.facts {
        Some(facts) => format!(
            "Import {} {} and {} {}, accepting these {} named {}",
            facts.blocks,
            plural(facts.blocks, "block", "blocks"),
            facts.relationships,
            plural(facts.relationships, "relationship", "relationships"),
            facts.blocking,
            plural(facts.blocking, "loss", "losses"),
        ),
        None => "Accept the losses and import".to_string(),
    };
    html! {
        h1 {
            @if committed { "Onboarded: " } @else { "Refused until you accept: " }
            (report.file_name.as_str())
        }
        p class="meta" {
            "project " code { (report.project.as_str()) }
            " · detected " (report.format_label.as_str())
            " · binding " code { (report.binding.as_str()) }
        }
        section class="onboard-summary" id="summary" {
            @for sentence in &sentences {
                p { (sentence.as_str()) }
            }
        }
        (facts_table(report))
        (classes_markup(report))
        (import_ui::retained_markup(
            &report.artifact_hash,
            report.commit.as_ref().map(|commit| commit.hash.as_str()),
        ))
        @if let Some(container_hash) = &report.container_hash {
            p class="meta" {
                "The .mdzip container you dropped is retained byte-for-byte as "
                code { (container_hash.as_str()) } ", and the XMI it holds as "
                code { (report.artifact_hash.as_str()) } "."
            }
        }
        @if committed {
            section class="model-section" id="next" {
                h2 { "Check it" }
                p {
                    a href=(health_href) { "Open the model-health view" }
                    " — the engine's own readings of this model, and what they mean."
                }
            }
        } @else {
            section class="model-section" id="accept" {
                h2 { "Import it" }
                p {
                    "Nothing has been committed. The source artifact is retained, so accepting \
                     re-reads the bytes by their hash — you do not upload the file again."
                }
                @if can_write {
                    form method="post"
                         action={ "/ui/projects/" (project) "/onboard/accept" }
                         class="onboard-accept" {
                        input type="hidden" name="artifactHash" value=(report.artifact_hash.as_str());
                        input type="hidden" name="branch" value=(report.branch.as_str());
                        input type="hidden" name="message" value=(report.message.as_str());
                        input type="hidden" name="all_losses" value=(all_identities);
                        button type="submit" name="accept_all" value="1" { (button_label) }
                    }
                } @else {
                    p { "Ask a writer to accept the losses and import this model." }
                }
                details class="onboard-losses" {
                    summary {
                        "See every one of the " (blocking.len()) " named losses first"
                    }
                    (import_ui::loss_report_markup(&report.loss_report))
                    @if can_write {
                        (import_ui::accept_form_markup(
                            &report.project,
                            &report.artifact_hash,
                            &report.branch,
                            &report.message,
                            "",
                            &blocking,
                            &report.unaccepted,
                        ))
                    }
                }
                (import_ui::fidelity_note_markup(Some(report.loss_report.binding.direction)))
            }
        }
    }
}

/// POST /ui/projects/:project/onboard/accept - the ONE button. It accepts every blocking loss by
/// its entry identity and re-runs the migration from the RETAINED artifact through the same
/// [accept_import_core] the import page uses, then renders the same result page.
pub async fn onboard_accept(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(project): Path<String>,
    form: Result<
        axum::extract::Form<std::collections::HashMap<String, String>>,
        axum::extract::rejection::FormRejection,
    >,
) -> Response {
    let mechanism = state.auth.mechanism();
    let identity = match resolve_identity(&state, &headers).await {
        Ok(identity) => identity,
        Err(error) => return layout::sign_in_page(mechanism, &error.message),
    };
    let form = match form {
        Ok(axum::extract::Form(form)) => form,
        Err(rejection) => {
            return layout::error_page(
                rejection.status(),
                Some(&identity.subject),
                mechanism,
                &rejection.body_text(),
            )
        }
    };
    match perform_onboard_accept(&state, &identity, &project, &form) {
        Ok(report) => {
            let nav = match layout::Nav::load(&state, &identity, Some(&project)) {
                Ok(nav) => nav,
                Err(error) => {
                    return layout::error_page(
                        error.status,
                        Some(&identity.subject),
                        mechanism,
                        &error.message,
                    )
                }
            };
            let status = if report.commit.is_some() {
                StatusCode::CREATED
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            };
            layout::html_response(
                status,
                layout::shell(
                    &format!("modelwrite — {} — onboarded", report.project),
                    &nav,
                    Some(&identity.subject),
                    identity.may(Permission::Administer),
                    mechanism,
                    result_page(&identity, &report),
                ),
            )
        }
        Err(error) => layout::error_page(
            error.status,
            Some(&identity.subject),
            mechanism,
            &error.message,
        ),
    }
}

/// The acceptance half of the one button: the same permission, scope and acceptance decisions
/// the import page's acceptance makes, through the same core.
fn perform_onboard_accept(
    state: &ApiState,
    identity: &Identity,
    project: &str,
    form: &std::collections::HashMap<String, String>,
) -> Result<OnboardReport, ApiError> {
    if !identity.may(Permission::Write) {
        return Err(ApiError::forbidden("write permission required"));
    }
    if !identity.may_reach(project) {
        return Err(ApiError::forbidden("project not in scope"));
    }
    let accept = import_ui::AcceptForm::from_form(form);
    if accept.artifact_hash.is_empty() {
        return Err(ApiError::bad_request("an artifact hash is required"));
    }
    if accept.accept_losses.is_empty() {
        return Err(ApiError::bad_request(
            "at least one accepted loss is required",
        ));
    }
    validate_name("branch name", &accept.branch)?;
    if accept.message.trim().is_empty() {
        return Err(ApiError::bad_request("commit message must not be empty"));
    }
    let author = identity.subject.clone();
    let holder = if accept.holder.is_empty() {
        None
    } else {
        Some(accept.holder.as_str())
    };
    verify_actor(&state.auth, identity, holder)?;

    let outcome = accept_import_core(
        state.store_for(identity).as_ref(),
        project,
        &accept.artifact_hash,
        &accept.branch,
        &author,
        &accept.message,
        holder,
        &accept.accept_losses,
        &identity.subject,
        state.auth.mechanism(),
        state.auth.authorizer().unwrap_or(""),
    )?;

    let file_name = accept
        .message
        .strip_prefix("onboard ")
        .unwrap_or(&accept.message)
        .to_string();
    let mut report = OnboardReport {
        project: project.to_string(),
        file_name,
        format_label: "the format detected when it was dropped".to_string(),
        binding: String::new(),
        artifact_hash: accept.artifact_hash.clone(),
        container_hash: None,
        artifact_bytes: 0,
        branch: accept.branch.clone(),
        message: accept.message.clone(),
        loss_report: LossReport {
            binding: binding_infos("", ""),
            mappings: Vec::new(),
            artifact_hash: String::new(),
        },
        unaccepted: Vec::new(),
        commit: None,
        facts: None,
    };

    match outcome {
        ImportOutcome::Committed {
            commit,
            artifact_hash,
            loss_report,
            fidelity,
            binding_id,
            binding_version,
            ..
        } => {
            let root = load_model(state.store_for(identity).as_ref(), project, &commit.hash)
                .map_err(map_store_error)?;
            // The acceptance page states the same facts as the drop page: the format is
            // re-detected from the RETAINED bytes, so nothing is remembered that the bytes do
            // not still say.
            let bytes = retained_bytes(state, identity, &artifact_hash)?;
            let artifact_bytes = bytes.len() as u64;
            report.format_label = detect(&bytes, &report.file_name).label().to_string();
            report.artifact_hash = artifact_hash;
            report.binding = format!("{}@{}", binding_id, binding_version);
            report.artifact_bytes = artifact_bytes;
            report.facts = Some(OnboardFacts::from_engine(FactsInput {
                format_label: &report.format_label,
                binding: &report.binding,
                direction: direction_of(&binding_id, &binding_version),
                round_trip: fidelity.as_ref().map(|measured| measured.diff.equal),
                root: &root,
                loss_report: &loss_report,
                artifact_bytes,
                health: health_facts(&root),
            }));
            report.loss_report = loss_report;
            report.commit = Some(*commit);
        }
        ImportOutcome::Blocking {
            artifact_hash,
            loss_report,
            unaccepted,
            fidelity,
            binding_id,
            binding_version,
        } => {
            let root = reread_root(
                state.store_for(identity).as_ref(),
                &artifact_hash,
                &binding_id,
                &binding_version,
            )?;
            let bytes = retained_bytes(state, identity, &artifact_hash)?;
            let artifact_bytes = bytes.len() as u64;
            report.format_label = detect(&bytes, &report.file_name).label().to_string();
            report.artifact_hash = artifact_hash;
            report.binding = format!("{}@{}", binding_id, binding_version);
            report.artifact_bytes = artifact_bytes;
            report.facts = Some(OnboardFacts::from_engine(FactsInput {
                format_label: &report.format_label,
                binding: &report.binding,
                direction: direction_of(&binding_id, &binding_version),
                round_trip: fidelity.as_ref().map(|measured| measured.diff.equal),
                root: &root,
                loss_report: &loss_report,
                artifact_bytes,
                health: None,
            }));
            report.loss_report = loss_report;
            report.unaccepted = unaccepted;
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use binding::MappingVerdict;

    fn mapping(subject: &str, verdict: MappingVerdict, note: &str) -> Mapping {
        Mapping {
            subject: subject.to_string(),
            verdict,
            note: note.to_string(),
        }
    }

    #[test]
    fn detection_prefers_the_bytes_over_the_file_name() {
        assert_eq!(
            detect(b"PK\x03\x04rest", "CoffeeMachine.mdzip"),
            Format::Cameo
        );
        assert_eq!(
            detect(b"<?xml version='1.0'?><xmi:XMI xmlns:uml='x'>", "model.xml"),
            Format::Xmi
        );
        assert_eq!(
            detect(b"package Coffee {\n  part def Machine;\n}", "model.txt"),
            Format::SysmlV2
        );
        assert!(matches!(detect(b"hello", "notes.txt"), Format::Unknown(_)));
    }

    #[test]
    fn a_project_name_is_derived_from_the_file_and_obeys_the_name_rule() {
        assert_eq!(derived_project_name("coffee-machine.xmi"), "coffee-machine");
        assert_eq!(
            derived_project_name("C:\\models\\Coffee Machine.mdzip"),
            "Coffee-Machine"
        );
        assert_eq!(derived_project_name("model.xmi"), "model");
        assert_eq!(derived_project_name("...xmi"), "model");
        assert_eq!(
            derived_project_name(&format!("{}.xmi", "a".repeat(90))).len(),
            64
        );
    }

    /// A REAL DEFLATE stream and a REAL zip container, both produced by .NET's
    /// System.IO.Compression and written down here verbatim. Unpacking is therefore checked
    /// against streams this code did not produce.
    const DEFLATE_FIXTURE: &str = "s6nIzbSK8PVUqMjNySu2AvJs1TNKSgqs9PXLy8v18nPT9fKL0vWLC1KT9YHK9I0MDI0NDQwM1aEaSnNzcGsI9fVBaLCzAaq18s1PSc1RyEvMTbVVT85PS0tN1c1NTM7IzEsFmZhplZliqx5vqK5PgmojklQbg1TrQ31tBwA=";
    const ZIP_FIXTURE: &str = "UEsDBBQAAAAIADNJNl2pTzqNfQAAAP8AAAAJAAAAbW9kZWwueG1ps6nIzbSK8PVUqMjNySu2AvJs1TNKSgqs9PXLy8v18nPT9fKL0vWLC1KT9YHK9I0MDI0NDQwM1aEaSnNzcGsI9fVBaLCzAaq18s1PSc1RyEvMTbVVT85PS0tN1c1NTM7IzEsFmZhplZliqx5vqK5PgmojklQbg1TrQ31tBwBQSwECFAAUAAAACAAzSTZdqU86jX0AAAD/AAAACQAAAAAAAAAAAAAAAAAAAAAAbW9kZWwueG1pUEsFBgAAAAABAAEANwAAAKQAAAAAAA==";
    const FIXTURE_TEXT: &str = "<xmi:XMI xmlns:xmi='http://www.omg.org/spec/XMI/20131001' xmlns:uml='http://www.omg.org/spec/UML/20131001'><uml:Model name='coffee-machine' xmi:id='_1'/><uml:Model name='coffee-machine' xmi:id='_2'/><uml:Model name='coffee-machine' xmi:id='_3'/></xmi:XMI>";

    fn fixture(encoded: &str) -> Vec<u8> {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .expect("the fixture must be valid base64")
    }

    /// The smallest zip that STORES one entry without compressing it, written here so the
    /// stored path is covered by a container this code built rather than by the reader under
    /// test round-tripping its own writer.
    fn stored_zip(name: &str, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        let central = out.len();
        out.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        let central_size = out.len() - central;
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&(central_size as u32).to_le_bytes());
        out.extend_from_slice(&(central as u32).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    #[test]
    fn a_real_deflate_stream_and_a_real_cameo_container_are_unpacked() {
        let deflated = fixture(DEFLATE_FIXTURE);
        assert_eq!(
            inflate(&deflated, FIXTURE_TEXT.len()).unwrap(),
            FIXTURE_TEXT.as_bytes(),
            "the deflate decoder must reproduce what a real compressor wrote"
        );

        // The container is exactly what a modelling tool writes: a zip whose model entry is
        // deflated. This is the path a dropped .mdzip takes.
        let container = fixture(ZIP_FIXTURE);
        assert!(is_zip(&container));
        let extracted = extract_cameo(&container).unwrap();
        assert_eq!(extracted, FIXTURE_TEXT.as_bytes());
        // The extracted document is then detected as XMI, so the drop routes it to the reader.
        assert_eq!(detect(&extracted, "model.xmi"), Format::Xmi);

        // A stored container takes the other arm of the same function.
        let stored = stored_zip("model.xmi", FIXTURE_TEXT.as_bytes());
        assert_eq!(
            extract_cameo(&stored).unwrap(),
            FIXTURE_TEXT.as_bytes(),
            "a stored entry is copied byte for byte"
        );
    }

    #[test]
    fn a_damaged_deflate_stream_is_refused_rather_than_trusted() {
        let deflated = fixture(DEFLATE_FIXTURE);
        // A declared size the stream cannot produce is a refusal, not a short read.
        assert!(inflate(&deflated, FIXTURE_TEXT.len() + 1).is_err());
        assert!(inflate(&deflated, FIXTURE_TEXT.len() - 1).is_err());
        // A truncated stream ends cleanly with an error.
        assert!(inflate(&deflated[..deflated.len() / 2], FIXTURE_TEXT.len()).is_err());
        // A container whose bytes are not a zip is refused with something to read.
        let not_a_zip = b"PK\x03\x04garbage".to_vec();
        assert!(extract_cameo(&not_a_zip).is_err());
    }

    #[test]
    fn loss_classes_partition_the_blocking_set_and_name_the_engines_own_constructs() {
        let report = LossReport {
            binding: binding_infos("sysml-v1-xmi", "2.4"),
            mappings: vec![
                mapping("uml:Property _a", MappingVerdict::Unmappable, "x"),
                mapping("uml:Property _b", MappingVerdict::Unmappable, "x"),
                mapping("uml:Class _c", MappingVerdict::Unmappable, "x"),
                mapping("uml:Class _d", MappingVerdict::Lossy, "x"),
                mapping("uml:Model _e", MappingVerdict::Lossy, "x"),
                mapping("uml:Model _f", MappingVerdict::Exact, "declaration: root"),
            ],
            artifact_hash: String::new(),
        };
        let classes = loss_classes(&report, 5);
        assert_eq!(classes.len(), 3, "three classes, not one per entry");
        assert_eq!(classes[0].name, "uml:Class");
        assert_eq!(classes[0].count, 2);
        assert_eq!(classes[1].name, "uml:Property");
        assert_eq!(classes[1].count, 2);
        assert_eq!(classes[2].name, "uml:Model");
        assert_eq!(classes[2].count, 1);
        // The classes partition the blocking set exactly: no entry is dropped and none is
        // counted twice, which is what lets the summary's class counts be trusted.
        let total: u64 = classes.iter().map(|class| class.count).sum();
        assert_eq!(total, report.blocking().len() as u64);
    }
}
