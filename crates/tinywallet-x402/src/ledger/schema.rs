//! The x402 ledger's storage schema and v1-to-v2 migration implementation.
//!
//! The bus crate carries only the current resource names and the stored record
//! vocabulary. Legacy decoding and transformations stay with the ledger that
//! owns their behavior.

mod legacy_payment;

use serde_json::{Map, Value, json};
use tinystoragedrivers_core::{
    CollectionSpec, Owner, Schema, ScopeClass, SourceRecord, Step, StorageError, TargetRecord,
};

use super::types::PaymentStatus;
use legacy_payment::LegacyPayment;
pub(super) use tinywallet_bus::storage::{
    BUDGETS_COLLECTION, BUDGETS_VERSION, HOLDS_ID, LEGACY_BUDGET_COLLECTION,
    LEGACY_PAYMENTS_STREAM, MIGRATION_READY_ID, OWNER, PAYMENTS_IMPORT_EPOCH, PAYMENTS_STREAM,
    STORAGE_SCHEMA_VERSION, StoredPayment, StoredPaymentStatus, StorageMigrationReady,
};

impl From<PaymentStatus> for StoredPaymentStatus {
    fn from(status: PaymentStatus) -> Self {
        match status {
            PaymentStatus::Pending => Self::Pending,
            PaymentStatus::Settled => Self::Settled,
            PaymentStatus::Failed => Self::Failed,
            PaymentStatus::Denied => Self::Denied,
        }
    }
}

impl From<StoredPaymentStatus> for PaymentStatus {
    fn from(status: StoredPaymentStatus) -> Self {
        match status {
            StoredPaymentStatus::Pending => Self::Pending,
            StoredPaymentStatus::Settled => Self::Settled,
            StoredPaymentStatus::Failed => Self::Failed,
            StoredPaymentStatus::Denied => Self::Denied,
        }
    }
}

/// What the x402 ledger stores: module-scoped wallet holds and its append-only
/// payment stream, plus the v1 migrations owned by this implementation.
///
/// # Panics
///
/// Never in practice: `wallet` is a fixed valid owner name.
#[must_use]
#[allow(clippy::expect_used, reason = "`wallet` is a fixed valid owner name")]
pub fn storage_schema() -> Schema {
    Schema::new(Owner::new(OWNER).expect("`wallet` is a valid owner"))
        .collection_with(
            BUDGETS_VERSION,
            CollectionSpec::new(BUDGETS_COLLECTION),
            ScopeClass::Module,
        )
        .stream(PAYMENTS_STREAM, ScopeClass::Module)
        .step(
            Step::new(
                "budgets_v2",
                BUDGETS_COLLECTION,
                1,
                BUDGETS_VERSION,
                migrate_budgets,
            )
            .source(LEGACY_BUDGET_COLLECTION),
        )
        .step(
            Step::stream("payments_v2", PAYMENTS_STREAM, 1, 2, |source, entries| {
                migrate_payments_stream(&source, entries)
            })
            .source_prefix(LEGACY_PAYMENTS_STREAM),
        )
}

/// Move and rewrite the retained `x402.payments` streams into the stored shape.
/// Unreadable entries follow the old ledger policy and are skipped.
fn migrate_payments_stream(
    source: &str,
    entries: Vec<Value>,
) -> tinystoragedrivers_core::Result<Option<(String, Vec<Value>)>> {
    if source != LEGACY_PAYMENTS_STREAM {
        return Ok(None);
    }
    let mut migrated = Vec::with_capacity(entries.len());
    for entry in entries {
        if let Ok(legacy) = serde_json::from_value::<LegacyPayment>(entry) {
            migrated.push(
                serde_json::to_value(StoredPayment::from(legacy))
                    .map_err(|error| StorageError::invalid_input(error.to_string()))?,
            );
        }
    }
    Ok(Some((PAYMENTS_STREAM.to_owned(), migrated)))
}

/// Rename the v1 hold document without discarding malformed reservations.
fn migrate_budgets(batch: Vec<SourceRecord>) -> tinystoragedrivers_core::Result<Vec<TargetRecord>> {
    batch
        .into_iter()
        .map(|record| {
            let holds = renamed_holds(&record.body).ok_or_else(|| {
                StorageError::invalid_input(format!(
                    "legacy wallet budget document `{}` is malformed",
                    record.id
                ))
            })?;
            Ok(
                TargetRecord::new(BUDGETS_COLLECTION, record.id, json!({ "holds": holds }))
                    .created_ms(record.meta.created_ms)
                    .expires_ms(record.meta.expires_ms),
            )
        })
        .collect()
}

/// `{holds: {key: {amount, expires_at_ms}}}` as `{key: {amount, expires_ms}}`.
fn renamed_holds(body: &Value) -> Option<Map<String, Value>> {
    let holds = match body.get("holds") {
        None => return Some(Map::new()),
        Some(holds) => holds.as_object()?,
    };
    holds
        .iter()
        .map(|(key, hold)| {
            let amount = hold.get("amount")?.as_u64()?;
            let expires_ms = hold.get("expires_at_ms")?.as_i64()?;
            Some((
                key.clone(),
                json!({ "amount": amount, "expires_ms": expires_ms }),
            ))
        })
        .collect()
}

#[cfg(test)]
#[path = "schema_migration_tests.rs"]
mod migration_tests;

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
