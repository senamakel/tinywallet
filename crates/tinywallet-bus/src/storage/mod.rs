//! Data-only x402 storage vocabulary shared by `TinyWallet` and its host.
//!
//! This module names the current wallet resources and stores the payment
//! record shape. Schema declarations and migration behavior belong to the
//! x402 implementation crate.

mod payment;
mod readiness;

pub use super::storage_names::{
    BUDGETS_COLLECTION, BUDGETS_VERSION, HOLDS_ID, LEGACY_BUDGET_COLLECTION,
    LEGACY_PAYMENTS_STREAM, MIGRATION_READY_ID, OWNER, PAYMENTS_IMPORT_EPOCH, PAYMENTS_STREAM,
    STORAGE_SCHEMA_VERSION,
};
pub use payment::{StoredPayment, StoredPaymentStatus};
pub use readiness::StorageMigrationReady;

#[cfg(test)]
#[path = "readiness_tests.rs"]
mod readiness_tests;
