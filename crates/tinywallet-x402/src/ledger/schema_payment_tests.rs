#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

#[test]
fn owner_migration_converter_emits_a_current_stored_payment_body() {
    let legacy: LegacyPayment = serde_json::from_value(serde_json::json!({
        "id": "legacy-attempt",
        "url": "https://merchant.example/item",
        "asset": "USD",
        "amountAtomic": 100,
        "amountDisplay": "1.00",
        "recipient": "recipient",
        "network": "eip155:1",
        "status": "settled",
        "timestamp": "2026-03-15T15:00:00.250+03:00",
        "sessionId": "session"
    }))
    .unwrap();
    let payment = convert_legacy_payment(legacy);

    let encoded = serde_json::to_value(&payment).unwrap();
    let decoded: StoredPayment = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(decoded, payment);
    assert_eq!(encoded["timestamp_ms"], 1_773_576_000_250_i64);
    assert_eq!(encoded["status"], "settled");
    assert!(encoded.get("timestamp").is_none());
}
