// SPDX-License-Identifier: AGPL-3.0-or-later
//! The transport name map and the deterministic CSV/NDJSON serialisers.
//!
//! The schema's logical column names are snake_case AT REST (CSV, Parquet, Arrow). The REST
//! JSON wire applies the platform's camelCase convention; the mapping is mechanical
//! (snake_case -> camelCase) and documented in the OpenAPI document. CSV and NDJSON keep the
//! at-rest snake_case names, so an export is byte-identical to the CLI's offline export.

use serde_json::Value;

/// snake_case -> camelCase. basis_element_count -> basisElementCount; single-word names
/// pass through unchanged.
pub fn camel_case(snake: &str) -> String {
    let mut out = String::with_capacity(snake.len());
    let mut upper = false;
    for c in snake.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.push(c.to_ascii_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// Recursively map every object key of a row from snake_case to camelCase for the JSON wire.
/// Nested objects (the attributes list of the elements table) are walked too; their keys are
/// single words, so they pass through unchanged.
pub fn to_camel_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                out.insert(camel_case(k), to_camel_value(v));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(to_camel_value).collect()),
        other => other.clone(),
    }
}

/// Serialise a table's rows to CSV. The header is the table's snake_case columns; rows are
/// written in their given (already-sorted) order. Byte-identical across runs: no timestamps
/// and no schema-version line are added by the serialiser.
pub fn to_csv(columns: &[&str], rows: &[Value]) -> String {
    let mut out = String::new();
    out.push_str(&columns.join(","));
    out.push('\n');
    for row in rows {
        let cells: Vec<String> = columns.iter().map(|c| csv_cell(&row[c])).collect();
        out.push_str(&cells.join(","));
        out.push('\n');
    }
    out
}

fn csv_cell(value: &Value) -> String {
    let s = match value {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

/// Serialise a table's rows to NDJSON: one JSON object (snake_case keys) per line.
pub fn to_ndjson(rows: &[Value]) -> String {
    let mut out = String::new();
    for row in rows {
        out.push_str(&serde_json::to_string(row).expect("row serialises"));
        out.push('\n');
    }
    out
}
