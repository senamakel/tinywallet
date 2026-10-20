//! The x402 ledger over the real storage bus: a host connection serves a
//! memory backend, the wallet reaches it through `tinystoragedrivers-busproxy`
//! rather than a process-local registry.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tinybus::Connection;
use tinystoragedrivers_busproxy::{PerCallerEndpoint, serve};
use tinystoragedrivers_client::StorageServer;
use tinystoragedrivers_core::{
    DocumentStore, MemoryStorage, Precondition, Result as StorageResult, Scope, ScopedStorage,
    StorageBackend, StreamEntry, StreamStore, async_trait,
};
use tinywallet_bus::wire::{X402CloseRequest, X402ConfigureRequest, X402SummaryRequest};
use tinywallet_x402::ledger::{LedgerStorage, PaymentLedger, SpendingBudget};

struct Fixture {
    wallet: crate::service::Wallet,
    /// The host's own handle on the scope the module is bound to.
    host: ScopedStorage,
    _served: tinystoragedrivers_busproxy::ServedStorage,
    broker: tokio::task::JoinHandle<tinybus::Result<()>>,
}

/// A broker with a storage host (memory backend, every caller bound to the
/// local scope) and a wallet connected to it.
async fn fixture() -> Fixture {
    let backend = MemoryStorage::new();
    let host = backend.for_scope(&Scope::local()).unwrap();
    // This backend is fresh and host-owned; model the host's completed schema
    // and import setup explicitly before serving the module.
    host_write_readiness(
        &host,
        serde_json::json!({
            "schema_version": 2,
            "import_epoch": "x402-payments-jsonl-v1",
        }),
    )
    .await;
    fixture_with_storage(host.clone(), host).await
}

/// A fixture whose storage ports accept only the names in `TinyWallet`'s current
/// storage declaration and record every request outside that grant.
async fn current_only_fixture() -> (Fixture, Arc<AtomicUsize>) {
    let (fixture, denied, _backend) = current_only_fixture_with_backend().await;
    (fixture, denied)
}

async fn current_only_fixture_with_backend() -> (Fixture, Arc<AtomicUsize>, MemoryStorage) {
    let backend = MemoryStorage::new();
    let host = backend.for_scope(&Scope::local()).unwrap();
    let denied = Arc::new(AtomicUsize::new(0));
    let current = ScopedStorage::new(
        host.scope().clone(),
        host.driver(),
        Arc::new(CurrentOnlyDocuments {
            inner: Arc::clone(host.documents()),
            denied: Arc::clone(&denied),
        }),
        Arc::new(CurrentOnlyStreams {
            inner: Arc::clone(host.streams()),
            denied: Arc::clone(&denied),
        }),
        Arc::clone(host.blobs()),
    );
    (fixture_with_storage(host, current).await, denied, backend)
}

async fn host_write_readiness(storage: &ScopedStorage, body: serde_json::Value) {
    let documents = storage.documents();
    documents
        .ensure_collection(&tinystoragedrivers_core::CollectionSpec::new(
            CURRENT_BUDGETS,
        ))
        .await
        .unwrap();
    documents
        .put_with(
            CURRENT_BUDGETS,
            "migration_ready",
            body,
            Precondition::Absent,
            tinystoragedrivers_core::WriteOptions::new().schema(2),
        )
        .await
        .unwrap();
}

async fn fixture_with_storage(host: ScopedStorage, served_scope: ScopedStorage) -> Fixture {
    let bus = tinybus::transport::memory::MemoryBus::new();
    let broker = tinybus::broker::Broker::new().spawn(bus.clone());
    let host_connection = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let endpoint =
        PerCallerEndpoint::new(move |_sender| Some(StorageServer::new(served_scope.clone())));
    let served = serve(&host_connection, Arc::new(endpoint)).await.unwrap();
    // Keep the host connection alive for the life of the fixture.
    std::mem::forget(host_connection);
    let client = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    Fixture {
        wallet: crate::service::Wallet {
            callbacks: Some(HostCallbacks::new(&client).unwrap()),
            ..crate::service::Wallet::default()
        },
        host,
        _served: served,
        broker,
    }
}

