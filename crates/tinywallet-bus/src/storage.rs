//! Pure storage declarations the host can ensure before starting the module.

use tinystoragedrivers_types::CollectionSpec;

/// Pure conversion of the previous x402 payment stream into the current
/// `wallet.payments` record contract.
#[cfg(feature = "wire")]
pub mod legacy_payments;

/// Collection names used by the wallet's persistent module storage.
///
/// Migration behavior remains in the x402 implementation. The host only needs
/// these declarations to ensure the collections in its already-bound scope.
#[must_use]
pub fn collections() -> Vec<CollectionSpec> {
    vec![
        CollectionSpec::new("wallet.budgets"),
        CollectionSpec::new("wallet.migrations"),
    ]
}

#[cfg(feature = "wire")]
#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
