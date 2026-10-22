//! Run against tinywallet v0.24.1 with TSD v1.0.0 to generate the checked-in
//! previous-release payment stream row through the public ledger API.

use chrono::{TimeZone, Utc};
use tinystoragedrivers_core::{MemoryStorage, Scope, StorageBackend};
use tinywallet_x402::ledger::{
    LedgerStorage, PaymentLedger, PaymentRecord, PaymentStatus, SpendingBudget,
};

fn main() {
    let output = std::env::args().nth(1).expect("fixture output path");
    let backend = MemoryStorage::new();
    let scope = Scope::local().module("wallet").expect("valid wallet scope");
    let scoped = backend.for_scope(&scope).expect("open wallet scope");
    let mut ledger = PaymentLedger::open(
        LedgerStorage::from_scoped(&scoped),
        "release-fixture-session",
        SpendingBudget::default(),
    )
    .expect("open previous-release public ledger");
    ledger.record_payment(PaymentRecord {
        id: "v0241-fixture-payment".into(),
        url: "https://merchant.example.test/resource".into(),
        asset: "USDC".into(),
        amount_atomic: 250_000,
        amount_display: "0.250000 USDC".into(),
        recipient: "WalletRecipient".into(),
        network: "solana:devnet".into(),
        tx_signature: Some("release-fixture-signature".into()),
        status: PaymentStatus::Settled,
        timestamp: Utc.with_ymd_and_hms(2025, 7, 14, 12, 30, 0).unwrap(),
        authorization_valid_until: None,
        solana_last_valid_block_height: None,
        authorization_expired_at: None,
        authorization_expiry_observed_height: None,
        session_id: "release-fixture-session".into(),
        thread_id: Some("release-fixture-thread".into()),
    });

    let streams = scoped.streams().clone();
    let payments = tinystoragedrivers_core::Blocking::new()
        .expect("blocking bridge")
        .run(async move {
            streams
                .read_window("wallet.payments", 0, 10)
                .await
                .expect("read previous-release output")
                .into_iter()
                .map(|entry| entry.value)
                .collect::<Vec<_>>()
        })
        .expect("read previous-release stream");
    std::fs::write(output, serde_json::to_vec_pretty(&payments).expect("encode fixture"))
        .expect("write fixture");
}
