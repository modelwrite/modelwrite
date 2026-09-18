// SPDX-License-Identifier: AGPL-3.0-or-later
//! The source registry and the dataset snapshot, exercised through the public
//! surface: a CSV becomes a content-addressed dataset, the same bytes always
//! produce the same address, an unregistered source is refused, and the trust
//! level travels with the data rather than being supplied at query time.

use analytics::{CsvError, Dataset, Registry, RegistryError, Source, SourceKind, TrustLevel};

fn source() -> Source {
    Source {
        id: "erp".to_string(),
        kind: SourceKind::Structured,
        trust: TrustLevel::Reported,
        description: "ERP cost export".to_string(),
    }
}

/// Build a stamped snapshot from the test source. The unstamped constructor no
/// longer exists: every snapshot must carry a read time.
fn snapshot(bytes: &[u8]) -> Dataset {
    Dataset::from_csv_at(source(), "2026-09-17T00:00:00Z".to_string(), bytes)
        .expect("valid CSV parses")
}

fn rows(spec: &[&[&str]]) -> Vec<Vec<String>> {
    spec.iter()
        .map(|r| r.iter().map(|s| s.to_string()).collect())
        .collect()
}

const CSV: &[u8] = b"id,name\nREQ-1,Widget\n";

// sha256 of the exact bytes of CSV, hex-encoded. Hardcoded so the test proves the
// address is stable across runs rather than re-deriving it with the code under test.
const CSV_HASH: &str = "e6f2c335bb68345fd38e8845fc0196cad94caf221b9c48cd36c1da1ac7a106e0";

#[test]
fn csv_becomes_a_dataset_with_a_stable_content_hash() {
    let ds = snapshot(CSV);
    assert_eq!(ds.rows, rows(&[&["id", "name"], &["REQ-1", "Widget"]]));
    assert_eq!(ds.content_hash, CSV_HASH);
    assert_eq!(ds.content_hash.len(), 64);
}

#[test]
fn the_same_bytes_produce_the_same_hash() {
    let a = snapshot(CSV);
    let b = snapshot(CSV);
    assert_eq!(a.content_hash, b.content_hash);
}

#[test]
fn different_bytes_produce_different_hashes() {
    let a = snapshot(b"a,b\n1,2\n");
    let b = snapshot(b"a,b\n1,3\n");
    assert_ne!(a.content_hash, b.content_hash);
}

#[test]
fn quoted_fields_keep_commas_newlines_and_escaped_quotes() {
    let bytes = b"id,note\nREQ-1,\"a, b\nc\"\nREQ-2,\"say \"\"hi\"\"\"\n";
    let ds = snapshot(bytes);
    assert_eq!(ds.rows[1][1], "a, b\nc");
    assert_eq!(ds.rows[2][1], "say \"hi\"");
}

#[test]
fn empty_input_is_zero_rows_not_a_panic() {
    let ds = snapshot(b"");
    assert!(ds.rows.is_empty());
}

#[test]
fn an_unterminated_quote_is_a_typed_error() {
    let err = Dataset::from_csv_at(source(), "t".to_string(), b"a,b\n\"oops\n").unwrap_err();
    assert!(matches!(err, CsvError::UnterminatedQuote { .. }));
}

#[test]
fn a_stray_quote_in_an_unquoted_field_is_a_typed_error() {
    let err = Dataset::from_csv_at(source(), "t".to_string(), b"a,b\n1,2\"3\n").unwrap_err();
    assert!(matches!(err, CsvError::StrayQuote { .. }));
}

#[test]
fn characters_after_a_closing_quote_are_a_typed_error() {
    let err = Dataset::from_csv_at(source(), "t".to_string(), b"a,b\n1,\"2\"x\n").unwrap_err();
    assert!(matches!(err, CsvError::TrailingCharactersAfterQuote { .. }));
}

#[test]
fn non_utf8_bytes_are_a_typed_error() {
    let err = Dataset::from_csv_at(source(), "t".to_string(), &[0xff, 0xfe]).unwrap_err();
    assert!(matches!(err, CsvError::NotUtf8));
}

#[test]
fn an_unregistered_source_is_refused() {
    let mut registry = Registry::new();
    let ds = snapshot(CSV);
    match registry.register_dataset(ds) {
        Err(RegistryError::UnregisteredSource(id)) => assert_eq!(id, "erp"),
        other => panic!("expected UnregisteredSource, got {:?}", other),
    }
}

#[test]
fn snapshot_refuses_an_unregistered_source() {
    let mut registry = Registry::new();
    match registry.snapshot("nope", "2026-09-17T00:00:00Z".to_string(), CSV) {
        Err(RegistryError::UnregisteredSource(id)) => assert_eq!(id, "nope"),
        other => panic!("expected UnregisteredSource, got {:?}", other),
    }
}

#[test]
fn snapshot_propagates_malformed_csv_as_a_typed_error() {
    let mut registry = Registry::new();
    registry.register(source()).unwrap();
    match registry.snapshot("erp", "t".to_string(), b"\"unterminated") {
        Err(RegistryError::Csv(CsvError::UnterminatedQuote { .. })) => {}
        other => panic!("expected Csv(UnterminatedQuote), got {:?}", other),
    }
}

