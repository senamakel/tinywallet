//! The x402 ledger's owner-local storage declarations and migrations.

use serde_json::{Map, Value, json};
use tinystoragedrivers_api::{DocumentStore, StreamStore};
use tinystoragedrivers_types::{
    CollectionSpec, ErrorKind, Precondition, StorageError, WriteOptions,
};

#[path = "schema_payment.rs"]
mod payment;
pub(super) use payment::StoredPaymentStatus;
pub(crate) use payment::{LegacyPayment, StoredPayment, convert_legacy_payment};

/// Collection containing the single budget-hold document.
pub(super) const BUDGETS_COLLECTION: &str = "wallet.budgets";
/// Schema version for [`BUDGETS_COLLECTION`].
pub(super) const BUDGETS_VERSION: u32 = 2;
/// Identifier of the budget-hold document.
pub(super) const HOLDS_ID: &str = "holds";
/// Append-only payment history stream.
pub(super) const PAYMENTS_STREAM: &str = "wallet.payments";
/// Previous budget collection name migrated on first open.
const LEGACY_BUDGET_COLLECTION: &str = "x402_budget";
/// Previous payment stream name migrated on first open.
const LEGACY_PAYMENTS_STREAM: &str = "x402.payments";

const MIGRATIONS_COLLECTION: &str = "wallet.migrations";
const MIGRATIONS_ID: &str = "x402";
const BATCH_SIZE: usize = 512;

/// Collections the host should ensure for a module-bound x402 ledger.
///
/// The ledger applies its own migration on open; the host only ensures these
/// pure collection declarations in the scope it has already bound.
#[must_use]
pub fn collections() -> Vec<CollectionSpec> {
    vec![
        CollectionSpec::new(BUDGETS_COLLECTION),
        CollectionSpec::new(MIGRATIONS_COLLECTION),
    ]
}

/// Apply x402-owned migrations before the ledger reads or writes its history.
pub(super) async fn migrate(
    documents: &dyn DocumentStore,
    streams: &dyn StreamStore,
) -> tinystoragedrivers_types::Result<()> {
    for spec in collections() {
        documents.ensure_collection(&spec).await?;
    }
    let state = documents.get(MIGRATIONS_COLLECTION, MIGRATIONS_ID).await?;
    let current = state
        .as_ref()
        .and_then(|record| record.doc.get("version"))
        .and_then(Value::as_u64)
        .unwrap_or_default();
    if current >= BUDGETS_VERSION.into() {
        return Ok(());
    }

    migrate_budgets(documents).await?;
    migrate_payments(streams).await?;

    let next = json!({ "version": BUDGETS_VERSION });
    let precondition = state.map_or(Precondition::Absent, |record| {
        Precondition::Version(record.version)
    });
    match documents
        .put(MIGRATIONS_COLLECTION, MIGRATIONS_ID, next, precondition)
        .await
    {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::Conflict => {
            let latest = documents.get(MIGRATIONS_COLLECTION, MIGRATIONS_ID).await?;
            let completed = latest
                .as_ref()
                .and_then(|record| record.doc.get("version"))
                .and_then(Value::as_u64)
                .is_some_and(|version| version >= BUDGETS_VERSION.into());
            if completed { Ok(()) } else { Err(error) }
        }
        Err(error) => Err(error),
    }
}

async fn migrate_budgets(documents: &dyn DocumentStore) -> tinystoragedrivers_types::Result<()> {
    let Some(legacy) = documents.get(LEGACY_BUDGET_COLLECTION, HOLDS_ID).await? else {
        return Ok(());
    };
    let holds = renamed_holds(&legacy.doc).unwrap_or_default();
    let migrated = json!({ "holds": holds.clone() });
    match documents.get(BUDGETS_COLLECTION, HOLDS_ID).await? {
        Some(existing) if budget_target_matches(&existing, &migrated, legacy.meta.expires_ms) => {
            Ok(())
        }
        Some(_) => Err(StorageError::backend(
            "wallet budget migration found conflicting target data",
        )),
        None => {
            // `created_ms` belongs to the host that performs the write, so a
            // module migration may preserve application metadata, schema and
            // expiry but must let the destination assign creation time.
            let mut options = WriteOptions::new().schema(BUDGETS_VERSION);
            if let Some(expires_ms) = legacy.meta.expires_ms {
                options = options.expires_ms(expires_ms);
            }
            match documents
                .put_with(
                    BUDGETS_COLLECTION,
                    HOLDS_ID,
                    migrated.clone(),
                    Precondition::Absent,
                    options,
                )
                .await
            {
                Ok(_) => Ok(()),
                Err(error) if error.kind() == ErrorKind::Conflict => {
                    let latest = documents.get(BUDGETS_COLLECTION, HOLDS_ID).await?;
                    if latest.is_some_and(|record| {
                        budget_target_matches(&record, &migrated, legacy.meta.expires_ms)
                    }) {
                        Ok(())
                    } else {
                        Err(error)
                    }
                }
                Err(error) => Err(error),
            }
        }
    }
}

