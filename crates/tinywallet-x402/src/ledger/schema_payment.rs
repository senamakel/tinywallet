//! The owner-defined payment record and legacy converter live in the public
//! bus contract so host importers and the module migration share one mapping.

pub(crate) use tinywallet_bus::storage::legacy_payments::{
    LegacyPayment, StoredPaymentRecord as StoredPayment, StoredPaymentStatus,
    convert_legacy_payment,
};

#[cfg(test)]
#[path = "schema_payment_tests.rs"]
mod tests;
