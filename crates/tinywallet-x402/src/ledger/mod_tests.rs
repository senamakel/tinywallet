//! Tests for the spending ledger: budgets, summaries, persistence, and the
//! process-wide handle.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Duration, TimeZone, Utc};

use super::faulty::{append_raw, faulty_storage};
use super::persist::{BUDGET_COLLECTION, HOLDS_ID, HoldsDoc, PAYMENTS_STREAM, Persist};
use super::*;
use tinystoragedrivers_core::{Blocking, Version};

const SESSION: &str = "session-a";
static NEXT_TEST_RECORD: AtomicU64 = AtomicU64::new(1);

fn budget() -> SpendingBudget {
    SpendingBudget {
        per_request_max_atomic: 500_000,
        daily_max_atomic: 2_000_000,
        monthly_max_atomic: 10_000_000,
    }
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 15, 12, 0, 0).unwrap()
}

#[test]
fn persistence_resource_names_match_the_bus_storage_declaration() {
    let needs = tinywallet_bus::storage_needs();

    assert_eq!(needs.collections.len(), 1);
    assert_eq!(needs.collections[0].name, BUDGET_COLLECTION);
    assert_eq!(needs.collections[0].indexes, Vec::new());
    assert_eq!(needs.collections[0].ttl_field, None);
    assert_eq!(needs.collections[0].search, None);
    assert_eq!(needs.stream_prefixes, [PAYMENTS_STREAM]);
    assert_eq!(needs.blob_prefixes, &[] as &[&str]);
    assert_eq!(needs.secret_prefixes, &[] as &[&str]);
}

fn record(
    amount: u64,
    status: PaymentStatus,
    timestamp: DateTime<Utc>,
    session: &str,
) -> PaymentRecord {
    PaymentRecord {
        id: format!(
            "id-{amount}-{}-{}",
            timestamp.timestamp(),
            NEXT_TEST_RECORD.fetch_add(1, Ordering::Relaxed)
        ),
        url: "https://api.example.com/data".into(),
        asset: "USDC".into(),
        amount_atomic: amount,
        amount_display: format!("{amount} atomic"),
        recipient: "Recipient".into(),
        network: "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp".into(),
        tx_signature: Some("sig123".into()),
        status,
        timestamp,
        authorization_valid_until: None,
        solana_last_valid_block_height: None,
        authorization_expired_at: None,
        authorization_expiry_observed_height: None,
        session_id: session.into(),
        thread_id: None,
    }
}

fn ledger_in(dir: &LedgerStorage) -> PaymentLedger {
    PaymentLedger::open(dir.clone(), SESSION, budget()).unwrap()
}

#[test]
fn the_default_budget_is_one_ten_and_a_hundred_usdc() {
    let b = SpendingBudget::default();
    assert_eq!(b.per_request_max_atomic, 1_000_000);
    assert_eq!(b.daily_max_atomic, 10_000_000);
    assert_eq!(b.monthly_max_atomic, 100_000_000);
}

#[test]
fn an_empty_ledger_allows_a_payment_within_the_limits() {
    let dir = LedgerStorage::memory();
    let ledger = ledger_in(&dir);
    assert_eq!(ledger.check_budget(100_000), BudgetCheck::Allowed);
    assert_eq!(ledger.check_budget_at(now(), 500_000), BudgetCheck::Allowed);
}

#[test]
fn a_payment_over_the_per_request_cap_is_refused() {
    let dir = LedgerStorage::memory();
    assert_eq!(
        ledger_in(&dir).check_budget(600_000),
        BudgetCheck::ExceedsPerRequest {
            requested: 600_000,
            cap: 500_000
        }
    );
}

#[test]
fn settled_payments_today_count_against_the_daily_budget() {
    let dir = LedgerStorage::memory();
    let mut ledger = ledger_in(&dir);
    ledger.record_payment(record(1_800_000, PaymentStatus::Settled, now(), SESSION));
    assert_eq!(
        ledger.check_budget_at(now(), 400_000),
        BudgetCheck::ExceedsDailyBudget {
            current: 1_800_000,
            cap: 2_000_000
        }
    );
    // Exactly reaching the cap is allowed; only exceeding it is not.
    assert_eq!(ledger.check_budget_at(now(), 200_000), BudgetCheck::Allowed);
}

