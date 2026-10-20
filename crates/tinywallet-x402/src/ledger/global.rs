//! The process-wide ledger handle.

use std::sync::{Mutex, MutexGuard, PoisonError};

use log::debug;

use super::persist::LedgerStorage;
use super::store::PaymentLedger;
use super::types::SpendingBudget;

const LOG_PREFIX: &str = "[x402::store]";
const NOT_INITIALISED: &str = "x402 payment ledger not initialized";

static GLOBAL_LEDGER: Mutex<Option<PaymentLedger>> = Mutex::new(None);

/// Storage a module-style host provisioned for later [`init_global_current`] calls that
/// do not carry their own handles (the `TinyBus` module configures the ledger
/// over a wire request that can only name a session and a budget).
static PROVISIONED: Mutex<Option<LedgerStorage>> = Mutex::new(None);

/// Provision the storage the next module-driven ledger will persist through,
/// replacing any earlier one.
pub fn provision_storage(storage: LedgerStorage) {
    *PROVISIONED.lock().unwrap_or_else(PoisonError::into_inner) = Some(storage);
    debug!("{LOG_PREFIX} ledger storage provisioned");
}

/// The provisioned storage, if the host has provided one.
#[must_use]
pub fn provisioned_storage() -> Option<LedgerStorage> {
    PROVISIONED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Forget the provisioned storage.
pub fn clear_provisioned_storage() {
    *PROVISIONED.lock().unwrap_or_else(PoisonError::into_inner) = None;
}

/// Run `f`, which blocks on storage (and on the ledger mutex), without holding
/// an async worker hostage.
///
/// On a multi-thread tokio runtime the worker hands its queue to another thread
/// first, so a storage transport that needs this same runtime to make progress
/// (the module's storage bus) cannot be starved by the callers waiting on it.
/// Anywhere else (no runtime, a current-thread runtime) `f` simply runs.
pub(crate) fn blocking<R>(f: impl FnOnce() -> R) -> R {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(f)
        }
        _ => f(),
    }
}

fn lock() -> MutexGuard<'static, Option<PaymentLedger>> {
    // A panic while holding the guard cannot leave the ledger half-written in a
    // way a reader could not tolerate, so a poisoned lock is recovered rather
    // than turning every later payment into an error.
    GLOBAL_LEDGER.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Open the ledger over `storage` and make it the process-wide one, replacing
/// any earlier one.
///
/// The budget is the caller's to choose: where limits come from (defaults,
/// environment, settings) is host policy, and so is the storage backend.
///
/// # Errors
///
/// The storage cannot be opened or read. The previous process-wide ledger, if
/// any, is left in place.
pub fn init_global(
    storage: LedgerStorage,
    session_id: &str,
    budget: SpendingBudget,
) -> Result<(), String> {
    let ledger = blocking(|| PaymentLedger::open(storage, session_id, budget))?;
    blocking(|| *lock() = Some(ledger));
    debug!("{LOG_PREFIX} global ledger initialized");
    Ok(())
}

/// Open the process-wide ledger through the current-only module storage grant.
///
/// The host must write a valid migration readiness document in this storage
/// scope only after wallet schema migration and the legacy payment import have
/// both completed. Unlike [`init_global`], this path never probes legacy names
/// that the module's current grant cannot access.
///
/// # Errors
///
/// The current ledger cannot be read or the host readiness document is missing,
/// malformed, stale, or unreadable. The previous process-wide ledger remains.
pub fn init_global_current(
    storage: LedgerStorage,
    session_id: &str,
    budget: SpendingBudget,
) -> Result<(), String> {
    let ledger = blocking(|| PaymentLedger::open_current(storage, session_id, budget))?;
    blocking(|| *lock() = Some(ledger));
    debug!("{LOG_PREFIX} current-only module ledger initialized");
    Ok(())
}

/// Drop the process-wide ledger after its owner has released all reservations.
pub fn clear_global() {
    blocking(|| *lock() = None);
    debug!("{LOG_PREFIX} global ledger cleared");
}

/// Run `f` over the process-wide ledger.
///
/// # Errors
///
/// `"x402 payment ledger not initialized"` before [`init_global`].
pub fn with_ledger<F, R>(f: F) -> Result<R, String>
where
    F: FnOnce(&PaymentLedger) -> R,
{
    blocking(|| {
        let guard = lock();
        let ledger = guard.as_ref().ok_or_else(|| NOT_INITIALISED.to_string())?;
        Ok(f(ledger))
    })
}

/// Run `f` over the process-wide ledger, mutably.
///
/// # Errors
///
/// `"x402 payment ledger not initialized"` before [`init_global`].
pub fn with_ledger_mut<F, R>(f: F) -> Result<R, String>
where
    F: FnOnce(&mut PaymentLedger) -> R,
{
    blocking(|| {
        let mut guard = lock();
        let ledger = guard.as_mut().ok_or_else(|| NOT_INITIALISED.to_string())?;
        Ok(f(ledger))
    })
}

/// Serialises the tests that touch the process-wide ledger. An async mutex, so
/// an async test can hold it across its awaits.
#[cfg(test)]
pub(crate) static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Drop the process-wide ledger, as at process start.
#[cfg(test)]
pub(crate) fn reset_global() {
    *lock() = None;
}

#[cfg(test)]
#[path = "global_tests.rs"]
mod tests;