const CURRENT_BUDGETS: &str = "wallet.budgets";
const CURRENT_PAYMENTS: &str = "wallet.payments";

fn denied_storage_access(
    counter: &AtomicUsize,
    target: &str,
) -> tinystoragedrivers_core::StorageError {
    counter.fetch_add(1, Ordering::Relaxed);
    tinystoragedrivers_core::StorageError::backend(format!(
        "current-only wallet grant refuses {target}"
    ))
}

#[derive(Debug)]
struct CurrentOnlyDocuments {
    inner: Arc<dyn DocumentStore>,
    denied: Arc<AtomicUsize>,
}

impl CurrentOnlyDocuments {
    fn allow(&self, collection: &str) -> StorageResult<()> {
        if collection == CURRENT_BUDGETS {
            Ok(())
        } else {
            Err(denied_storage_access(&self.denied, collection))
        }
    }
}

#[async_trait]
impl DocumentStore for CurrentOnlyDocuments {
    fn capabilities(&self) -> tinystoragedrivers_core::Capabilities {
        self.inner.capabilities()
    }

    async fn ensure_collection(
        &self,
        spec: &tinystoragedrivers_core::CollectionSpec,
    ) -> StorageResult<()> {
        self.allow(&spec.name)?;
        self.inner.ensure_collection(spec).await
    }

    async fn get(
        &self,
        collection: &str,
        id: &str,
    ) -> StorageResult<Option<tinystoragedrivers_core::Versioned<serde_json::Value>>> {
        self.allow(collection)?;
        self.inner.get(collection, id).await
    }

    async fn put(
        &self,
        collection: &str,
        id: &str,
        doc: serde_json::Value,
        precondition: Precondition,
    ) -> StorageResult<tinystoragedrivers_core::Version> {
        self.allow(collection)?;
        self.inner.put(collection, id, doc, precondition).await
    }

    async fn put_with(
        &self,
        collection: &str,
        id: &str,
        doc: serde_json::Value,
        precondition: Precondition,
        options: tinystoragedrivers_core::WriteOptions,
    ) -> StorageResult<tinystoragedrivers_core::Version> {
        self.allow(collection)?;
        self.inner
            .put_with(collection, id, doc, precondition, options)
            .await
    }

    async fn delete(
        &self,
        collection: &str,
        id: &str,
        precondition: Precondition,
    ) -> StorageResult<bool> {
        self.allow(collection)?;
        self.inner.delete(collection, id, precondition).await
    }

    async fn query(
        &self,
        collection: &str,
        query: &tinystoragedrivers_core::Query,
    ) -> StorageResult<
        tinystoragedrivers_core::Page<tinystoragedrivers_core::Versioned<serde_json::Value>>,
    > {
        self.allow(collection)?;
        self.inner.query(collection, query).await
    }

    async fn count(
        &self,
        collection: &str,
        filter: &tinystoragedrivers_core::Filter,
    ) -> StorageResult<u64> {
        self.allow(collection)?;
        self.inner.count(collection, filter).await
    }

    async fn delete_where(
        &self,
        collection: &str,
        filter: &tinystoragedrivers_core::Filter,
    ) -> StorageResult<u64> {
        self.allow(collection)?;
        self.inner.delete_where(collection, filter).await
    }

    async fn claim(
        &self,
        collection: &str,
        filter: &tinystoragedrivers_core::Filter,
        sort: &[tinystoragedrivers_core::Sort],
        patch: &serde_json::Value,
    ) -> StorageResult<Option<tinystoragedrivers_core::Versioned<serde_json::Value>>> {
        self.allow(collection)?;
        self.inner.claim(collection, filter, sort, patch).await
    }

