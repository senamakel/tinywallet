//! [`Reservation`]: a hold on the process-wide ledger's budget that cannot leak.

use super::global::with_ledger_mut;
use super::types::{BudgetRefusal, PaymentRecord, ReservationId};

/// Budget held for one payment between the budget check and the moment its
/// outcome is recorded.
///
/// Dropping a `Reservation` releases the hold, so an error, an early return or a
/// cancelled future cannot leave part of the budget locked. To keep the amount
/// counted, [`commit_durable`](Self::commit_durable) the outcome instead: the
/// record and the release happen under one lock, so the amount is never
/// missing from the totals and never counted twice.
#[derive(Debug)]
pub struct Reservation {
    id: Option<ReservationId>,
    amount: u64,
}

impl Reservation {
    /// The amount held, in atomic units.
    #[must_use]
    pub fn amount(&self) -> u64 {
        self.amount
    }

    /// Durably record the outcome before releasing this hold.
    ///
    /// On write failure the reservation remains active until this value is
    /// dropped, so callers can refuse to expose a signed payment whose budget
    /// accounting was not saved.
    ///
    /// # Errors
    ///
    /// Returns an error if the global ledger is unavailable or the record
    /// cannot be durably appended.
    pub fn commit_durable(mut self, record: PaymentRecord) -> Result<(), String> {
        let Some(id) = self.id else {
            return Ok(());
        };
        let result = with_ledger_mut(|ledger| ledger.try_commit_reservation(id, record))?;
        result?;
        self.id = None;
        Ok(())
    }

    /// End the hold without recording anything. The same as dropping it.
    pub fn release(self) {}
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            // A ledger that was replaced or never set up holds nothing to free.
            let _ = with_ledger_mut(|l| l.release(id));
        }
    }
}

/// Check `amount` against the process-wide ledger's budget and hold it, atomically.
///
/// # Errors
///
/// The outer error is `"x402 payment ledger not initialized"` or a storage
/// failure (nothing is held); the inner one is the [`BudgetRefusal`] that
/// stopped the amount.
pub fn reserve(amount: u64) -> Result<Result<Reservation, BudgetRefusal>, String> {
    with_ledger_mut(|l| l.reserve(amount))?.map(|held| {
        held.map(|id| Reservation {
            id: Some(id),
            amount,
        })
    })
}
