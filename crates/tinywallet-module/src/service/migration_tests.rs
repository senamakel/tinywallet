//! Tests for the Wallet-owned schema declaration and transform dispatch.

use super::*;

use serde_json::{Value, json};
use tinystoragedrivers_core::{
    Meta,
    schema::{Expiry, Item, ScopeClass, SourceRecord, TransformKind},
};

fn handler() -> WalletMigrationHandler {
    WalletMigrationHandler {
        schema: tinywallet_x402::ledger::storage_schema(),
    }
}

fn v1_payment() -> Value {
    json!({
        "id": "pay-1",
        "url": "https://fixture.invalid/",
        "asset": "USDC",
        "amountAtomic": 2500,
        "amountDisplay": "0.002500 USDC",
        "recipient": "fixture recipient",
        "network": "eip155:8453",
        "txSignature": "0xabc",
        "status": "settled",
        "timestamp": "2026-03-15T12:00:00Z",
        "authorizationValidUntil": "2026-03-15T12:05:00.250Z",
        "sessionId": "migration-test",
        "threadId": "thread-1",
        "committed_hold": "test-owner"
    })
}

#[test]
fn declaration_exports_both_steps_and_only_the_wallet_items() {
    let schema = tinywallet_x402::ledger::storage_schema();
    let declaration_result = declaration_from_schema(&schema, operation_for_step);
    assert!(
        declaration_result.is_ok(),
        "the Wallet schema must produce a valid migration declaration"
    );
    let Ok(declaration) = declaration_result else {
        return;
    };
    assert_eq!(declaration.owner.as_str(), "wallet");
    assert_eq!(
        declaration
            .items
            .iter()
            .map(|item| (item.name(), item.scope()))
            .collect::<Vec<_>>(),
        vec![
            ("wallet.budgets", ScopeClass::Module),
            ("wallet.payments", ScopeClass::Module),
        ]
    );
    assert!(matches!(
        declaration.items.first(),
        Some(Item::Collection { version: 2, .. })
    ));
    assert_eq!(declaration.steps.len(), 2);
    assert_eq!(declaration.steps[0].id, BUDGETS_STEP);
    assert_eq!(declaration.steps[0].item, "wallet.budgets");
    assert_eq!(declaration.steps[0].from, 1);
    assert_eq!(declaration.steps[0].to, 2);
    assert_eq!(declaration.steps[0].source.as_deref(), Some("x402_budget"));
    assert_eq!(declaration.steps[0].transform.operation, BUDGETS_OPERATION);
    assert_eq!(declaration.steps[0].transform.version, OPERATION_VERSION);
    assert_eq!(
        declaration.steps[0].transform.kind,
        TransformKind::Documents
    );
    assert_eq!(declaration.steps[1].id, PAYMENTS_STEP);
    assert_eq!(declaration.steps[1].item, "wallet.payments");
    assert_eq!(
        declaration.steps[1].source.as_deref(),
        Some("x402.payments")
    );
    assert_eq!(declaration.steps[1].transform.operation, PAYMENTS_OPERATION);
    assert_eq!(declaration.steps[1].transform.version, OPERATION_VERSION);
    assert_eq!(declaration.steps[1].transform.kind, TransformKind::Stream);
}

#[test]
fn budget_dispatch_reuses_the_schema_closure_and_preserves_metadata() {
    let reply = handler().transform(MigrationPayload::Documents(DocumentsTransformRequest {
        operation: BUDGETS_OPERATION.to_owned(),
        version: OPERATION_VERSION,
        records: vec![SourceRecord {
            id: "holds".to_owned(),
            body: json!({"holds": {
                "test-owner": {"amount": 25, "expires_at_ms": 8_000_i64}
            }}),
            meta: Meta {
                schema: 1,
                created_ms: 11,
                updated_ms: 13,
                expires_ms: Some(99),
            },
        }],
    }));
    assert!(
        matches!(&reply, MigrationPayloadReply::Documents(_)),
        "wallet budget migration returned an unexpected response: {reply:?}"
    );
    let MigrationPayloadReply::Documents(result) = reply else {
        return;
    };
    assert!(result.is_ok(), "wallet budget migration should succeed");
    let Ok(records) = result else { return };
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].collection, "wallet.budgets");
    assert_eq!(records[0].id, "holds");
    assert_eq!(records[0].body["holds"]["test-owner"]["expires_ms"], 8_000);
    assert_eq!(records[0].created_ms, Some(11));
    assert_eq!(records[0].expires, Expiry::At(99));
}

#[test]
fn payments_dispatch_filters_unreadable_entries_through_the_schema_closure() {
    let handler = handler();
    let plan = handler.transform(MigrationPayload::StreamPlan(StreamPlanRequest {
        operation: PAYMENTS_OPERATION.to_owned(),
        version: OPERATION_VERSION,
        source: "x402.payments".to_owned(),
    }));
    assert_eq!(
        plan,
        MigrationPayloadReply::StreamPlan(Ok(Some(StreamPlan {
            target: "wallet.payments".to_owned(),
            mode: StreamMapMode::IndependentEntries,
        })))
    );

    let entries = handler.transform(MigrationPayload::StreamEntries(StreamEntriesRequest {
        operation: PAYMENTS_OPERATION.to_owned(),
        version: OPERATION_VERSION,
        entries: vec![v1_payment(), json!("unreadable"), json!({"id": 7})],
    }));
    assert!(
        matches!(&entries, MigrationPayloadReply::StreamEntries(_)),
        "wallet payment migration returned an unexpected response: {entries:?}"
    );
    let MigrationPayloadReply::StreamEntries(result) = entries else {
        return;
    };
    assert!(result.is_ok(), "wallet payment migration should succeed");
    let Ok(entries) = result else { return };
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["id"], "pay-1");
    assert_eq!(entries[0]["amount_atomic"], 2500);
}

#[test]
fn migration_dispatch_fails_closed_for_unknown_operations_and_versions() {
    let wrong_operation =
        handler().transform(MigrationPayload::Documents(DocumentsTransformRequest {
            operation: "wallet.unknown".to_owned(),
            version: OPERATION_VERSION,
            records: Vec::new(),
        }));
    assert!(matches!(
        wrong_operation,
        MigrationPayloadReply::Documents(Err(TransformError {
            kind: TransformErrorKind::UnsupportedOperation,
            ..
        }))
    ));

    let wrong_version = handler().transform(MigrationPayload::StreamPlan(StreamPlanRequest {
        operation: PAYMENTS_OPERATION.to_owned(),
        version: OPERATION_VERSION + 1,
        source: "x402.payments".to_owned(),
    }));
    assert!(matches!(
        wrong_version,
        MigrationPayloadReply::StreamPlan(Err(TransformError {
            kind: TransformErrorKind::UnsupportedVersion,
            ..
        }))
    ));

    let wrong_family = handler().transform(MigrationPayload::BlobTarget(
        tinystoragedrivers_core::schema::delivery::BlobTargetRequest {
            operation: PAYMENTS_OPERATION.to_owned(),
            version: OPERATION_VERSION,
            source: "wallet/legacy".to_owned(),
        },
    ));
    assert!(matches!(
        wrong_family,
        MigrationPayloadReply::BlobTarget(Err(TransformError {
            kind: TransformErrorKind::UnsupportedOperation,
            ..
        }))
    ));
}