    async fn atomic_batch(
        &self,
        ops: Vec<tinystoragedrivers_core::WriteOp>,
    ) -> StorageResult<Vec<tinystoragedrivers_core::WriteResult>> {
        for op in &ops {
            let collection = match op {
                tinystoragedrivers_core::WriteOp::Put { collection, .. }
                | tinystoragedrivers_core::WriteOp::Delete { collection, .. } => collection,
            };
            self.allow(collection)?;
        }
        self.inner.atomic_batch(ops).await
    }

    async fn atomic_batch_checked(
        &self,
        checks: Vec<tinystoragedrivers_core::ReadCheck>,
        ops: Vec<tinystoragedrivers_core::WriteOp>,
    ) -> StorageResult<Vec<tinystoragedrivers_core::WriteResult>> {
        for check in &checks {
            self.allow(&check.collection)?;
        }
        for op in &ops {
            let collection = match op {
                tinystoragedrivers_core::WriteOp::Put { collection, .. }
                | tinystoragedrivers_core::WriteOp::Delete { collection, .. } => collection,
            };
            self.allow(collection)?;
        }
        self.inner.atomic_batch_checked(checks, ops).await
    }

    async fn search(
        &self,
        collection: &str,
        text: &str,
        limit: usize,
    ) -> StorageResult<Vec<tinystoragedrivers_core::SearchHit>> {
        self.allow(collection)?;
        self.inner.search(collection, text, limit).await
    }

    async fn drop_collection(&self, collection: &str) -> StorageResult<()> {
        self.allow(collection)?;
        self.inner.drop_collection(collection).await
    }

    async fn collections(&self, prefix: &str) -> StorageResult<Vec<String>> {
        Ok(self
            .inner
            .collections(prefix)
            .await?
            .into_iter()
            .filter(|name| name == CURRENT_BUDGETS)
            .collect())
    }
}

#[derive(Debug)]
struct CurrentOnlyStreams {
    inner: Arc<dyn StreamStore>,
    denied: Arc<AtomicUsize>,
}

impl CurrentOnlyStreams {
    fn allow(&self, stream: &str) -> StorageResult<()> {
        if stream.starts_with(CURRENT_PAYMENTS) {
            Ok(())
        } else {
            Err(denied_storage_access(&self.denied, stream))
        }
    }
}

#[async_trait]
impl StreamStore for CurrentOnlyStreams {
    async fn append(&self, stream: &str, value: serde_json::Value) -> StorageResult<u64> {
        self.allow(stream)?;
        self.inner.append(stream, value).await
    }

    async fn append_batch(
        &self,
        stream: &str,
        values: Vec<serde_json::Value>,
    ) -> StorageResult<u64> {
        self.allow(stream)?;
        self.inner.append_batch(stream, values).await
    }

    async fn append_batch_if_offset(
        &self,
        stream: &str,
        expected_next_offset: u64,
        values: Vec<serde_json::Value>,
    ) -> StorageResult<u64> {
        self.allow(stream)?;
        self.inner
            .append_batch_if_offset(stream, expected_next_offset, values)
            .await
    }

    async fn read_window(
        &self,
        stream: &str,
        from: u64,
        limit: usize,
    ) -> StorageResult<Vec<StreamEntry>> {
        self.allow(stream)?;
        self.inner.read_window(stream, from, limit).await
    }

    async fn len(&self, stream: &str) -> StorageResult<u64> {
        self.allow(stream)?;
        self.inner.len(stream).await
    }

    async fn truncate_before(&self, stream: &str, offset: u64) -> StorageResult<u64> {
        self.allow(stream)?;
        self.inner.truncate_before(stream, offset).await
    }

    async fn delete_stream(&self, stream: &str) -> StorageResult<bool> {
        self.allow(stream)?;
        self.inner.delete_stream(stream).await
    }

    async fn seal(&self, stream: &str) -> StorageResult<tinystoragedrivers_core::SealOutcome> {
        self.allow(stream)?;
        self.inner.seal(stream).await
    }

    async fn delete_sealed_stream(&self, stream: &str, sealed_ms: i64) -> StorageResult<bool> {
        self.allow(stream)?;
        self.inner.delete_sealed_stream(stream, sealed_ms).await
    }

