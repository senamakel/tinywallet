//! Test support for deterministic failures in the ledger's storage ports.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use serde_json::Value;
use tinystoragedrivers_api::StreamStore;
use tinystoragedrivers_testkit::{Failing, FaultPlan};
use tinystoragedrivers_types::ErrorKind;

use super::blocking::Blocking;
use super::persist::LedgerStorage;

/// Fault schedules for stream append and read operations.
#[derive(Debug, Clone)]
pub(crate) struct Faults {
    /// Fails the next scheduled stream append.
    pub(crate) append: FaultPlan,
    /// Fails the next scheduled stream read.
    pub(crate) read: FaultPlan,
}

/// Real in-memory storage decorated with injectable storage-port failures.
pub(crate) fn faulty_storage() -> (LedgerStorage, Faults) {
    let base = LedgerStorage::memory();
    let faults = Faults {
        append: FaultPlan::new(),
        read: FaultPlan::new(),
    };
    let append = Arc::new(Failing::with_plan(
        Arc::clone(&base.streams),
        faults.append.clone(),
    ));
    let streams: Arc<dyn StreamStore> = Arc::new(Failing::with_plan(append, faults.read.clone()));
    (
        LedgerStorage::new(Arc::clone(&base.documents), streams),
        faults,
    )
}

/// Fail the next append or read, according to the supplied schedule.
pub(crate) fn fail_next(plan: &FaultPlan, operation: &str) {
    plan.fail_operation_nth(operation, 1, ErrorKind::Backend);
}

/// Append raw JSON entries behind a ledger as another writer would.
pub(crate) fn append_raw(storage: &LedgerStorage, values: Vec<Value>) {
    let streams = Arc::clone(&storage.streams);
    Blocking::new()
        .unwrap()
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