#[test]
fn unsigned_failed_or_denied_payments_do_not_consume_the_budget() {
    let dir = LedgerStorage::memory();
    let mut ledger = ledger_in(&dir);
    for status in [PaymentStatus::Failed, PaymentStatus::Denied] {
        ledger.record_payment(record(1_800_000, status, now(), SESSION));
    }
    assert_eq!(ledger.check_budget_at(now(), 400_000), BudgetCheck::Allowed);
    assert_eq!(ledger.summary_at(now()), SpendingSummary::default());
}

#[test]
fn pending_authorizations_remain_budgeted_after_reopening_the_ledger() {
    let dir = LedgerStorage::memory();
    let mut ledger = ledger_in(&dir);
    let mut pending = record(1_800_000, PaymentStatus::Pending, now(), SESSION);
    pending.id = "signed-authorization".to_string();
    ledger.record_payment(pending);

    assert_eq!(
        ledger.check_budget_at(now(), 300_000),
        BudgetCheck::ExceedsDailyBudget {
            current: 1_800_000,
            cap: 2_000_000
        }
    );
    assert_eq!(ledger.summary_at(now()), SpendingSummary::default());

    drop(ledger);
    let reopened = ledger_in(&dir);
    assert_eq!(
        reopened.check_budget_at(now(), 300_000),
        BudgetCheck::ExceedsDailyBudget {
            current: 1_800_000,
            cap: 2_000_000
        },
        "a process restart must not release a still-authorized payment"
    );
    assert_eq!(reopened.summary_at(now()), SpendingSummary::default());
}

#[test]
fn yesterdays_payments_count_against_the_month_but_not_the_day() {
    let dir = LedgerStorage::memory();
    let mut ledger = ledger_in(&dir);
    let yesterday = now() - Duration::days(1);
    for _ in 0..5 {
        ledger.record_payment(record(
            1_900_000,
            PaymentStatus::Settled,
            yesterday,
            SESSION,
        ));
    }
    // 9.5M settled this month; the daily total is still zero.
    assert_eq!(ledger.check_budget_at(now(), 400_000), BudgetCheck::Allowed);
    assert_eq!(
        ledger.check_budget_at(now(), 500_000),
        BudgetCheck::Allowed,
        "9.5M + 0.5M reaches the monthly cap exactly"
    );
    ledger.record_payment(record(100_000, PaymentStatus::Settled, yesterday, SESSION));
    assert_eq!(
        ledger.check_budget_at(now(), 500_000),
        BudgetCheck::ExceedsMonthlyBudget {
            current: 9_600_000,
            cap: 10_000_000
        }
    );
}

#[test]
fn last_months_payments_do_not_count() {
    let dir = LedgerStorage::memory();
    let mut ledger = ledger_in(&dir);
    let last_month = now() - Duration::days(40);
    ledger.record_payment(record(500_000, PaymentStatus::Settled, last_month, SESSION));
    let summary = ledger.summary_at(now());
    assert_eq!(summary.monthly_count, 0);
    assert_eq!(summary.daily_count, 0);
    assert_eq!(summary.session_count, 1, "the session total ignores dates");
}

#[test]
fn the_summary_splits_session_day_and_month() {
    let dir = LedgerStorage::memory();
    let mut ledger = ledger_in(&dir);
    ledger.record_payment(record(100_000, PaymentStatus::Settled, now(), SESSION));
    ledger.record_payment(record(200_000, PaymentStatus::Settled, now(), SESSION));
    ledger.record_payment(record(50_000, PaymentStatus::Failed, now(), SESSION));
    ledger.record_payment(record(
        300_000,
        PaymentStatus::Settled,
        now() - Duration::days(2),
        "session-b",
    ));

    let summary = ledger.summary_at(now());
    assert_eq!(summary.session_total_atomic, 300_000);
    assert_eq!(summary.session_count, 2);
    assert_eq!(summary.daily_total_atomic, 300_000);
    assert_eq!(summary.daily_count, 2);
    assert_eq!(summary.monthly_total_atomic, 600_000);
    assert_eq!(summary.monthly_count, 3);

    // The wall-clock variant agrees on what it can: nothing is dated today.
    assert_eq!(ledger.summary().session_count, 2);
}

