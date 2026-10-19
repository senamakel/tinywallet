//! Tests for the wallet storage schema: its declarations compose, and the v1
//! `x402` records migrate into the v2 collection and stream.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

use std::sync::Arc;

use serde_json::json;
use tinystoragedrivers_core::{
    Blocking, ErrorKind, Item, MemoryStorage, MigrateOptions, Precondition, Registry, Scope,
    ScopedStorage, StorageBackend,
};

const V1_PAYMENT_MS: i64 = 1_773_576_000_000; // 2026-03-15T12:00:00Z

fn v1_payment() -> Value {
    json!({
        "id": "pay-1",
        "url": "https://api.example.com/data",
        "asset": "USDC",
        "amountAtomic": 2500,
        "amountDisplay": "0.002500 USDC",
        "recipient": "Recipient",
        "network": "eip155:8453",
        "txSignature": "0xabc",
        "status": "settled",
        "timestamp": "2026-03-15T12:00:00Z",
        "authorizationValidUntil": "2026-03-15T12:05:00.250Z",
        "sessionId": "session-a",
        "threadId": "thread-1",
        "committed_hold": "owner:1"
    })
}

fn scoped() -> (MemoryStorage, ScopedStorage) {
    let backend = MemoryStorage::new();
    let scoped = backend
        .for_scope(&Scope::local().module("wallet").unwrap())
        .unwrap();
    (backend, scoped)
}

/// Declare the v2 items on `backend` and run the steps over `scoped`.
fn try_migrate(
    backend: &MemoryStorage,
    scoped: &ScopedStorage,
) -> tinystoragedrivers_core::Result<Registry> {
    let registry = Registry::compose([storage_schema()]).unwrap();
    let bridge = Blocking::new().unwrap();
    let backend = backend.clone();
    let scoped = scoped.clone();
    bridge
        .run(async move {
            registry.prepare(&backend).await?;
            registry.migrate(&scoped, &MigrateOptions::new()).await?;
            Ok(registry)
        })
        .unwrap()
}

fn migrate(backend: &MemoryStorage, scoped: &ScopedStorage) -> Registry {
    try_migrate(backend, scoped).unwrap()
}

fn put_v1(scoped: &ScopedStorage, collection: &'static str, id: &'static str, body: Value) {
    let documents = Arc::clone(scoped.documents());
    Blocking::new()
        .unwrap()
        .run(async move {
            documents
                .ensure_collection(&CollectionSpec::new(collection))
                .await
                .unwrap();
            documents
                .put(collection, id, body, Precondition::None)
                .await
                .unwrap();
        })
        .unwrap();
}

fn append_v1(scoped: &ScopedStorage, entries: Vec<Value>) {
    let streams = Arc::clone(scoped.streams());
    Blocking::new()
        .unwrap()
        .run(async move {
            for entry in entries {
                streams.append("x402.payments", entry).await.unwrap();
            }
        })
        .unwrap();
}

#[test]
fn the_schema_composes_and_declares_module_scoped_wallet_items() {
    let schema = storage_schema();
    assert_eq!(schema.owner.as_str(), "wallet");
    Registry::compose([schema.clone()]).unwrap();

    let mut names: Vec<(String, ScopeClass)> = schema
        .items
        .iter()
        .map(|item| (item.name().to_owned(), item.scope()))
        .collect();
    names.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        names,
        vec![
            ("wallet.budgets".to_owned(), ScopeClass::Module),
            ("wallet.payments".to_owned(), ScopeClass::Module),
        ]
    );
    assert!(
        schema
            .items
            .iter()
            .any(|item| matches!(item, Item::Collection { version: 2, .. }))
    );
}

#[test]
fn v1_payments_migrate_to_snake_case_millisecond_entries() {
    let (backend, scoped) = scoped();
    append_v1(&scoped, vec![v1_payment()]);

    migrate(&backend, &scoped);

    let streams = Arc::clone(scoped.streams());
    let (migrated, legacy_left) = Blocking::new()
        .unwrap()
        .run(async move {
            (
                streams.read_window("wallet.payments", 0, 10).await.unwrap(),
                streams.len("x402.payments").await.unwrap(),
            )
        })
        .unwrap();
    assert_eq!(legacy_left, 0, "the v1 stream is removed");
    assert_eq!(migrated.len(), 1);
    assert_eq!(
        migrated[0].value,
        json!({
            "id": "pay-1",
            "url": "https://api.example.com/data",
            "asset": "USDC",
            "amount_atomic": 2500,
            "amount_display": "0.002500 USDC",
            "recipient": "Recipient",
            "network": "eip155:8453",
            "tx_signature": "0xabc",
            "status": "settled",
            "timestamp_ms": V1_PAYMENT_MS,
            "authorization_valid_until_ms": V1_PAYMENT_MS + 300_250,
            "session_id": "session-a",
            "thread_id": "thread-1",
            "committed_hold": "owner:1"
        })
    );
}

#[test]
fn unreadable_v1_payment_entries_are_dropped() {
    let (backend, scoped) = scoped();
    append_v1(
        &scoped,
        vec![json!("not a record"), v1_payment(), json!({"id": 7})],
    );

    migrate(&backend, &scoped);

    let streams = Arc::clone(scoped.streams());
    let migrated = Blocking::new()
        .unwrap()
        .run(async move { streams.read_window("wallet.payments", 0, 10).await.unwrap() })
        .unwrap();
    assert_eq!(migrated.len(), 1);
}

