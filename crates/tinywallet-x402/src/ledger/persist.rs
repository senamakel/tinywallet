//! The ledger's storage: a payments stream and one budget-hold document, both
//! through the `tinystoragedrivers-core` ports.
//!
//! The ports are async and the ledger's API is synchronous (it runs under a
//! std mutex), so every call goes through [`Blocking`], which drives the future
//! on a dedicated runtime thread. Nothing here names a backend: the host hands
//! in the handles and picks sqlite, mongodb or memory.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use log::debug;
use serde::{Deserialize, Serialize};
use tinystoragedrivers_core::{
    Blocking, CollectionSpec, DocumentStore, ErrorKind, MemoryStorage, Precondition, Scope,
    ScopedStorage, StorageBackend, StreamStore, Version,
};

use super::types::PaymentRecord;

const LOG_PREFIX: &str = "[x402::persist]";

/// Append-only stream of payment records. A payment that changes status is
/// appended again with the same id; readers fold to the latest per id.
pub(super) const PAYMENTS_STREAM: &str = tinywallet_bus::X402_PAYMENTS_STREAM_PREFIX;
/// Collection holding the single budget-hold document.
pub(super) const BUDGET_COLLECTION: &str = tinywallet_bus::X402_BUDGET_COLLECTION;
/// Id of the budget-hold document.
pub(super) const HOLDS_ID: &str = "holds";
/// How long a hold survives without being released or committed. A process that
/// dies between signing and recording must not lock budget forever.
pub(super) const HOLD_TTL_MS: i64 = 15 * 60 * 1000;
/// How often an owner of long-lived holds should call
/// [`PaymentLedger::renew_holds`](super::PaymentLedger::renew_holds): a third of
/// the TTL, so two missed beats still leave the lease alive.
pub const HOLD_RENEW_EVERY: std::time::Duration = std::time::Duration::from_secs(5 * 60);
/// Records read per stream window while loading.
const WINDOW: usize = 512;

/// The storage handles a ledger persists through.
#[derive(Debug, Clone)]
pub struct LedgerStorage {
    pub(super) documents: Arc<dyn DocumentStore>,
    pub(super) streams: Arc<dyn StreamStore>,
}

impl LedgerStorage {
    /// Persist through the given document and stream ports.
    #[must_use]
    pub fn new(documents: Arc<dyn DocumentStore>, streams: Arc<dyn StreamStore>) -> Self {
        Self { documents, streams }
    }

    /// Persist through a host-provisioned scoped handle.
    #[must_use]
    pub fn from_scoped(storage: &ScopedStorage) -> Self {
        Self::new(storage.documents().clone(), storage.streams().clone())
    }

    /// A private in-memory store. Nothing survives the process: for tests and
    /// hosts that deliberately want an ephemeral ledger.
    ///
    /// # Panics
    ///
    /// Never in practice: the fixed scope name is valid and the memory backend
    /// opens any valid scope.
    #[must_use]
    #[allow(
        clippy::expect_used,
        reason = "fixed valid scope on the memory backend"
    )]
    pub fn memory() -> Self {
        let scope = Scope::new("x402").expect("valid scope");
        let scoped = MemoryStorage::new()
            .for_scope(&scope)
            .expect("memory backend opens any scope");
        Self::from_scoped(&scoped)
    }
}

/// One budget hold as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Hold {
    pub amount: u64,
    pub expires_at_ms: i64,
}

/// The budget-hold document: every outstanding hold, across all processes
/// sharing the storage, keyed by owner-qualified id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct HoldsDoc {
    #[serde(default)]
    pub holds: BTreeMap<String, Hold>,
}

/// Internal stream envelope. Flattening keeps old payment entries readable and
/// leaves the public payment wire type unchanged.
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct StoredPayment {
    #[serde(flatten)]
    pub record: PaymentRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committed_hold: Option<String>,
}

/// Storage plus the sync bridge.
#[derive(Debug)]
pub(super) struct Persist {
    storage: LedgerStorage,
    bridge: Blocking,
}

