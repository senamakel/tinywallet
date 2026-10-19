//! Test support: a [`LedgerStorage`] whose payments stream can be made to fail
//! on demand, to pin down what the ledger does when storage refuses.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;
use tinystoragedrivers_core::{
    DocumentStore, Result, StorageError, StreamEntry, StreamStore, async_trait,
};

use super::persist::LedgerStorage;

/// Switches for the injected faults.
#[derive(Debug, Default)]
pub(crate) struct Faults {
    /// Every stream append fails.
    pub(crate) append: AtomicBool,
    /// Every stream read fails.
    pub(crate) read: AtomicBool,
}

#[derive(Debug)]
struct FaultyStreams {
    inner: Arc<dyn StreamStore>,
    faults: Arc<Faults>,
}

#[async_trait]
impl StreamStore for FaultyStreams {
    async fn append(&self, stream: &str, value: Value) -> Result<u64> {
        if self.faults.append.load(Ordering::SeqCst) {
            return Err(StorageError::backend("injected append failure"));
        }
        self.inner.append(stream, value).await
    }

    async fn append_batch(&self, stream: &str, values: Vec<Value>) -> Result<u64> {
        self.inner.append_batch(stream, values).await
    }

    async fn read_window(&self, stream: &str, from: u64, limit: usize) -> Result<Vec<StreamEntry>> {
        if self.faults.read.load(Ordering::SeqCst) {
            return Err(StorageError::backend("injected read failure"));
        }
        self.inner.read_window(stream, from, limit).await
    }

    async fn len(&self, stream: &str) -> Result<u64> {
        self.inner.len(stream).await
    }

    async fn truncate_before(&self, stream: &str, offset: u64) -> Result<u64> {
        self.inner.truncate_before(stream, offset).await
    }

    async fn delete_stream(&self, stream: &str) -> Result<bool> {
        self.inner.delete_stream(stream).await
    }

    async fn streams(&self, prefix: &str) -> Result<Vec<String>> {
        self.inner.streams(prefix).await
    }
}

/// In-memory storage plus the switches that break its payments stream.
pub(crate) fn faulty_storage() -> (LedgerStorage, Arc<Faults>) {
    let base = LedgerStorage::memory();
    let faults = Arc::new(Faults::default());
    let streams: Arc<dyn StreamStore> = Arc::new(FaultyStreams {
        inner: Arc::clone(&base.streams),
        faults: Arc::clone(&faults),
    });
    let documents: Arc<dyn DocumentStore> = Arc::clone(&base.documents);
    (LedgerStorage::new(documents, streams), faults)
}

/// Append raw JSON entries to the payments stream behind `storage`, as another
/// writer (or an older version of this crate) would have left them.
pub(crate) fn append_raw(storage: &LedgerStorage, values: Vec<Value>) {
    let streams = Arc::clone(&storage.streams);
    let blocking = tinystoragedrivers_core::Blocking::new().unwrap();
    blocking
        .run(async move {
            for value in values {
                streams
                    .append(super::schema::PAYMENTS_STREAM, value)
                    .await
                    .unwrap();
            }
        })
        .unwrap();
}