fn budget_target_matches(
    record: &tinystoragedrivers_types::Versioned<Value>,
    migrated: &Value,
    expires_ms: Option<i64>,
) -> bool {
    record.doc == *migrated
        && record.meta.schema == BUDGETS_VERSION
        && record.meta.expires_ms == expires_ms
}

impl From<super::types::PaymentStatus> for StoredPaymentStatus {
    fn from(status: super::types::PaymentStatus) -> Self {
        match status {
            super::types::PaymentStatus::Pending => Self::Pending,
            super::types::PaymentStatus::Settled => Self::Settled,
            super::types::PaymentStatus::Failed => Self::Failed,
            super::types::PaymentStatus::Denied => Self::Denied,
        }
    }
}

impl From<StoredPaymentStatus> for super::types::PaymentStatus {
    fn from(status: StoredPaymentStatus) -> Self {
        match status {
            StoredPaymentStatus::Pending => Self::Pending,
            StoredPaymentStatus::Settled => Self::Settled,
            StoredPaymentStatus::Failed => Self::Failed,
            StoredPaymentStatus::Denied => Self::Denied,
        }
    }
}

async fn migrate_payments(streams: &dyn StreamStore) -> tinystoragedrivers_types::Result<()> {
    let source_len = streams.len(LEGACY_PAYMENTS_STREAM).await?;
    let mut source = Vec::new();
    let mut offset = 0;
    while offset < source_len {
        let window = streams
            .read_window(LEGACY_PAYMENTS_STREAM, offset, BATCH_SIZE)
            .await?;
        for (index, entry) in window.iter().enumerate() {
            let expected = offset
                .checked_add(u64::try_from(index).map_err(|_| {
                    StorageError::backend("wallet payment migration source offset overflowed")
                })?)
                .ok_or_else(|| {
                    StorageError::backend("wallet payment migration source offset overflowed")
                })?;
            if entry.offset != expected {
                return Err(StorageError::backend(
                    "wallet payment migration source window was incomplete",
                ));
            }
        }
        let Some(last) = window.last() else {
            return Err(StorageError::backend(
                "wallet payment migration source ended before its reported length",
            ));
        };
        offset = last.offset.checked_add(1).ok_or_else(|| {
            StorageError::backend("wallet payment migration source offset overflowed")
        })?;
        if offset > source_len {
            return Err(StorageError::backend(
                "wallet payment migration source exceeded its reported length",
            ));
        }
        source.extend(window.into_iter().map(|entry| entry.value));
    }
    if streams.len(LEGACY_PAYMENTS_STREAM).await? != source_len {
        return Err(StorageError::backend(
            "wallet payment migration source changed while being read",
        ));
    }
    let migrated = migrate_payments_stream(LEGACY_PAYMENTS_STREAM, source)?
        .map_or_else(Vec::new, |(_, entries)| entries);
    if migrated.is_empty() {
        return Ok(());
    }

    let target_len = streams.len(PAYMENTS_STREAM).await?;
    let target_len_usize = usize::try_from(target_len).map_err(|_| {
        StorageError::backend("wallet payment migration target stream is too large")
    })?;
    let migrated_len = u64::try_from(migrated.len()).map_err(|_| {
        StorageError::backend("wallet payment migration source stream is too large")
    })?;
    if target_len > migrated_len {
        return Err(StorageError::backend(
            "wallet payment migration found unexpected target entries",
        ));
    }
    let existing = streams
        .read_window(PAYMENTS_STREAM, 0, target_len_usize)
        .await?;
    if u64::try_from(existing.len()).ok() != Some(target_len)
        || existing
            .iter()
            .enumerate()
            .any(|(index, entry)| u64::try_from(index).ok() != Some(entry.offset))
    {
        return Err(StorageError::backend(
            "wallet payment migration target prefix was incomplete",
        ));
    }
    if existing
        .iter()
        .zip(migrated.iter())
        .any(|(entry, value)| entry.value != *value)
    {
        return Err(StorageError::backend(
            "wallet payment migration found conflicting target data",
        ));
    }
    let start = target_len_usize;
    if start < migrated.len() {
        streams
            .append_batch_if_offset(PAYMENTS_STREAM, target_len, migrated[start..].to_vec())
            .await?;
    }
    Ok(())
}

fn migrate_payments_stream(
    source: &str,
    entries: Vec<Value>,
) -> tinystoragedrivers_types::Result<Option<(String, Vec<Value>)>> {
    if source != LEGACY_PAYMENTS_STREAM {
        return Ok(None);
    }
    let mut migrated = Vec::with_capacity(entries.len());
    for entry in entries {
        if let Ok(legacy) = serde_json::from_value::<LegacyPayment>(entry) {
            let payment = convert_legacy_payment(legacy);
            migrated.push(
                serde_json::to_value(payment)
                    .map_err(|error| StorageError::invalid_input(error.to_string()))?,
            );
        }
    }
    Ok(Some((PAYMENTS_STREAM.to_owned(), migrated)))
}

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
#[path = "schema_tests.rs"]
mod tests;