impl Persist {
    /// Run one port call on the bridge, folding both layers of failure into a
    /// message that names the operation.
    fn run<T, F>(&self, what: &str, future: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: Future<Output = tinystoragedrivers_core::Result<T>> + Send + 'static,
    {
        match self.bridge.run(future) {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) | Err(error) => Err(format!("{what}: {error}")),
        }
    }

    pub(super) fn open(storage: LedgerStorage) -> Result<Self, String> {
        let bridge = Blocking::new().map_err(|e| format!("start storage bridge: {e}"))?;
        let persist = Self { storage, bridge };
        let documents = Arc::clone(&persist.storage.documents);
        persist.run("declare x402 budget collection", async move {
            documents
                .ensure_collection(&CollectionSpec::new(BUDGET_COLLECTION))
                .await
        })?;
        Ok(persist)
    }

    /// Records from stream offset `from` to the end, with the next offset.
    /// Entries that no longer parse are skipped with a warning.
    pub(super) fn read_from(&self, from: u64) -> Result<(Vec<StoredPayment>, u64), String> {
        let mut records = Vec::new();
        let mut next = from;
        loop {
            let streams = Arc::clone(&self.storage.streams);
            let window = self.run("read payments stream", async move {
                streams.read_window(PAYMENTS_STREAM, next, WINDOW).await
            })?;
            let Some(last) = window.last() else { break };
            next = last.offset.saturating_add(1);
            for entry in window {
                match serde_json::from_value::<StoredPayment>(entry.value) {
                    Ok(record) => records.push(record),
                    Err(error) => log::warn!(
                        "{LOG_PREFIX} corrupt record at offset {}: {error}",
                        entry.offset
                    ),
                }
            }
        }
        Ok((records, next))
    }

    /// Append one record and return its stream offset.
    pub(super) fn append(
        &self,
        record: &PaymentRecord,
        committed_hold: Option<&str>,
    ) -> Result<u64, String> {
        let value = serde_json::to_value(StoredPayment {
            record: record.clone(),
            committed_hold: committed_hold.map(str::to_owned),
        })
        .map_err(|error| error.to_string())?;
        let streams = Arc::clone(&self.storage.streams);
        self.run("append ledger record", async move {
            streams.append(PAYMENTS_STREAM, value).await
        })
    }

    /// The hold document and the version it was read at (`None` when absent).
    pub(super) fn load_holds(&self) -> Result<(Option<Version>, HoldsDoc), String> {
        let documents = Arc::clone(&self.storage.documents);
        let found = self.run("read budget holds", async move {
            documents.get(BUDGET_COLLECTION, HOLDS_ID).await
        })?;
        match found {
            None => Ok((None, HoldsDoc::default())),
            Some(versioned) => {
                let doc = serde_json::from_value::<HoldsDoc>(versioned.doc)
                    .map_err(|e| format!("budget holds document is unreadable: {e}"))?;
                Ok((Some(versioned.version), doc))
            }
        }
    }

    /// Compare-and-swap the hold document. `Ok(false)` means someone else wrote
    /// it since `version` was read; the caller re-reads and retries.
    pub(super) fn store_holds(
        &self,
        version: Option<Version>,
        doc: &HoldsDoc,
    ) -> Result<bool, String> {
        let body = serde_json::to_value(doc).map_err(|error| error.to_string())?;
        let precondition = version.map_or(Precondition::Absent, Precondition::Version);
        let documents = Arc::clone(&self.storage.documents);
        let result = self.bridge.run(async move {
            documents
                .put(BUDGET_COLLECTION, HOLDS_ID, body, precondition)
                .await
        });
        match result.map_err(|error| format!("write budget holds: {error}"))? {
            Ok(_) => Ok(true),
            Err(error)
                if matches!(error.kind(), ErrorKind::Conflict | ErrorKind::AlreadyExists) =>
            {
                debug!("{LOG_PREFIX} budget hold CAS lost the race");
                Ok(false)
            }
            Err(error) => Err(format!("write budget holds: {error}")),
        }
    }
}
