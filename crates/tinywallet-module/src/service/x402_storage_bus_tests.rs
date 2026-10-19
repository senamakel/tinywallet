//! The x402 ledger over the real storage bus: a host connection serves a
//! memory backend, the wallet reaches it through `tinystoragedrivers-busproxy`
//! rather than a process-local registry.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

use std::sync::Arc;

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
    let bus = tinybus::transport::memory::MemoryBus::new();
    let broker = tinybus::broker::Broker::new().spawn(bus.clone());
    let host_connection = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let backend = MemoryStorage::new();
    let host = backend.for_scope(&Scope::local()).unwrap();
    let served_scope = host.clone();
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
        .set_test_ledger_storage(unreadable_stream_storage());
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
fn unreadable_stream_storage() -> LedgerStorage {
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
    LedgerStorage::new(documents, Arc::new(NoRead(Arc::clone(scoped.streams()))))
}
