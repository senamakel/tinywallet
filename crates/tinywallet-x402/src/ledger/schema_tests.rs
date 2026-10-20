//! The x402-owned schema migrates v1 records into the ledger's current stores.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

use std::sync::Arc;

use serde_json::{Value, json};
use tinystoragedrivers_core::{
    Blocking, MemoryStorage, MigrateOptions, Registry, Scope, ScopedStorage, StorageBackend,
};

use super::super::{LedgerStorage, PaymentLedger, SpendingBudget};

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
fn migrate(backend: &MemoryStorage, scoped: &ScopedStorage) -> Registry {
    let registry = Registry::compose([storage_schema()]).unwrap();
    let bridge = Blocking::new().unwrap();
    let backend = backend.clone();
    let scoped = scoped.clone();
    bridge
        .run(async move {
            registry.prepare(&backend).await.unwrap();
            registry
                .migrate(&scoped, &MigrateOptions::new())
                .await
                .unwrap();
            registry
        })
        .unwrap()
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
fn migrated_payments_count_against_the_budget_of_a_reopened_ledger() {
    let (backend, scoped) = scoped();
    append_v1(&scoped, vec![v1_payment()]);
    migrate(&backend, &scoped);

    let ledger = PaymentLedger::open(
        LedgerStorage::from_scoped(&scoped),
        "session-a",
        SpendingBudget::default(),
    )
    .unwrap();
    let records = ledger.recent_payments(10);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, "pay-1");
    assert_eq!(records[0].status, PaymentStatus::Settled);
    assert_eq!(records[0].amount_atomic, 2500);
    assert_eq!(records[0].timestamp.timestamp_millis(), V1_PAYMENT_MS);
    assert_eq!(records[0].thread_id.as_deref(), Some("thread-1"));
}

#[test]
fn a_ledger_refuses_to_open_while_unmigrated_payments_remain() {
    let (_backend, scoped) = scoped();
    append_v1(&scoped, vec![v1_payment()]);

    let opened = PaymentLedger::open(
        LedgerStorage::from_scoped(&scoped),
        "session-a",
        SpendingBudget::default(),
    );
    let Err(error) = opened else {
        panic!("legacy payments must block opening");
    };
    assert!(error.contains("unmigrated x402.payments"), "{error}");
}

#[test]
fn a_host_readiness_marker_does_not_bypass_direct_legacy_refusal() {
    let (_backend, scoped) = scoped();
    append_v1(&scoped, vec![v1_payment()]);
    let documents = Arc::clone(scoped.documents());
    Blocking::new()
        .unwrap()
        .run(async move {
            documents
                .put(
                    BUDGETS_COLLECTION,
                    MIGRATION_READY_ID,
                    json!({
                        "schema_version": STORAGE_SCHEMA_VERSION,
                        "import_epoch": PAYMENTS_IMPORT_EPOCH,
                    }),
                    tinystoragedrivers_core::Precondition::Absent,
                )
                .await
        })
        .unwrap()
        .unwrap();

    let opened = PaymentLedger::open(
        LedgerStorage::from_scoped(&scoped),
        "session-a",
        SpendingBudget::default(),
    );
    let Err(error) = opened else {
        panic!("a readiness marker cannot hide legacy payments from direct callers");
    };
    assert!(error.contains("unmigrated x402.payments"), "{error}");
}

#[test]
fn a_ledger_refuses_to_open_while_legacy_budget_holds_remain() {
    let (_backend, scoped) = scoped();
    let documents = Arc::clone(scoped.documents());
    Blocking::new()
        .unwrap()
        .run(async move {
            documents
                .put(
                    "x402_budget",
                    "holds",
                    json!({ "holds": {} }),
                    tinystoragedrivers_core::Precondition::None,
                )
                .await
                .unwrap();
        })
        .unwrap();

    let opened = PaymentLedger::open(
        LedgerStorage::from_scoped(&scoped),
        "session-a",
        SpendingBudget::default(),
    );
    let Err(error) = opened else {
        panic!("legacy holds must block opening");
    };
    assert!(error.contains("legacy x402 budget holds"), "{error}");
}