#[test]
fn recent_payments_are_newest_first_and_limited() {
    let dir = LedgerStorage::memory();
    let mut ledger = ledger_in(&dir);
    for amount in [1, 2, 3, 4] {
        ledger.record_payment(record(amount, PaymentStatus::Settled, now(), SESSION));
    }
    let recent = ledger.recent_payments(2);
    assert_eq!(
        recent.iter().map(|r| r.amount_atomic).collect::<Vec<_>>(),
        vec![4, 3]
    );
    assert_eq!(ledger.recent_payments(99).len(), 4);
}

#[test]
fn updating_the_budget_takes_effect() {
    let dir = LedgerStorage::memory();
    let mut ledger = ledger_in(&dir);
    assert_eq!(ledger.budget(), &budget());
    ledger.update_budget(SpendingBudget {
        per_request_max_atomic: 10,
        daily_max_atomic: 20,
        monthly_max_atomic: 30,
    });
    assert_eq!(ledger.budget().per_request_max_atomic, 10);
    assert_eq!(
        ledger.check_budget(11),
        BudgetCheck::ExceedsPerRequest {
            requested: 11,
            cap: 10
        }
    );
}

#[test]
fn records_survive_a_restart() {
    let dir = LedgerStorage::memory();
    {
        let mut ledger = ledger_in(&dir);
        ledger.record_payment(record(100_000, PaymentStatus::Settled, now(), SESSION));
        ledger.record_payment(record(200_000, PaymentStatus::Failed, now(), SESSION));
    }
    let reopened = PaymentLedger::open(dir.clone(), "session-b", budget()).unwrap();
    let recent = reopened.recent_payments(10);
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].status, PaymentStatus::Failed);
    assert_eq!(recent[1].amount_atomic, 100_000);
}

#[test]
fn memory_storage_reopens_payment_history_and_released_holds() {
    let storage = LedgerStorage::memory();
    {
        let mut ledger = ledger_in(&storage);
        ledger.record_payment(record(100_000, PaymentStatus::Settled, now(), SESSION));
        let hold = ledger.reserve_at(now(), 50_000).unwrap().unwrap();
        ledger.release(hold);
        assert_eq!(ledger.reserved_atomic(), 0);
    }

    let reopened = ledger_in(&storage);
    let payments = reopened.recent_payments(10);
    assert_eq!(payments.len(), 1);
    assert_eq!(payments[0].amount_atomic, 100_000);
    assert_eq!(reopened.reserved_atomic(), 0);
}

#[test]
fn a_record_serialises_camel_case_with_snake_case_status() {
    let json = serde_json::to_value(record(7, PaymentStatus::Denied, now(), SESSION)).unwrap();
    assert_eq!(json["amountAtomic"], 7);
    assert_eq!(json["txSignature"], "sig123");
    assert_eq!(json["sessionId"], SESSION);
    assert_eq!(json["status"], "denied");
    let back: PaymentRecord = serde_json::from_value(json).unwrap();
    assert_eq!(back.status, PaymentStatus::Denied);
}

#[test]
fn the_global_ledger_errors_until_initialised_then_works() {
    let _guard = TEST_LOCK.blocking_lock();
    reset_global();

    assert_eq!(
        with_ledger(|_| ()).unwrap_err(),
        "x402 payment ledger not initialized"
    );
    assert_eq!(
        with_ledger_mut(|_| ()).unwrap_err(),
        "x402 payment ledger not initialized"
    );

    let dir = LedgerStorage::memory();
    init_global(dir.clone(), SESSION, budget()).unwrap();
    with_ledger_mut(|l| l.record_payment(record(9, PaymentStatus::Settled, Utc::now(), SESSION)))
        .unwrap();
    assert_eq!(with_ledger(|l| l.recent_payments(5).len()).unwrap(), 1);
    assert_eq!(
        with_ledger(|l| l.budget().daily_max_atomic).unwrap(),
        2_000_000
    );

    // Initialising again replaces the ledger with the one on disk.
    init_global(dir.clone(), "session-b", SpendingBudget::default()).unwrap();
    assert_eq!(
        with_ledger(|l| l.budget().daily_max_atomic).unwrap(),
        10_000_000
    );
    reset_global();
}

#[test]
fn a_poisoned_global_lock_is_recovered() {
    let _guard = TEST_LOCK.blocking_lock();
    let dir = LedgerStorage::memory();
    init_global(dir.clone(), SESSION, budget()).unwrap();
    let _ = std::panic::catch_unwind(|| {
        let _ = with_ledger(|_| panic!("poison the ledger lock"));
    });
    assert!(with_ledger(|l| l.recent_payments(1).len()).is_ok());
    reset_global();
}

