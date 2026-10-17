//! The storage resources the x402 ledger requires from its host.

use std::sync::LazyLock;

use tinystoragedrivers_bus_contract::{CollectionSpec, StorageNeeds};

/// The collection containing the x402 budget-hold document.
pub const X402_BUDGET_COLLECTION: &str = "x402_budget";
/// Prefix of the append-only stream containing x402 payment records.
pub const X402_PAYMENTS_STREAM_PREFIX: &str = "x402.payments";

static COLLECTIONS: LazyLock<[CollectionSpec; 1]> = LazyLock::new(|| {
    [CollectionSpec {
        name: X402_BUDGET_COLLECTION.to_owned(),
        indexes: Vec::new(),
        ttl_field: None,
        search: None,
    }]
});
static STREAM_PREFIXES: [&str; 1] = [X402_PAYMENTS_STREAM_PREFIX];

/// Return the minimal host storage declaration for x402 ledger persistence.
///
/// It requests one hold-document collection and the payment stream prefix. It
/// requires no indexes, expiry, search, blobs, or secrets. The owned collection
/// schema is initialized once so this borrowed declaration can live for
/// `'static` without copying the canonical storage-driver type.
#[must_use]
pub fn storage_needs() -> StorageNeeds<'static> {
    StorageNeeds {
        collections: &COLLECTIONS,
        blob_prefixes: &[],
        stream_prefixes: &STREAM_PREFIXES,
        secret_prefixes: &[],
    }
}
