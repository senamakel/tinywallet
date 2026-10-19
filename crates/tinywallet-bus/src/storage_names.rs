//! Canonical current and legacy names for wallet-owned storage resources.

/// The schema owner: every current name below starts with `wallet.`.
#[cfg(feature = "storage")]
pub const OWNER: &str = "wallet";
/// Collection holding the single budget-hold document.
pub const BUDGETS_COLLECTION: &str = "wallet.budgets";
/// Body version of [`BUDGETS_COLLECTION`].
#[cfg(feature = "storage")]
pub const BUDGETS_VERSION: u32 = 2;
/// Id of the budget-hold document.
#[cfg(feature = "storage")]
pub const HOLDS_ID: &str = "holds";
/// Append-only stream of payment records.
pub const PAYMENTS_STREAM: &str = "wallet.payments";

/// v1 name of [`BUDGETS_COLLECTION`], retained as a migration source.
#[cfg(feature = "storage")]
pub const LEGACY_BUDGET_COLLECTION: &str = "x402_budget";
/// v1 name of [`PAYMENTS_STREAM`], retained as a migration source.
#[cfg(feature = "storage")]
pub const LEGACY_PAYMENTS_STREAM: &str = "x402.payments";