// ---------------------------------------------------------------------------
// Reservations: check and hold in one critical section
// ---------------------------------------------------------------------------

#[test]
fn a_reservation_counts_against_the_daily_budget_until_released() {
    let dir = LedgerStorage::memory();
    let mut ledger = ledger_in(&dir);
    // The day allows 2_000_000: four holds of 500_000 fit, a fifth does not.
    let ids: Vec<_> = (0..4)
        .map(|_| ledger.reserve_at(now(), 500_000).unwrap().unwrap())
        .collect();
    assert_eq!(ledger.reserved_atomic(), 2_000_000);
    assert_eq!(
        ledger.reserve_at(now(), 1).unwrap().unwrap_err(),
        BudgetRefusal::Daily {
            current: 2_000_000,
            cap: 2_000_000
        }
    );

    ledger.release(ids[0]);
    assert_eq!(ledger.reserved_atomic(), 1_500_000);
    assert!(ledger.reserve_at(now(), 500_000).unwrap().is_ok());
}

#[test]
fn a_reservation_counts_against_the_monthly_budget() {
    let dir = LedgerStorage::memory();
    let mut ledger = PaymentLedger::open(
        dir.clone(),
        SESSION,
        SpendingBudget {
            per_request_max_atomic: 500_000,
            daily_max_atomic: 10_000_000,
            monthly_max_atomic: 600_000,
        },
    )
    .unwrap();
    ledger.reserve_at(now(), 500_000).unwrap().unwrap();
    assert_eq!(
        ledger.reserve_at(now(), 500_000).unwrap().unwrap_err(),
        BudgetRefusal::Monthly {
            current: 500_000,
            cap: 600_000
        }
    );
}

#[test]
fn a_refused_reservation_holds_nothing() {
    let dir = LedgerStorage::memory();
    let mut ledger = ledger_in(&dir);
    assert_eq!(
        ledger.reserve_at(now(), 600_000).unwrap().unwrap_err(),
        BudgetRefusal::PerRequest {
            requested: 600_000,
            cap: 500_000
        }
    );
    assert_eq!(ledger.reserved_atomic(), 0);
}

#[test]
fn releasing_an_unknown_reservation_is_harmless() {
    let dir = LedgerStorage::memory();
    let mut ledger = ledger_in(&dir);
    let id = ledger.reserve_at(now(), 1_000).unwrap().unwrap();
    ledger.release(id);
    ledger.release(id);
    assert_eq!(ledger.reserved_atomic(), 0);
}

#[test]
fn committing_a_reservation_records_the_payment_and_frees_the_hold() {
    let dir = LedgerStorage::memory();
    let mut ledger = ledger_in(&dir);
    let id = ledger.reserve(400_000).unwrap().unwrap();

    ledger.commit_reservation(
        id,
        record(400_000, PaymentStatus::Settled, Utc::now(), SESSION),
    );

    assert_eq!(ledger.reserved_atomic(), 0);
    let summary = ledger.summary();
    assert_eq!(
        summary.daily_total_atomic, 400_000,
        "counted once, as settled"
    );
    assert_eq!(ledger.recent_payments(5).len(), 1);
}

#[test]
fn the_ledger_reports_its_own_session_id() {
    let dir = LedgerStorage::memory();
    assert_eq!(ledger_in(&dir).session_id(), SESSION);
}

#[test]
fn a_global_reservation_is_released_when_dropped() {
    let _guard = TEST_LOCK.blocking_lock();
    let dir = LedgerStorage::memory();
    init_global(dir.clone(), SESSION, budget()).unwrap();

    let held = reserve(500_000).unwrap().unwrap();
    assert_eq!(held.amount(), 500_000);
    assert_eq!(
        with_ledger(PaymentLedger::reserved_atomic).unwrap(),
        500_000
    );
    drop(held);
    assert_eq!(with_ledger(PaymentLedger::reserved_atomic).unwrap(), 0);
    reset_global();
}

#[test]
fn a_global_reservation_can_be_released_explicitly() {
    let _guard = TEST_LOCK.blocking_lock();
    let dir = LedgerStorage::memory();
    init_global(dir.clone(), SESSION, budget()).unwrap();

    let held = reserve(500_000).unwrap().unwrap();
    held.release();
    assert_eq!(with_ledger(PaymentLedger::reserved_atomic).unwrap(), 0);
    reset_global();
}

