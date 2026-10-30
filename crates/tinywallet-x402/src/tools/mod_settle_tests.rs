//! Tests for matching the settlement receipt to the payment it settles.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::json;

use super::test::{init_ledger, paid_config, records, run, text, tool};
use crate::ledger::{self, PaymentStatus};
use crate::test_support::{ServerConfig, TestServer, evm_requirement};
use crate::wire::SettlementResponse;

#[tokio::test]
async fn a_receipt_for_another_network_or_amount_does_not_settle_the_payment() {
    let _guard = ledger::TEST_LOCK.lock().await;
    for (network, amount) in [("eip155:84532", None), ("eip155:8453", Some("1"))] {
        let _dir = init_ledger();
        let receipt = B64.encode(
            serde_json::to_vec(&SettlementResponse {
                success: true,
                transaction: "0xother".into(),
                network: network.into(),
                payer: None,
                error_reason: None,
                amount: amount.map(Into::into),
                extensions: serde_json::Map::new(),
            })
            .unwrap(),
        );
        let server = TestServer::start(ServerConfig {
            receipt: Some(receipt),
            ..paid_config(evm_requirement())
        })
        .await;
        let result = run(&tool(), json!({"url": server.url})).await;
        assert!(!result.is_error, "{}", text(&result));
        let records = records();
        assert_eq!(
            records.last().unwrap().status,
            PaymentStatus::Pending,
            "a {network} receipt for {amount:?} settled a 2500 payment on eip155:8453"
        );
        assert_eq!(records.last().unwrap().tx_signature, None);
        ledger::reset_global();
    }
}
