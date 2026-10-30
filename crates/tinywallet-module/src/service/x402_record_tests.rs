#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::service::x402::tests::pending_record;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

fn receipt(network: &str, amount: Option<&str>) -> String {
    let mut receipt = serde_json::json!({
        "success": true,
        "transaction": "0xsettled",
        "network": network
    });
    if let Some(amount) = amount {
        receipt["amount"] = amount.into();
    }
    BASE64.encode(serde_json::to_vec(&receipt).unwrap())
}

#[tokio::test]
async fn record_settles_only_on_a_receipt_for_the_payments_network_and_amount() {
    let _ledger_guard = LEDGER_TEST_LOCK.lock().await;
    let bus = tinybus::transport::memory::MemoryBus::new();
    let broker_task = tinybus::broker::Broker::new().spawn(bus.clone());
    let connection = tinybus::Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let mut runtime = X402Runtime::new(
        HostCallbacks::new(&connection).unwrap(),
        X402ConfigResponse {
            workspace_dir: workspace.path().display().to_string(),
            session_id: "coverage-test".to_string(),
            per_request_max_atomic: 100,
            daily_max_atomic: 100,
            monthly_max_atomic: 100,
        },
        memory_ledger_storage(),
    )
    .unwrap();
    let handle = runtime.handle();
    // The pending payment is 1 atomic unit on eip155:1.
    ledger::with_ledger_mut(|ledger| ledger.try_record_payment(pending_record("paid", None)))
        .unwrap()
        .unwrap();
    let status = || ledger::with_ledger(|ledger| ledger.payment("paid").unwrap().status).unwrap();
    let record = |runtime: &X402Runtime, settlement: String| {
        runtime
            .record(X402RecordPaymentRequest {
                service: handle.clone(),
                payment_id: "paid".to_string(),
                response_status: 200,
                settlement_response: Some(settlement),
            })
            .unwrap();
    };

    record(&runtime, receipt("eip155:8453", None));
    assert_eq!(
        status(),
        PaymentStatus::Pending,
        "another network settled it"
    );
    record(&runtime, receipt("eip155:1", Some("2")));
    assert_eq!(
        status(),
        PaymentStatus::Pending,
        "another amount settled it"
    );
    assert_eq!(runtime.summary(&handle).unwrap().reserved_atomic, 1);

    record(&runtime, receipt("eip155:1", Some("1")));
    assert_eq!(status(), PaymentStatus::Settled);
    runtime.close(&handle).unwrap();
    broker_task.abort();
}