#[test]
fn a_global_reservation_committed_is_recorded_once() {
    let _guard = TEST_LOCK.blocking_lock();
    let dir = LedgerStorage::memory();
    init_global(dir.clone(), SESSION, budget()).unwrap();

    let held = reserve(500_000).unwrap().unwrap();
    held.commit(record(500_000, PaymentStatus::Settled, Utc::now(), SESSION));

    assert_eq!(with_ledger(PaymentLedger::reserved_atomic).unwrap(), 0);
    assert_eq!(
        with_ledger(|l| l.summary().daily_total_atomic).unwrap(),
        500_000
    );
    reset_global();
}

#[test]
fn a_global_reservation_is_refused_over_budget_and_needs_a_ledger() {
    let _guard = TEST_LOCK.blocking_lock();
    reset_global();
    assert_eq!(
        reserve(1).unwrap_err(),
        "x402 payment ledger not initialized"
    );

    let dir = LedgerStorage::memory();
    init_global(dir.clone(), SESSION, budget()).unwrap();
    assert_eq!(
        reserve(600_000).unwrap().unwrap_err(),
        BudgetRefusal::PerRequest {
            requested: 600_000,
            cap: 500_000
        }
    );
    reset_global();
}

#[test]
fn dropping_a_reservation_after_the_ledger_is_gone_is_harmless() {
    let _guard = TEST_LOCK.blocking_lock();
    let dir = LedgerStorage::memory();
    init_global(dir.clone(), SESSION, budget()).unwrap();
    let held = reserve(500_000).unwrap().unwrap();
    reset_global();
    drop(held);

    // A new ledger never inherits an old hold, even if ids were reused.
    init_global(dir.clone(), SESSION, budget()).unwrap();
    assert_eq!(with_ledger(PaymentLedger::reserved_atomic).unwrap(), 0);
    reset_global();
}

#[test]
fn a_verdict_converts_to_its_refusal() {
    assert_eq!(BudgetCheck::Allowed.refusal(), None);
    assert_eq!(
        BudgetCheck::ExceedsPerRequest {
            requested: 2,
            cap: 1
        }
        .refusal(),
        Some(BudgetRefusal::PerRequest {
            requested: 2,
            cap: 1
        })
    );
}

#[test]
fn a_thread_is_written_only_when_there_is_one() {
    let mut with = record(1, PaymentStatus::Settled, now(), SESSION);
    assert!(!serde_json::to_string(&with).unwrap().contains("threadId"));
    with.thread_id = Some("thread-1".into());
    let json = serde_json::to_value(&with).unwrap();
    assert_eq!(json["threadId"], "thread-1");
    let back: PaymentRecord = serde_json::from_value(json).unwrap();
    assert_eq!(back.thread_id.as_deref(), Some("thread-1"));
}

#[test]
fn unparseable_and_foreign_stream_entries_are_skipped_on_load() {
    let dir = LedgerStorage::memory();
    let good = serde_json::to_value(record(1, PaymentStatus::Settled, now(), SESSION)).unwrap();
    append_raw(
        &dir,
        vec![
            good.clone(),
            serde_json::json!("not a record"),
            serde_json::json!({"id": 7}),
            good,
        ],
    );
    assert_eq!(ledger_in(&dir).recent_payments(10).len(), 2);
}

#[test]
fn a_record_written_before_threads_existed_still_loads() {
    let dir = LedgerStorage::memory();
    let old = serde_json::json!({"id":"old","url":"https://x","asset":"USDC","amountAtomic":5,"amountDisplay":"5","recipient":"r","network":"n","txSignature":null,"status":"settled","timestamp":"2026-03-15T12:00:00Z","sessionId":"session-a"});
    append_raw(&dir, vec![old]);

    let records = ledger_in(&dir).recent_payments(5);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].thread_id, None);
    assert_eq!(records[0].session_id, "session-a");
}

#[test]
fn a_payment_is_kept_in_memory_when_the_append_fails() {
    let (storage, faults) = faulty_storage();
    let mut ledger = ledger_in(&storage);
    faults.append.store(true, Ordering::SeqCst);
    ledger.record_payment(record(5, PaymentStatus::Settled, now(), SESSION));
    assert_eq!(ledger.recent_payments(10).len(), 1);
    faults.append.store(false, Ordering::SeqCst);
    assert!(
        ledger_in(&storage).recent_payments(10).is_empty(),
        "nothing reached the stream"
    );
}