#[test]
fn trust_travels_with_the_data_not_the_query() {
    let mut registry = Registry::new();
    registry
        .register(Source {
            id: "erp".to_string(),
            kind: SourceKind::Structured,
            trust: TrustLevel::Measured,
            description: "measured".to_string(),
        })
        .unwrap();

    // A snapshot is requested by id alone: the caller cannot hand in a trust level at
    // snapshot time. The trust carried is the one declared at registration.
    let ds = registry
        .snapshot("erp", "2026-09-17T00:00:00Z".to_string(), CSV)
        .unwrap();
    assert_eq!(ds.source.id, "erp");
    assert_eq!(ds.source.trust, TrustLevel::Measured);
}

#[test]
fn a_registered_source_is_accepted_and_snapshotted() {
    let mut registry = Registry::new();
    registry.register(source()).unwrap();

    registry.register_dataset(snapshot(CSV)).unwrap();

    let ds = registry
        .snapshot("erp", "2026-09-17T00:00:00Z".to_string(), CSV)
        .unwrap();
    assert_eq!(ds.captured_at, "2026-09-17T00:00:00Z");
    assert_eq!(ds.source.trust, TrustLevel::Reported);
    assert_eq!(ds.content_hash, CSV_HASH);
}

#[test]
fn a_source_is_declared_once() {
    let mut registry = Registry::new();
    registry.register(source()).unwrap();
    match registry.register(source()) {
        Err(RegistryError::DuplicateSource(id)) => assert_eq!(id, "erp"),
        other => panic!("expected DuplicateSource, got {:?}", other),
    }
}

#[test]
fn from_csv_at_stamps_the_read_time() {
    let stamped = Dataset::from_csv_at(source(), "2026-09-17T00:00:00Z".to_string(), CSV).unwrap();
    assert_eq!(stamped.captured_at, "2026-09-17T00:00:00Z");
}

#[test]
fn a_snapshot_retains_the_exact_bytes_it_was_built_from() {
    let bytes: &[u8] = b"id,name\nREQ-1,Widget\n";
    let ds = snapshot(bytes);
    assert_eq!(ds.bytes, bytes);
}

#[test]
fn a_snapshots_bytes_are_fetchable_by_its_source_and_content_hash() {
    let mut registry = Registry::new();
    registry.register(source()).unwrap();
    let bytes: &[u8] = b"id,cost\nREQ-1,100\nREQ-2,250\n";
    let ds = registry
        .snapshot("erp", "2026-09-17T00:00:00Z".to_string(), bytes)
        .unwrap();
    assert_eq!(registry.bytes(&ds.source.id, &ds.content_hash), Some(bytes));
    assert_eq!(
        registry
            .dataset(&ds.source.id, &ds.content_hash)
            .unwrap()
            .bytes,
        bytes
    );
}

#[test]
fn bytes_round_trip_exactly_including_line_endings() {
    let mut registry = Registry::new();
    registry.register(source()).unwrap();
    // CRLF line endings and a trailing newline are normalised away into rows by
    // parsing, so only byte-for-byte retention can reproduce them; reconstructing
    // from parsed rows would silently rewrite the source. The fetch must return the
    // bytes exactly as handed in.
    let bytes: &[u8] = b"id,cost\r\nREQ-1,100\r\nREQ-2,250\r\n";
    let ds = registry.snapshot("erp", "t".to_string(), bytes).unwrap();
    assert_eq!(registry.bytes(&ds.source.id, &ds.content_hash), Some(bytes));
}

#[test]
fn fetching_an_unknown_source_or_hash_returns_none_not_a_panic() {
    let mut registry = Registry::new();
    registry.register(source()).unwrap();
    let unknown = "0000000000000000000000000000000000000000000000000000000000000000";
    assert!(registry.bytes("nope", unknown).is_none());
    assert!(registry.dataset("nope", unknown).is_none());
    assert!(registry.bytes("erp", unknown).is_none());
}

#[test]
fn two_sources_sharing_bytes_do_not_overwrite_each_other() {
    let mut registry = Registry::new();
    registry
        .register(Source {
            id: "erp".to_string(),
            kind: SourceKind::Structured,
            trust: TrustLevel::Reported,
            description: "erp".to_string(),
        })
        .unwrap();
    registry
        .register(Source {
            id: "plm".to_string(),
            kind: SourceKind::Structured,
            trust: TrustLevel::Estimated,
            description: "plm".to_string(),
        })
        .unwrap();

    // Byte-identical input from two different sources: the content hash is equal,
    // but the provenance (trust) is not, so neither snapshot may overwrite the
    // other.
    let bytes: &[u8] = b"id,cost\nREQ-1,100\n";
    let erp = registry.snapshot("erp", "t1".to_string(), bytes).unwrap();
    let plm = registry.snapshot("plm", "t2".to_string(), bytes).unwrap();
    assert_eq!(erp.content_hash, plm.content_hash);

    assert_eq!(
        registry
            .dataset("erp", &erp.content_hash)
            .unwrap()
            .source
            .trust,
        TrustLevel::Reported
    );
    assert_eq!(
        registry
            .dataset("plm", &plm.content_hash)
            .unwrap()
            .source
            .trust,
        TrustLevel::Estimated
    );
    assert_eq!(registry.datasets().count(), 2);
}