#[test]
fn v1_holds_migrate_with_expires_ms_and_keep_their_creation_time() {
    let (backend, scoped) = scoped();
    put_v1(
        &scoped,
        "x402_budget",
        "holds",
        json!({"holds": {
            "owner:1": {"amount": 40, "expires_at_ms": 1_800_000_000_000_i64},
            "owner:2": {"amount": 7, "expires_at_ms": 1_800_000_500_000_i64}
        }}),
    );

    let documents = Arc::clone(scoped.documents());
    let created_ms = Blocking::new()
        .unwrap()
        .run(async move {
            documents
                .get("x402_budget", "holds")
                .await
                .unwrap()
                .unwrap()
                .meta
                .created_ms
        })
        .unwrap();

    migrate(&backend, &scoped);

    let documents = Arc::clone(scoped.documents());
    let (found, legacy) = Blocking::new()
        .unwrap()
        .run(async move {
            (
                documents.get("wallet.budgets", "holds").await.unwrap(),
                documents.get("x402_budget", "holds").await.unwrap(),
            )
        })
        .unwrap();
    assert!(legacy.is_none(), "the v1 collection is emptied");
    let found = found.expect("the hold document was migrated");
    assert_eq!(
        found.doc,
        json!({"holds": {
            "owner:1": {"amount": 40, "expires_ms": 1_800_000_000_000_i64},
            "owner:2": {"amount": 7, "expires_ms": 1_800_000_500_000_i64}
        }})
    );
    assert_eq!(found.meta.schema, 2);
    assert_eq!(found.meta.created_ms, created_ms);
}

#[test]
fn an_unreadable_v1_hold_document_fails_closed_and_preserves_its_original() {
    let (backend, scoped) = scoped();
    put_v1(&scoped, "x402_budget", "holds", json!({"holds": 5}));

    let documents = Arc::clone(scoped.documents());
    let original = Blocking::new()
        .unwrap()
        .run(async move {
            documents
                .get("x402_budget", "holds")
                .await
                .unwrap()
                .unwrap()
        })
        .unwrap();

    let error = try_migrate(&backend, &scoped).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidInput);

    let documents = Arc::clone(scoped.documents());
    let (legacy, migrated) = Blocking::new()
        .unwrap()
        .run(async move {
            (
                documents.get("x402_budget", "holds").await.unwrap(),
                documents.get("wallet.budgets", "holds").await.unwrap(),
            )
        })
        .unwrap();
    assert_eq!(
        legacy,
        Some(original),
        "the malformed source bytes and metadata remain"
    );
    assert!(migrated.is_none(), "no empty v2 hold document is written");
}

#[test]
fn a_malformed_v1_hold_entry_cannot_drop_an_active_reservation() {
    let (backend, scoped) = scoped();
    let legacy_doc = json!({"holds": {
        "owner:live": {"amount": 40, "expires_at_ms": 1_800_000_000_000_i64},
        "owner:malformed": {"amount": "not an integer", "expires_at_ms": 1_800_000_500_000_i64}
    }});
    put_v1(&scoped, "x402_budget", "holds", legacy_doc);

    let documents = Arc::clone(scoped.documents());
    let original = Blocking::new()
        .unwrap()
        .run(async move {
            documents
                .get("x402_budget", "holds")
                .await
                .unwrap()
                .unwrap()
        })
        .unwrap();

    let error = try_migrate(&backend, &scoped).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidInput);

    let documents = Arc::clone(scoped.documents());
    let (legacy, migrated) = Blocking::new()
        .unwrap()
        .run(async move {
            (
                documents.get("x402_budget", "holds").await.unwrap(),
                documents.get("wallet.budgets", "holds").await.unwrap(),
            )
        })
        .unwrap();
    assert_eq!(
        legacy,
        Some(original),
        "the live reservation and original metadata remain"
    );
    assert!(
        migrated.is_none(),
        "migration must not succeed with empty holds"
    );
}

#[test]
fn migrating_twice_changes_nothing() {
    let (backend, scoped) = scoped();
    append_v1(&scoped, vec![v1_payment()]);
    put_v1(
        &scoped,
        "x402_budget",
        "holds",
        json!({"holds": {"owner:1": {"amount": 1, "expires_at_ms": 5}}}),
    );
    migrate(&backend, &scoped);
    migrate(&backend, &scoped);

    let streams = Arc::clone(scoped.streams());
    let len = Blocking::new()
        .unwrap()
        .run(async move { streams.len("wallet.payments").await.unwrap() })
        .unwrap();
    assert_eq!(len, 1);
}

#[test]
fn rfc3339_instants_parse_to_epoch_milliseconds() {
    use super::payment::parse_rfc3339_ms as parse;
    assert_eq!(parse("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(parse("2026-03-15T12:00:00Z"), Some(V1_PAYMENT_MS));
    assert_eq!(
        parse("2026-03-15T12:05:00.250Z"),
        Some(V1_PAYMENT_MS + 300_250)
    );
    assert_eq!(
        parse("2026-03-15T14:00:00.1239+02:00"),
        Some(V1_PAYMENT_MS + 123)
    );
    assert_eq!(parse("2024-02-29T00:00:00Z"), Some(1_709_164_800_000));
    assert!(parse("2026-03-15T23:59:59+23:59").is_some());
    assert_eq!(parse("1969-12-31T23:59:59Z"), Some(-1_000));
    for bad in [
        "",
        "2026-03-15",
        "2026-13-15T12:00:00Z",
        "2026-02-31T00:00:00Z",
        "2025-02-29T00:00:00Z",
        "2026-04-31T00:00:00Z",
        "2026-03-15T12:00:00",
        "2026-03-15T12:00:00.Z",
        "2026-03-15T12:00:00+0200",
        "2026-03-15T12:00:00+24:00",
        "2026-03-15T12:00:00+99:99",
        "2026-03-15T12:00:00+00:60",
    ] {
        assert_eq!(parse(bad), None, "{bad}");
    }
}
