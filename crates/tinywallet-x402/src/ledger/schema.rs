//! The ledger's storage schema lives in the contract crate
//! (`tinywallet_bus::storage`), so a host reads it without linking this
//! implementation. This module re-exports it at its old path and adds the
//! conversions between the public records and their storage shape.

pub use tinywallet_bus::storage::storage_schema;
pub(super) use tinywallet_bus::storage::{
    BUDGETS_COLLECTION, BUDGETS_VERSION, HOLDS_ID, LEGACY_BUDGET_COLLECTION,
    LEGACY_PAYMENTS_STREAM, OWNER, PAYMENTS_STREAM, StoredPayment, StoredPaymentStatus,
};

use super::types::PaymentStatus;

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

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