#[test]
fn a_fallible_record_is_not_visible_until_it_is_saved() {
    let (storage, faults) = faulty_storage();
    let mut ledger = ledger_in(&storage);
    faults.append.store(true, Ordering::SeqCst);

    assert!(
        ledger
            .try_record_payment(record(5, PaymentStatus::Pending, now(), SESSION))
            .is_err()
    );
    assert_eq!(ledger.recent_payments(10), Vec::<PaymentRecord>::new());
}

#[test]
fn a_failed_durable_commit_keeps_the_reservation_held() {
    let (storage, faults) = faulty_storage();
    let mut ledger = ledger_in(&storage);
    let reservation = ledger.reserve(5).unwrap().unwrap();
    faults.append.store(true, Ordering::SeqCst);

    assert!(
        ledger
            .try_commit_reservation(
                reservation,
                record(5, PaymentStatus::Pending, now(), SESSION)
            )
            .is_err()
    );
    assert_eq!(ledger.reserved_atomic(), 5);
    assert_eq!(ledger.recent_payments(10), Vec::<PaymentRecord>::new());
}

#[test]
fn a_ledger_that_cannot_read_its_history_refuses_to_open() {
    let (storage, faults) = faulty_storage();
    faults.read.store(true, Ordering::SeqCst);
    let error = PaymentLedger::open(storage, SESSION, budget()).unwrap_err();
    assert!(error.contains("read payments stream"), "{error}");
}

#[test]
fn a_reservation_fails_closed_when_the_stream_cannot_be_read() {
    let (storage, faults) = faulty_storage();
    let mut ledger = ledger_in(&storage);
    faults.read.store(true, Ordering::SeqCst);
    assert!(ledger.reserve_at(now(), 1_000).is_err());
    assert_eq!(ledger.reserved_atomic(), 0, "nothing was held");
}

// ---------------------------------------------------------------------------
// Holds are a compare-and-swap on shared storage
// ---------------------------------------------------------------------------

#[test]
fn two_ledgers_on_one_storage_cannot_overspend_the_daily_budget() {
    let dir = LedgerStorage::memory();
    let mut a = ledger_in(&dir);
    let mut b = ledger_in(&dir);
    // The day allows 2_000_000. A takes three quarters, so B's two must not both fit.
    a.reserve_at(now(), 500_000).unwrap().unwrap();
    a.reserve_at(now(), 500_000).unwrap().unwrap();
    a.reserve_at(now(), 500_000).unwrap().unwrap();
    b.reserve_at(now(), 500_000).unwrap().unwrap();
    assert_eq!(
        b.reserve_at(now(), 500_000).unwrap().unwrap_err(),
        BudgetRefusal::Daily {
            current: 2_000_000,
            cap: 2_000_000
        },
        "B sees A's holds"
    );
}

#[test]
fn a_release_on_one_ledger_frees_budget_for_another() {
    let dir = LedgerStorage::memory();
    let mut a = ledger_in(&dir);
    let mut b = ledger_in(&dir);
    let held: Vec<_> = (0..4)
        .map(|_| a.reserve_at(now(), 500_000).unwrap().unwrap())
        .collect();
    assert!(b.reserve_at(now(), 500_000).unwrap().is_err());
    a.release(held[0]);
    assert!(b.reserve_at(now(), 500_000).unwrap().is_ok());
}

#[test]
fn another_writers_records_count_against_the_next_reservation() {
    let dir = LedgerStorage::memory();
    let mut a = ledger_in(&dir);
    let mut b = ledger_in(&dir);
    for _ in 0..4 {
        b.record_payment(record(500_000, PaymentStatus::Settled, now(), SESSION));
    }
    assert!(
        a.reserve_at(now(), 500_000).unwrap().is_err(),
        "A re-reads the stream before checking"
    );
    assert_eq!(a.recent_payments(10).len(), 4);
}

