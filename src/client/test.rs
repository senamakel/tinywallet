//! Unit tests for chain queries.
//!
//! Driven through a scripted [`Transport`], so every chain's request shape and
//! response parsing is exercised with no network. The four chains each read a
//! different field out of a differently-shaped answer, which is exactly the
//! kind of translation that rots silently.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{Error, balance};
use crate::asset::{EvmNetwork, Network, SolanaCluster};
use crate::rpc::{NetworkId, Transport, TransportError, TransportResult};

/// Records every request and replays one canned answer.
struct Scripted {
    answer: TransportResult<String>,
    calls: std::sync::Mutex<Vec<String>>,
}

impl Scripted {
    fn json(value: &Value) -> Self {
        Self {
            answer: Ok(value.to_string()),
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn raw(body: &str) -> Self {
        Self {
            answer: Ok(body.to_string()),
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn failing(error: TransportError) -> Self {
        Self {
            answer: Err(error),
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn value(&self) -> TransportResult<Value> {
        match &self.answer {
            Ok(body) => Ok(serde_json::from_str(body).unwrap_or(Value::Null)),
            Err(e) => Err(e.clone()),
        }
    }
}

#[async_trait]
impl Transport for Scripted {
    async fn json_rpc(
        &self,
        network: NetworkId,
        method: &str,
        params: Value,
    ) -> TransportResult<Value> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{network} json_rpc {method} {params}"));
        self.value()
    }

    async fn rest_get(&self, network: NetworkId, path: &str) -> TransportResult<String> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{network} GET {path}"));
        self.answer.clone()
    }

    async fn rest_post(
        &self,
        network: NetworkId,
        path: &str,
        body: String,
        content_type: &str,
    ) -> TransportResult<String> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{network} POST {path} [{content_type}] {body}"));
        self.answer.clone()
    }
}

const EVM_ADDR: &str = "0x52908400098527886E0F7030069857D2E4169EE7";
const BTC_ADDR: &str = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";
const SOL_ADDR: &str = "11111111111111111111111111111111";
const TRON_ADDR: &str = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t";

#[tokio::test]
async fn evm_reads_a_hex_wei_balance_at_the_latest_block() {
    let transport = Scripted::json(&json!("0xde0b6b3a7640000")); // 1e18
    let wei = balance(&transport, Network::Evm(EvmNetwork::Base), EVM_ADDR)
        .await
        .unwrap();

    assert_eq!(wei, 1_000_000_000_000_000_000);
    let call = &transport.calls()[0];
    assert!(call.contains("eth_getBalance"), "{call}");
    // `latest`, not `pending`: a pending balance can go backwards without
    // anything having failed.
    assert!(call.contains("latest"), "{call}");
    // Routed by chain id, so the host can tell Base from Ethereum.
    assert!(call.starts_with("evm:8453"), "{call}");
}

#[tokio::test]
async fn evm_accepts_a_zero_balance() {
    let transport = Scripted::json(&json!("0x0"));
    assert_eq!(
        balance(&transport, Network::Evm(EvmNetwork::Ethereum), EVM_ADDR)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn evm_rejects_a_non_hex_balance_as_malformed() {
    let transport = Scripted::json(&json!("not-hex"));
    match balance(&transport, Network::Evm(EvmNetwork::Base), EVM_ADDR)
        .await
        .unwrap_err()
    {
        Error::MalformedResponse { operation, .. } => assert_eq!(operation, "eth_getBalance"),
        other => panic!("expected MalformedResponse, got {other:?}"),
    }
}

#[tokio::test]
async fn solana_unwraps_the_context_envelope() {
    // Solana wraps results in {context, value}; reading the envelope instead
    // of `value` would yield nonsense.
    let transport = Scripted::json(&json!({ "context": { "slot": 1 }, "value": 2_500_000_000u64 }));
    let lamports = balance(
        &transport,
        Network::Solana(SolanaCluster::Mainnet),
        SOL_ADDR,
    )
    .await
    .unwrap();
    assert_eq!(lamports, 2_500_000_000);
    assert!(transport.calls()[0].contains("getBalance"));
}

#[tokio::test]
async fn btc_sums_confirmed_and_mempool_activity() {
    // Esplora reports funded/spent totals separately for chain and mempool.
    // The balance is (funded - spent) across both.
    let transport = Scripted::json(&json!({
        "chain_stats":   { "funded_txo_sum": 100_000u64, "spent_txo_sum": 40_000u64 },
        "mempool_stats": { "funded_txo_sum":  10_000u64, "spent_txo_sum":  1_000u64 },
    }));
    let sats = balance(&transport, Network::Btc, BTC_ADDR).await.unwrap();

    assert_eq!(sats, 60_000 + 9_000);
    assert_eq!(
        transport.calls(),
        vec![format!("btc GET address/{BTC_ADDR}")]
    );
}

#[tokio::test]
async fn btc_clamps_rather_than_underflowing_on_a_nonsensical_response() {
    // spent > funded should be impossible. If a broken or hostile endpoint
    // says otherwise, clamping to zero is far better than underflowing into a
    // balance near u64::MAX and showing the user a fortune.
    let transport = Scripted::json(&json!({
        "chain_stats":   { "funded_txo_sum": 1u64, "spent_txo_sum": 999u64 },
        "mempool_stats": { "funded_txo_sum": 0u64, "spent_txo_sum": 0u64 },
    }));
    assert_eq!(
        balance(&transport, Network::Btc, BTC_ADDR).await.unwrap(),
        0
    );
}

#[tokio::test]
async fn btc_rejects_a_response_that_is_not_esplora() {
    let transport = Scripted::raw("<html>captive portal</html>");
    assert!(matches!(
        balance(&transport, Network::Btc, BTC_ADDR)
            .await
            .unwrap_err(),
        Error::MalformedResponse { .. }
    ));
}

#[tokio::test]
async fn tron_posts_the_hex_address_not_the_base58_one() {
    // TronGrid wants the 41-prefixed hex form; sending base58check silently
    // returns an empty account, which reads as a zero balance.
    let transport = Scripted::json(&json!({ "balance": 12_345_678u64 }));
    let sun = balance(&transport, Network::Tron, TRON_ADDR).await.unwrap();

    assert_eq!(sun, 12_345_678);
    let call = &transport.calls()[0];
    assert!(call.contains("wallet/getaccount"), "{call}");
    assert!(call.contains("application/json"), "{call}");
    assert!(
        call.contains(&crate::address::tron::to_hex(TRON_ADDR).unwrap()),
        "must send the hex form: {call}"
    );
    assert!(!call.contains(TRON_ADDR), "must not send base58: {call}");
}

#[tokio::test]
async fn tron_treats_an_empty_account_as_zero() {
    // Unlike the other chains, TronGrid returns `{}` for an account that has
    // never been funded rather than a balance of 0. That is not malformed.
    let transport = Scripted::json(&json!({}));
    assert_eq!(
        balance(&transport, Network::Tron, TRON_ADDR).await.unwrap(),
        0
    );
}

#[tokio::test]
async fn a_bad_address_is_rejected_before_any_request_is_made() {
    // "You typed a bad address" beats a node's inconsistent answer, and beats
    // spending a round trip to find out.
    let transport = Scripted::json(&json!("0x0"));
    let err = balance(&transport, Network::Evm(EvmNetwork::Base), "nope")
        .await
        .unwrap_err();

    assert!(matches!(err, Error::Address(_)), "got {err:?}");
    assert!(
        transport.calls().is_empty(),
        "no request should have been sent: {:?}",
        transport.calls()
    );
}

#[tokio::test]
async fn an_address_from_the_wrong_chain_is_rejected() {
    let transport = Scripted::json(&json!("0x0"));
    for (network, wrong) in [
        (Network::Evm(EvmNetwork::Base), SOL_ADDR),
        (Network::Btc, EVM_ADDR),
        (Network::Tron, BTC_ADDR),
    ] {
        let err = balance(&transport, network, wrong).await.unwrap_err();
        assert!(matches!(err, Error::Address(_)), "{network}: {err:?}");
    }
    assert!(transport.calls().is_empty());
}

#[tokio::test]
async fn a_transport_failure_surfaces_with_its_retryability_intact() {
    // The client must not flatten the distinction the seam draws — a host's
    // failover logic depends on it.
    let network = NetworkId::evm(8453);
    let transport = Scripted::failing(TransportError::Unreachable {
        network,
        message: "connection refused".to_string(),
    });
    match balance(&transport, Network::Evm(EvmNetwork::Base), EVM_ADDR)
        .await
        .unwrap_err()
    {
        Error::Transport(e) => assert!(e.is_retryable()),
        other => panic!("expected Transport, got {other:?}"),
    }

    let transport = Scripted::failing(TransportError::Rpc {
        network,
        message: "method not found".to_string(),
    });
    match balance(&transport, Network::Evm(EvmNetwork::Base), EVM_ADDR)
        .await
        .unwrap_err()
    {
        Error::Transport(e) => assert!(!e.is_retryable()),
        other => panic!("expected Transport, got {other:?}"),
    }
}

#[tokio::test]
async fn every_chain_issues_exactly_one_request_for_a_balance() {
    for (network, address, body) in [
        (Network::Evm(EvmNetwork::Ethereum), EVM_ADDR, json!("0x1")),
        (
            Network::Solana(SolanaCluster::Devnet),
            SOL_ADDR,
            json!({"value": 1u64}),
        ),
        (Network::Tron, TRON_ADDR, json!({"balance": 1u64})),
        (
            Network::Btc,
            BTC_ADDR,
            json!({
                "chain_stats":   {"funded_txo_sum": 1u64, "spent_txo_sum": 0u64},
                "mempool_stats": {"funded_txo_sum": 0u64, "spent_txo_sum": 0u64},
            }),
        ),
    ] {
        let transport = Scripted::json(body);
        let out = balance(&transport, network, address).await.unwrap();
        assert_eq!(out, 1, "{network} balance");
        assert_eq!(transport.calls().len(), 1, "{network} request count");
    }
}
