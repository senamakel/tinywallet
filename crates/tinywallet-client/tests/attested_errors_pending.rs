//! Pending the separately owned TinyBus detailed-attested-call API.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

mod support;

use tinywallet_client::ClientError;

#[tokio::test]
async fn invalid_seed_preserves_remote_error_classification_on_strict_delivery() {
    let fixture = support::fixture().await;
    let result = fixture
        .wallet
        .derive_mnemonic_seed("not a valid mnemonic".to_string())
        .await;
    let failure = match result {
        Err(ClientError::Remote(failure)) => failure,
        _ => panic!("strict remote InvalidInput must retain its detailed failure type"),
    };

    assert!(
        failure.wire_name == "ai.tinyhumans.tinywallet.Error.InvalidInput",
        "strict remote failure must preserve the wire classification"
    );
    assert!(!format!("{failure:?}").contains("not a valid mnemonic"));
    assert!(!failure.to_string().contains("not a valid mnemonic"));
}