#[test]
fn an_expired_hold_from_a_dead_process_stops_counting() {
    let dir = LedgerStorage::memory();
    let mut dead = ledger_in(&dir);
    for _ in 0..4 {
        dead.reserve_at(now(), 500_000).unwrap().unwrap();
    }
    let mut live = ledger_in(&dir);
    assert!(live.reserve_at(now(), 500_000).unwrap().is_err());
    let later = now() + Duration::milliseconds(super::persist::HOLD_TTL_MS + 1);
    // The holds are day-old news only by the clock; the daily window is the
    // same day, so only the hold expiry can be what frees the budget.
    assert!(live.reserve_at(later, 500_000).unwrap().is_ok());
}

#[test]
fn concurrent_reservations_admit_exactly_what_fits() {
    let dir = LedgerStorage::memory();
    let admitted = std::sync::Arc::new(AtomicU64::new(0));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let dir = dir.clone();
            let admitted = std::sync::Arc::clone(&admitted);
            std::thread::spawn(move || {
                let mut ledger = ledger_in(&dir);
                for _ in 0..2 {
                    if ledger.reserve_at(now(), 500_000).unwrap().is_ok() {
                        admitted.fetch_add(1, Ordering::SeqCst);
                    }
                }
                // Keep the holds: dropping the ledger leaves them in storage.
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(
        admitted.load(Ordering::SeqCst),
        4,
        "2_000_000 / 500_000, no more, no fewer"
    );
}

#[test]
fn a_ledger_picks_up_a_hold_document_it_did_not_write() {
    let dir = LedgerStorage::memory();
    let mut a = ledger_in(&dir);
    a.reserve_at(now(), 500_000).unwrap().unwrap();
    let mut b = ledger_in(&dir);
    b.reserve_at(now(), 500_000).unwrap().unwrap();
    assert_eq!(
        b.reserved_atomic(),
        1_000_000,
        "B's mirror includes A's hold"
    );
}

fn corrupt_the_hold_document(storage: &LedgerStorage) {
    let documents = std::sync::Arc::clone(&storage.documents);
    Blocking::new()
        .unwrap()
        .run(async move {
            documents
                .put(
                    BUDGET_COLLECTION,
                    HOLDS_ID,
                    serde_json::json!({"holds": 5}),
                    tinystoragedrivers_core::Precondition::None,
                )
                .await
                .unwrap();
        })
        .unwrap();
}

#[test]
fn an_unreadable_hold_document_refuses_reservations() {
    let dir = LedgerStorage::memory();
    let mut ledger = ledger_in(&dir);
    corrupt_the_hold_document(&dir);
    let error = ledger.reserve_at(now(), 1_000).unwrap_err();
    assert!(error.contains("unreadable"), "{error}");
    assert_eq!(ledger.reserved_atomic(), 0);
}

#[test]
fn a_release_that_cannot_reach_storage_leaves_the_hold_to_expire() {
    let dir = LedgerStorage::memory();
    let mut ledger = ledger_in(&dir);
    let id = ledger.reserve_at(now(), 1_000).unwrap().unwrap();
    corrupt_the_hold_document(&dir);
    ledger.release(id);
    assert_eq!(
        ledger.reserved_atomic(),
        0,
        "this ledger forgets it at once"
    );
}

#[test]
fn the_hold_document_is_a_compare_and_swap() {
    let persist = Persist::open(LedgerStorage::memory()).unwrap();
    let doc = HoldsDoc::default();
    assert!(
        persist.store_holds(None, &doc).unwrap(),
        "first write inserts"
    );
    assert!(
        !persist.store_holds(None, &doc).unwrap(),
        "inserting over an existing document loses"
    );
    let (version, _) = persist.load_holds().unwrap();
    assert!(version.is_some());
    assert!(
        !persist.store_holds(Some(Version(99)), &doc).unwrap(),
        "a stale version loses"
    );
    assert!(persist.store_holds(version, &doc).unwrap());
}

#[test]
fn the_fault_injecting_streams_pass_everything_else_through() {
    let (storage, _faults) = faulty_storage();
    let streams = std::sync::Arc::clone(&storage.streams);
    Blocking::new()
        .unwrap()
        .run(async move {
            streams
                .append_batch("s", vec![serde_json::json!(1), serde_json::json!(2)])
                .await
                .unwrap();
            assert_eq!(streams.len("s").await.unwrap(), 2);
            assert_eq!(streams.streams("").await.unwrap(), vec!["s".to_string()]);
            assert_eq!(streams.truncate_before("s", 1).await.unwrap(), 1);
            assert!(streams.delete_stream("s").await.unwrap());
        })
        .unwrap();
}
