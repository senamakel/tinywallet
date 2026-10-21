#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::legacy_payments::{LegacyPayment, StoredPaymentStatus, convert_legacy_payment};

fn convert(
    value: serde_json::Value,
) -> Result<super::legacy_payments::StoredPaymentRecord, serde_json::Error> {
    serde_json::from_value::<LegacyPayment>(value).map(convert_legacy_payment)
}

#[test]
fn legacy_payment_conversion_preserves_current_fields_and_normalizes_instants() {
    let legacy = serde_json::json!({
        "id": "payment-1",
        "url": "https://merchant.example/item",
        "asset": "USDC",
        "amountAtomic": 12345,
        "amountDisplay": "0.012345",
        "recipient": "recipient",
        "network": "eip155:1",
        "txSignature": "0xabc",
        "status": "settled",
        "timestamp": "2026-03-15T15:00:00.250000+03:00",
        "authorizationValidUntil": "2026-03-15T12:00:01.9999Z",
        "solanaLastValidBlockHeight": 42,
        "authorizationExpiredAt": "2026-03-15T12:00:02Z",
        "authorizationExpiryObservedHeight": 43,
        "sessionId": "session-1",
        "threadId": "thread-1",
        "committed_hold": "hold-1"
    });

    let payment = convert(legacy).unwrap();
    assert_eq!(payment.id, "payment-1");
    assert_eq!(payment.status, StoredPaymentStatus::Settled);
    assert_eq!(payment.timestamp_ms, 1_773_576_000_250);
    assert_eq!(
        payment.authorization_valid_until_ms,
        Some(1_773_576_001_999)
    );
    assert_eq!(payment.authorization_expired_at_ms, Some(1_773_576_002_000));
    assert_eq!(payment.solana_last_valid_block_height, Some(42));
    assert_eq!(payment.authorization_expiry_observed_height, Some(43));
    assert_eq!(payment.session_id, "session-1");
    assert_eq!(payment.thread_id.as_deref(), Some("thread-1"));
    assert_eq!(payment.committed_hold.as_deref(), Some("hold-1"));

    let current = serde_json::to_value(payment).unwrap();
    assert_eq!(current["amount_atomic"], 12345);
    assert_eq!(current["timestamp_ms"], 1_773_576_000_250_i64);
    assert_eq!(
        current["authorization_valid_until_ms"],
        1_773_576_001_999_i64
    );
    assert_eq!(current["tx_signature"], "0xabc");
    assert!(current.get("timestamp").is_none());
}

#[test]
fn legacy_payment_conversion_preserves_rfc3339_timestamp_edge_cases() {
    let row = |timestamp: &str| {
        serde_json::json!({
            "id": "payment-time",
            "url": "https://merchant.example/item",
            "asset": "USD",
            "amountAtomic": 1,
            "amountDisplay": "1",
            "recipient": "recipient",
            "network": "eip155:1",
            "status": "pending",
            "timestamp": timestamp,
            "sessionId": "session"
        })
    };

    for (timestamp, expected_ms) in [
        ("2026-03-15t15:00:00.250000+03:00", 1_773_576_000_250),
        ("2026-03-15 12:00:00.9z", 1_773_576_000_900),
        ("2026-03-15T12:00:60Z", 1_773_576_060_000),
    ] {
        assert_eq!(convert(row(timestamp)).unwrap().timestamp_ms, expected_ms);
    }
    assert!(convert(row("2024-02-29T00:00:00Z")).is_ok());
}

#[test]
fn legacy_payment_conversion_defaults_optional_fields_and_preserves_repeated_ids() {
    let row = |status: &str, timestamp: &str| {
        serde_json::json!({
            "id": "same-attempt",
            "url": "https://merchant.example/item",
            "asset": "USD",
            "amountAtomic": 1,
            "amountDisplay": "1",
            "recipient": "recipient",
            "network": "eip155:1",
            "status": status,
            "timestamp": timestamp,
            "sessionId": "session"
        })
    };
    let pending = convert(row("pending", "2026-03-15T12:00:00Z")).unwrap();
    let settled = convert(row("settled", "2026-03-15T12:00:01Z")).unwrap();
    assert_eq!(pending.id, settled.id);
    assert_eq!(pending.status, StoredPaymentStatus::Pending);
    assert_eq!(settled.status, StoredPaymentStatus::Settled);
    assert_eq!(pending.tx_signature, None);
    assert_eq!(pending.authorization_valid_until_ms, None);
    assert_eq!(pending.thread_id, None);
    assert_eq!(pending.committed_hold, None);
}

#[test]
fn legacy_payment_conversion_rejects_malformed_required_and_optional_data() {
    let base = serde_json::json!({
        "id": "payment-1",
        "url": "https://merchant.example/item",
        "asset": "USD",
        "amountAtomic": 1,
        "amountDisplay": "1",
        "recipient": "recipient",
        "network": "eip155:1",
        "status": "pending",
        "timestamp": "2026-03-15T12:00:00Z",
        "sessionId": "session"
    });

    for (field, value) in [
        ("timestamp", serde_json::json!("yesterday")),
        ("timestamp", serde_json::json!("2026-13-15T12:00:00Z")),
        ("timestamp", serde_json::json!("2026-03-00T12:00:00Z")),
        ("timestamp", serde_json::json!("2026-02-29T12:00:00Z")),
        ("timestamp", serde_json::json!("2024-02-31T12:00:00Z")),
        ("timestamp", serde_json::json!("2026-03-15T24:00:00Z")),
        ("timestamp", serde_json::json!("2026-03-15T12:60:00Z")),
        ("timestamp", serde_json::json!("2026-03-15T12:00:61Z")),
        ("timestamp", serde_json::json!("2026-03-15T12:00:00.")),
        ("timestamp", serde_json::json!("2026-03-15T12:00:00+01:0x")),
        ("timestamp", serde_json::json!("2026-03-15T12:00:00+24:00")),
        ("timestamp", serde_json::json!("2026-03-15T12:00:00+01:60")),
        (
            "timestamp",
            serde_json::json!("2026-03-15T12:00:00+01:00tail"),
        ),
        ("timestamp", serde_json::json!("2026/03/15T12:00:00Z")),
        (
            "authorizationValidUntil",
            serde_json::json!("not-a-timestamp"),
        ),
        ("status", serde_json::json!("unknown")),
        ("amountAtomic", serde_json::json!(-1)),
    ] {
        let mut malformed = base.clone();
        malformed[field] = value;
        assert!(convert(malformed).is_err(), "field {field}");
    }
    assert!(convert(serde_json::json!([])).is_err());
}