    async fn info(
        &self,
        stream: &str,
    ) -> StorageResult<Option<tinystoragedrivers_core::StreamInfo>> {
        self.allow(stream)?;
        self.inner.info(stream).await
    }

    async fn streams(&self, prefix: &str) -> StorageResult<Vec<String>> {
        Ok(self
            .inner
            .streams(prefix)
            .await?
            .into_iter()
            .filter(|stream| stream.starts_with(CURRENT_PAYMENTS))
            .collect())
    }
}

fn config(session: &str) -> X402ConfigResponse {
    X402ConfigResponse {
        workspace_dir: "/unused".to_string(),
        session_id: session.to_string(),
        per_request_max_atomic: 100,
        daily_max_atomic: 100,
        monthly_max_atomic: 1_000,
    }
}

fn host_ledger(fixture: &Fixture) -> PaymentLedger {
    PaymentLedger::open(
        LedgerStorage::from_scoped(&fixture.host),
        "host-side",
        SpendingBudget {
            per_request_max_atomic: 100,
            daily_max_atomic: 100,
            monthly_max_atomic: 1_000,
        },
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn current_only_grant_opens_from_host_ready_current_history_without_legacy_reads() {
    let _guard = LEDGER_TEST_LOCK.lock().await;
    let (fixture, denied) = current_only_fixture().await;

    // Seed current history and a live reservation as the host after its schema
    // migration. The literal readiness row models the host's completion record
    // without coupling this regression test to a not-yet-added bus DTO.
    let mut seed = host_ledger(&fixture);
    let mut history = super::tests::pending_record("migrated-payment", None);
    history.status = PaymentStatus::Settled;
    history.amount_atomic = 60;
    seed.record_payment(history);
    let _host_hold = seed.reserve(30).unwrap().unwrap();
    let documents = Arc::clone(fixture.host.documents());
    host_write_readiness(
        &fixture.host,
        serde_json::json!({
            "schema_version": 2,
            "import_epoch": "x402-payments-jsonl-v1",
        }),
    )
    .await;

    let configured = fixture
        .wallet
        .x402_configure(X402ConfigureRequest {
            config: config("bus-current-only"),
        })
        .await;
    assert!(
        configured.is_ok(),
        "current-only configure should use the host readiness record and current data; error={configured:?}; denied accesses={}",
        denied.load(Ordering::Relaxed)
    );
    let service = configured.unwrap();
    let history = ledger::with_ledger(|ledger| ledger.recent_payments(10)).unwrap();
    assert!(history.iter().any(|record| record.id == "migrated-payment"));

    // The migrated 60 payment and 30 live hold leave exactly 10 of the daily
    // limit. These requests prove the module read current history/holds and can
    // still write its own hold through the same restricted bus path.
    assert!(
        ledger::with_ledger_mut(|ledger| ledger.reserve(11))
            .unwrap()
            .unwrap()
            .is_err()
    );
    let module_hold = ledger::reserve(10).unwrap().unwrap();
    let holds = documents
        .get(CURRENT_BUDGETS, "holds")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(holds.doc["holds"].as_object().unwrap().len(), 2);

    ledger::with_ledger_mut(|ledger| {
        ledger.record_payment(super::tests::pending_record("module-payment", None));
    })
    .unwrap();
    assert_eq!(
        fixture.host.streams().len(CURRENT_PAYMENTS).await.unwrap(),
        2
    );
    assert_eq!(denied.load(Ordering::Relaxed), 0);

    drop(module_hold);
    fixture
        .wallet
        .x402_close(X402CloseRequest { service })
        .await
        .unwrap();
    fixture.broker.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn current_only_configure_refuses_missing_or_stale_readiness_without_legacy_reads() {
    let _guard = LEDGER_TEST_LOCK.lock().await;
    let cases = [
        ("missing", None, false),
        (
            "malformed",
            Some(serde_json::json!({
                "schema_version": 2,
                "import_epoch": 17,
            })),
            false,
        ),
        (
            "unknown field",
            Some(serde_json::json!({
                "schema_version": 2,
                "import_epoch": "x402-payments-jsonl-v1",
                "host_override": true,
            })),
            false,
        ),
        (
            "wrong schema",
            Some(serde_json::json!({
                "schema_version": 1,
                "import_epoch": "x402-payments-jsonl-v1",
            })),
            false,
        ),
        (
            "wrong import epoch",
            Some(serde_json::json!({
                "schema_version": 2,
                "import_epoch": "x402-payments-jsonl-v0",
            })),
            false,
        ),
        (
            "marker in another scope",
            Some(serde_json::json!({
                "schema_version": 2,
                "import_epoch": "x402-payments-jsonl-v1",
            })),
            true,
        ),
    ];

    for (case, marker, other_scope) in cases {
        let (fixture, denied, backend) = current_only_fixture_with_backend().await;
        if let Some(marker) = marker {
            let marker_scope = if other_scope {
                backend
                    .for_scope(&Scope::new("wallet-other-scope").unwrap())
                    .unwrap()
            } else {
                fixture.host.clone()
            };
            host_write_readiness(&marker_scope, marker).await;
        }

        let configured = fixture
            .wallet
            .x402_configure(X402ConfigureRequest {
                config: config("bus-invalid-readiness"),
            })
            .await;
        assert!(configured.is_err(), "{case} readiness must fail closed");
        assert_eq!(
            denied.load(Ordering::Relaxed),
            0,
            "{case} readiness must fail before probing legacy resources"
        );
        fixture.broker.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn configure_persists_payments_and_budget_holds_on_the_host_storage() {
    let _guard = LEDGER_TEST_LOCK.lock().await;
    let fixture = fixture().await;
    let service = fixture
        .wallet
        .x402_configure(X402ConfigureRequest {
            config: config("bus-persist"),
        })
        .await
        .unwrap();

    // Through the module's global ledger: a hold, then a recorded payment.
    ledger::with_ledger_mut(|ledger| {
        ledger.reserve(30).unwrap().unwrap();
        ledger.record_payment(super::tests::pending_record("bus-payment", None));
    })
    .unwrap();

    // The host's own storage holds both, under the documented names.
    let streams = Arc::clone(fixture.host.streams());
    let documents = Arc::clone(fixture.host.documents());
    assert_eq!(streams.len("wallet.payments").await.unwrap(), 1);
    let holds = documents
        .get("wallet.budgets", "holds")
        .await
        .unwrap()
        .unwrap();
    let amounts: Vec<u64> = holds.doc["holds"]
        .as_object()
        .unwrap()
        .values()
        .map(|hold| hold["amount"].as_u64().unwrap())
        .collect();
    assert_eq!(amounts, vec![30]);

    fixture
        .wallet
        .x402_close(X402CloseRequest { service })
        .await
        .unwrap();
    fixture.broker.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_host_side_ledger_and_the_module_ledger_share_one_atomic_budget() {
    let _guard = LEDGER_TEST_LOCK.lock().await;
    let fixture = fixture().await;
    let service = fixture
        .wallet
        .x402_configure(X402ConfigureRequest {
            config: config("bus-cas"),
        })
        .await
        .unwrap();

    // The day allows 100. The module takes 70 over the bus; a second writer
    // on the host's storage must see that hold through the CAS document.
    ledger::with_ledger_mut(|ledger| ledger.reserve(70).unwrap().unwrap()).unwrap();
    let mut other = host_ledger(&fixture);
    assert!(
        other.reserve(40).unwrap().is_err(),
        "70 held over the bus plus 40 exceeds the day"
    );
    other.reserve(30).unwrap().unwrap();
    assert!(
        ledger::with_ledger_mut(|ledger| ledger.reserve(1).unwrap().is_err()).unwrap(),
        "the module sees the host-side hold on its next reservation"
    );

    fixture
        .wallet
        .x402_close(X402CloseRequest { service })
        .await
        .unwrap();
    fixture.broker.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_module_ledger_resumes_the_records_a_previous_one_wrote() {
    let _guard = LEDGER_TEST_LOCK.lock().await;
    let fixture = fixture().await;
    let first = fixture
        .wallet
        .x402_configure(X402ConfigureRequest {
            config: config("bus-resume"),
        })
        .await
        .unwrap();
    ledger::with_ledger_mut(|ledger| {
        ledger.record_payment(super::tests::pending_record("resumed", None));
    })
    .unwrap();

    // Reconfiguring replaces the in-memory ledger; the records come back from
    // the host's stream.
    let second = fixture
        .wallet
        .x402_configure(X402ConfigureRequest {
            config: config("bus-resume"),
        })
        .await
        .unwrap();
    assert_ne!(first, second);
    let payments = ledger::with_ledger(|ledger| ledger.recent_payments(10).len()).unwrap();
    assert_eq!(payments, 1);

    fixture
        .wallet
        .x402_close(X402CloseRequest { service: second })
        .await
        .unwrap();
    fixture.broker.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_reconfigure_keeps_the_previous_service_usable() {
    let _guard = LEDGER_TEST_LOCK.lock().await;
    let fixture = fixture().await;
    let first = fixture
        .wallet
        .x402_configure(X402ConfigureRequest {
            config: config("bus-keep"),
        })
        .await
        .unwrap();
    ledger::with_ledger_mut(|ledger| ledger.reserve(20).unwrap().unwrap()).unwrap();

    // The replacement's storage cannot be read, so it refuses to open.
    fixture
        .wallet
        .callbacks
        .as_ref()
        .unwrap()
        .set_test_ledger_storage(unreadable_stream_storage().await);
    let failed = fixture
        .wallet
        .x402_configure(X402ConfigureRequest {
            config: config("bus-replacement"),
        })
        .await;
    assert!(failed.is_err(), "an unreadable ledger must not open");

    let summary = fixture
        .wallet
        .x402_spending_summary(X402SummaryRequest {
            service: first.clone(),
        })
        .await
        .expect("the previous handle still answers");
    assert_eq!(summary.reserved_atomic, 20, "its ledger and holds survive");

    fixture
        .wallet
        .x402_close(X402CloseRequest { service: first })
        .await
        .unwrap();
    fixture.broker.abort();
}

// With `e2e-memory-storage` a missing host falls back to memory by design.
#[cfg(not(feature = "e2e-memory-storage"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn configure_fails_closed_when_storage_goes_away_for_the_replacement() {
    let _guard = LEDGER_TEST_LOCK.lock().await;
    // No storage host at all on this broker.
    let bus = tinybus::transport::memory::MemoryBus::new();
    let broker = tinybus::broker::Broker::new().spawn(bus.clone());
    let client = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let wallet = crate::service::Wallet {
        callbacks: Some(HostCallbacks::new(&client).unwrap()),
        ..crate::service::Wallet::default()
    };
    let result = wallet
        .x402_configure(X402ConfigureRequest {
            config: config("no-host"),
        })
        .await;
    assert!(result.is_err());
    broker.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_lease_task_extends_the_hold_expiry_in_host_storage() {
    let _guard = LEDGER_TEST_LOCK.lock().await;
    let fixture = fixture().await;
    let service = fixture
        .wallet
        .x402_configure(X402ConfigureRequest {
            config: config("bus-lease"),
        })
        .await
        .unwrap();
    ledger::with_ledger_mut(|ledger| ledger.reserve(10).unwrap().unwrap()).unwrap();
    let expiry = |doc: &serde_json::Value| -> i64 {
        doc["holds"]
            .as_object()
            .unwrap()
            .values()
            .map(|hold| hold["expires_ms"].as_i64().unwrap())
            .max()
            .unwrap()
    };
    let documents = Arc::clone(fixture.host.documents());
    let before = expiry(
        &documents
            .get("wallet.budgets", "holds")
            .await
            .unwrap()
            .unwrap()
            .doc,
    );

    // Age the stored lease deterministically instead of sleeping on the clock.
    let stored = documents
        .get("wallet.budgets", "holds")
        .await
        .unwrap()
        .unwrap();
    let mut aged = stored.doc.clone();
    for hold in aged["holds"].as_object_mut().unwrap().values_mut() {
        hold["expires_ms"] = serde_json::json!(before - 1_000);
    }
    documents
        .put(
            "wallet.budgets",
            "holds",
            aged,
            Precondition::Version(stored.version),
        )
        .await
        .unwrap();
    renew_holds_once().await;

    let after = expiry(
        &documents
            .get("wallet.budgets", "holds")
            .await
            .unwrap()
            .unwrap()
            .doc,
    );
    assert!(
        after > before - 1_000,
        "renewal moved the aged expiry forward: {} -> {after}",
        before - 1_000
    );

    fixture
        .wallet
        .x402_close(X402CloseRequest { service })
        .await
        .unwrap();
    fixture.broker.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_reservations_over_the_bus_do_not_starve_the_runtime() {
    let _guard = LEDGER_TEST_LOCK.lock().await;
    let fixture = fixture().await;
    let service = fixture
        .wallet
        .x402_configure(X402ConfigureRequest {
            config: config("bus-concurrent"),
        })
        .await
        .unwrap();

    // Eight tasks on a two-worker runtime each block on bus-backed storage.
    // The day allows 100, each asks for 25: exactly four fit.
    let tasks: Vec<_> = (0..8)
        .map(|_| tokio::spawn(async { ledger::reserve(25).expect("storage answers").ok() }))
        .collect();
    // Keep the winners alive: a dropped reservation releases its hold.
    let mut held = Vec::new();
    for task in tasks {
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(60), task)
            .await
            .expect("no deadlock")
            .unwrap();
        held.extend(outcome);
    }
    assert_eq!(held.len(), 4);
    drop(held);

    fixture
        .wallet
        .x402_close(X402CloseRequest { service })
        .await
        .unwrap();
    fixture.broker.abort();
}

/// Memory storage whose payments stream refuses every read.
async fn unreadable_stream_storage() -> LedgerStorage {
    #[derive(Debug)]
    struct NoRead(Arc<dyn StreamStore>);

    #[async_trait]
    impl StreamStore for NoRead {
        async fn append(&self, s: &str, v: serde_json::Value) -> StorageResult<u64> {
            self.0.append(s, v).await
        }
        async fn append_batch(&self, s: &str, v: Vec<serde_json::Value>) -> StorageResult<u64> {
            self.0.append_batch(s, v).await
        }
        async fn read_window(&self, _: &str, _: u64, _: usize) -> StorageResult<Vec<StreamEntry>> {
            Err(tinystoragedrivers_core::StorageError::backend(
                "injected read failure",
            ))
        }
        async fn len(&self, s: &str) -> StorageResult<u64> {
            self.0.len(s).await
        }
        async fn truncate_before(&self, s: &str, o: u64) -> StorageResult<u64> {
            self.0.truncate_before(s, o).await
        }
        async fn delete_stream(&self, s: &str) -> StorageResult<bool> {
            self.0.delete_stream(s).await
        }
        async fn streams(&self, p: &str) -> StorageResult<Vec<String>> {
            self.0.streams(p).await
        }
    }

    let scoped = MemoryStorage::new()
        .for_scope(&Scope::new("x402-unreadable").unwrap())
        .unwrap();
    let documents: Arc<dyn DocumentStore> = Arc::clone(scoped.documents());
    documents
        .ensure_collection(&tinystoragedrivers_core::CollectionSpec::new(
            CURRENT_BUDGETS,
        ))
        .await
        .unwrap();
    documents
        .put_with(
            CURRENT_BUDGETS,
            "migration_ready",
            serde_json::json!({
                "schema_version": 2,
                "import_epoch": "x402-payments-jsonl-v1",
            }),
            Precondition::Absent,
            tinystoragedrivers_core::WriteOptions::new().schema(2),
        )
        .await
        .unwrap();
    LedgerStorage::new(documents, Arc::new(NoRead(Arc::clone(scoped.streams()))))
}
