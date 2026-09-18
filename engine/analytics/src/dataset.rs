// SPDX-License-Identifier: AGPL-3.0-or-later
//! A dataset snapshot: external tabular data, content-addressed so an answer can
//! be reproduced a year later and a changed spreadsheet cannot silently rewrite
//! history. A snapshot is built from bytes handed to it - there is no network and
//! no live sampling.

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::source::Source;

/// The content hash of a snapshot: sha256 over the exact bytes handed in,
/// hex-encoded. This is the same discipline `engine/okf` uses to content-address
/// OKF documents; for a raw blob the bytes ARE the canonical form, so the same
/// bytes always produce the same address, across runs and machines.
fn content_hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// A content-addressed snapshot of a structured source. The source - with its
/// trust level - travels with the data, so a consumer a year later still knows
/// where the numbers came from and how much to trust them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dataset {
    pub source: Source,
    /// When the source was read. A snapshot is a deliberate, reproducible record,
    /// so the read time is stamped by the caller rather than read off a wall clock.
    pub captured_at: String,
    /// Parsed rows, each a list of field strings. A header row, if present, is row
    /// zero: parsing does not special-case it.
    pub rows: Vec<Vec<String>>,
    /// The exact bytes the snapshot was built from, retained byte-for-byte so the
    /// snapshot can be fetched back by its content hash and reproduced later. A hash
    /// alone is a promise, not a snapshot: without the bytes there is nothing to
    /// reproduce.
    pub bytes: Vec<u8>,
    pub content_hash: String,
}

impl Dataset {
    /// Build a snapshot from CSV bytes. The content hash is the address of the
    /// bytes; the trust level is whatever the source already carries. The read time
    /// is left empty - use `Dataset::from_csv_at` (or the registry's
    /// `Registry::snapshot`) to stamp it.
    pub fn from_csv(source: Source, bytes: &[u8]) -> Result<Self, CsvError> {
        Self::from_csv_at(source, String::new(), bytes)
    }

    /// Build a snapshot from CSV bytes and stamp its read time.
    pub fn from_csv_at(
        source: Source,
        captured_at: String,
        bytes: &[u8],
    ) -> Result<Self, CsvError> {
        let content_hash = content_hash(bytes);
        let rows = parse_csv(bytes)?;
        Ok(Self {
            source,
            captured_at,
            rows,
            bytes: bytes.to_vec(),
            content_hash,
        })
    }
}

/// A malformed CSV is a typed error, never a panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CsvError {
    /// The bytes are not valid UTF-8, so no rows can be read.
    NotUtf8,
    /// A quoted field was opened and never closed before the end of input.
    UnterminatedQuote { row: usize },
    /// A double quote appeared inside an unquoted field.
    StrayQuote { row: usize },
    /// Characters followed a closing quote before the field's comma or newline.
    TrailingCharactersAfterQuote { row: usize },
}

impl fmt::Display for CsvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CsvError::NotUtf8 => write!(f, "CSV is not valid UTF-8"),
            CsvError::UnterminatedQuote { row } => {
                write!(f, "CSV row {} has an unterminated quoted field", row)
            }
            CsvError::StrayQuote { row } => {
                write!(f, "CSV row {} has a quote inside an unquoted field", row)
            }
            CsvError::TrailingCharactersAfterQuote { row } => {
                write!(f, "CSV row {} has characters after a closing quote", row)
            }
        }
    }
}

impl std::error::Error for CsvError {}

/// Parse RFC-4180-style CSV: rows separated by CRLF, LF or CR; fields separated by
/// commas; fields may be quoted and may contain commas and newlines; a literal
/// quote inside a quoted field is written as two quotes. Parsed by hand because a
/// snapshot is a small, well-defined format and a dependency for it is not
/// justified.
fn parse_csv(bytes: &[u8]) -> Result<Vec<Vec<String>>, CsvError> {
    let text = std::str::from_utf8(bytes).map_err(|_| CsvError::NotUtf8)?;

    #[derive(PartialEq)]
    enum State {
        FieldStart,
        Unquoted,
        Quoted,
        AfterQuote,
    }

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut state = State::FieldStart;
    // True once a row has begun. At end of input a begun row must be finalized,
    // which is what makes a trailing comma yield its trailing empty field while an
    // empty input stays zero rows.
    let mut pending = false;
    let mut row_number = 1usize;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        match state {
            State::FieldStart => match c {
                '"' => {
                    pending = true;
                    state = State::Quoted;
                }
                ',' => {
                    pending = true;
                    row.push(std::mem::take(&mut field));
                }
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    finish_row(&mut rows, &mut row, &mut field);
                    pending = false;
                    row_number += 1;
                }
                '\n' => {
                    finish_row(&mut rows, &mut row, &mut field);
                    pending = false;
                    row_number += 1;
                }
                _ => {
                    pending = true;
                    field.push(c);
                    state = State::Unquoted;
                }
            },
            State::Unquoted => match c {
                ',' => {
                    row.push(std::mem::take(&mut field));
                    state = State::FieldStart;
                }
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    finish_row(&mut rows, &mut row, &mut field);
                    pending = false;
                    row_number += 1;
                    state = State::FieldStart;
                }
                '\n' => {
                    finish_row(&mut rows, &mut row, &mut field);
                    pending = false;
                    row_number += 1;
                    state = State::FieldStart;
                }
                '"' => return Err(CsvError::StrayQuote { row: row_number }),
                _ => field.push(c),
            },
            State::Quoted => match c {
                '"' => {
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        field.push('"');
                    } else {
                        state = State::AfterQuote;
                    }
                }
                // A newline inside quotes is part of the field, not a row end.
                _ => field.push(c),
            },
            State::AfterQuote => match c {
                ',' => {
                    row.push(std::mem::take(&mut field));
                    state = State::FieldStart;
                }
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    finish_row(&mut rows, &mut row, &mut field);
                    pending = false;
                    row_number += 1;
                    state = State::FieldStart;
                }
                '\n' => {
                    finish_row(&mut rows, &mut row, &mut field);
                    pending = false;
                    row_number += 1;
                    state = State::FieldStart;
                }
                _ => return Err(CsvError::TrailingCharactersAfterQuote { row: row_number }),
            },
        }
    }

    if state == State::Quoted {
        return Err(CsvError::UnterminatedQuote { row: row_number });
    }
    if pending {
        finish_row(&mut rows, &mut row, &mut field);
    }
    Ok(rows)
}

fn finish_row(rows: &mut Vec<Vec<String>>, row: &mut Vec<String>, field: &mut String) {
    row.push(std::mem::take(field));
    rows.push(std::mem::take(row));
}
