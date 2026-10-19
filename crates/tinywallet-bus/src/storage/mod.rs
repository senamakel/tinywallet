//! Data-only x402 storage vocabulary shared by `TinyWallet` and its host.
//!
//! This module names the current wallet resources and stores the payment
//! record shape. Schema declarations and migration behavior belong to the
//! x402 implementation crate.

mod payment;

pub use super::storage_names::{
    BUDGETS_COLLECTION, BUDGETS_VERSION, HOLDS_ID, LEGACY_BUDGET_COLLECTION,
    LEGACY_PAYMENTS_STREAM, OWNER, PAYMENTS_STREAM,
};
pub use payment::{StoredPayment, StoredPaymentStatus};
