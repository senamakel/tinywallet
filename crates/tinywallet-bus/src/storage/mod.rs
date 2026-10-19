//! The wallet module's storage schema (owner `wallet`) and the v1 to v2 steps
//! that bring the `x402` records forward.
//!
//! | v1 | v2 |
//! |---|---|
//! | collection `x402_budget` | collection `wallet.budgets` (hold `expires_at_ms` becomes `expires_ms`) |
//! | stream `x402.payments` | stream `wallet.payments` (`snake_case` entries, every instant `*_ms`) |
//!
//! The host composes [`storage_schema`] into its registry, and the module's
//! storage grant is generated from the same declaration. The steps are pure: a
//! re-run over the same input produces the same output. The ports crate this
//! uses declares no backend, so the contract stays synchronous and I/O-free.

mod payment;

use serde_json::{Map, Value, json};
use tinystoragedrivers_core::{
    CollectionSpec, Owner, Schema, ScopeClass, SourceRecord, Step, StorageError, TargetRecord,
};

pub use payment::{LegacyPayment, StoredPayment, StoredPaymentStatus};

pub use super::storage_names::{
    BUDGETS_COLLECTION, BUDGETS_VERSION, HOLDS_ID, LEGACY_BUDGET_COLLECTION,
    LEGACY_PAYMENTS_STREAM, OWNER, PAYMENTS_STREAM,
};

/// What the wallet module stores: the budget-hold collection and the payments
/// stream, both in the module's own scope, with the steps that migrate the v1
/// `x402_budget` collection and `x402.payments` stream into them.
///
/// The payments stream is a spending history and is never sealed or deleted.
///
/// # Panics
///
/// Never in practice: `wallet` is a valid owner name.
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

/// `x402.payments` entries to `wallet.payments` entries. An entry that no
/// longer parses is dropped: the v1 reader skipped it with a warning too, so it
/// never counted toward a budget.
fn migrate_payments_stream(
    source: &str,
    entries: Vec<Value>,
) -> tinystoragedrivers_core::Result<Option<(String, Vec<Value>)>> {
    if source != LEGACY_PAYMENTS_STREAM {
        return Ok(None);
    }
    let mut migrated = Vec::with_capacity(entries.len());
    for entry in entries {
        // An unreadable entry is dropped: the v1 reader skipped it too.
        // Steps are pure, so nothing is logged.
        if let Ok(legacy) = serde_json::from_value::<LegacyPayment>(entry) {
            migrated.push(
                serde_json::to_value(StoredPayment::from(legacy))
                    .map_err(|error| StorageError::invalid_input(error.to_string()))?,
            );
        }
    }
    Ok(Some((PAYMENTS_STREAM.to_owned(), migrated)))
}

/// The v1 hold document, renamed field by field. Reject malformed records so
/// migration preserves the source instead of silently releasing reservations.
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
#[path = "mod_tests.rs"]
mod tests;
