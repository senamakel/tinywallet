//! Behavior tests for pure quote-to-intent comparisons.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::TronTransfer;
use crate::wire::{
    AuthorizeWeb3ExecutionRequest, EvmNetwork, NativeWalletExecutionIntent, PreparedKind,
    PreparedStatus, PreparedTransaction, TransactionSpec, WalletChain, Web3ExecutionIntent,
    Web3QuoteKind, Web3QuoteResponse,
};

fn prepared_quote(
    quote_id: &str,
    chain: WalletChain,
    kind: PreparedKind,
    amount_raw: &str,
    to_address: &str,
) -> PreparedTransaction {
    PreparedTransaction {
        quote_id: quote_id.into(),
        kind,
        chain,
        evm_network: Some(EvmNetwork::EthereumMainnet),
        from_address: "sender".into(),
        to_address: to_address.into(),
        asset_symbol: "ETH".into(),
        amount_raw: amount_raw.into(),
        amount_formatted: amount_raw.into(),
        receive_symbol: None,
        min_receive_raw: None,
        calldata: None,
        token_address: None,
        estimated_fee_raw: "1".into(),
        status: PreparedStatus::AwaitingConfirmation,
        created_at_ms: 1,
        expires_at_ms: u64::MAX,
        notes: vec![],
    }
}

fn evm_intent(to: &str, value: &str, data: &str, chain_id: u64) -> NativeWalletExecutionIntent {
    NativeWalletExecutionIntent::Transaction {
        transaction: TransactionSpec::Evm {
            to: to.into(),
            value_wei: value.into(),
            data_hex: data.into(),
            nonce: 0,
            gas_limit: 21_000,
            gas_price_wei: "1".into(),
            chain_id,
        },
    }
}

fn erc20_transfer_data(recipient: &str, amount: u64) -> String {
    let address = recipient.strip_prefix("0x").unwrap();
    format!("0xa9059cbb{address:0>64}{amount:064x}")
}

#[test]
fn native_bitcoin_transfer_requires_the_exact_chain_addresses_and_u64_amount() {
    let quote = prepared_quote(
        "btc-quote",
        WalletChain::Btc,
        PreparedKind::NativeTransfer,
        "42",
        "recipient",
    );
    let intent = NativeWalletExecutionIntent::Transaction {
        transaction: TransactionSpec::Btc {
            from: "sender".into(),
            to: "recipient".into(),
            amount_sat: 42,
            fee_sat: 1,
            utxos: vec![],
        },
    };
    assert!(super::native_intent_matches_quote(&quote, &intent));

    let wrong_sender = NativeWalletExecutionIntent::Transaction {
        transaction: TransactionSpec::Btc {
            from: "other-sender".into(),
            to: "recipient".into(),
            amount_sat: 42,
            fee_sat: 1,
            utxos: vec![],
        },
    };
    assert!(!super::native_intent_matches_quote(&quote, &wrong_sender));
    let wrong_amount_quote = prepared_quote(
        "btc-overflow-quote",
        WalletChain::Btc,
        PreparedKind::NativeTransfer,
        "18446744073709551616",
        "recipient",
    );
    assert!(!super::native_intent_matches_quote(
        &wrong_amount_quote,
        &intent
    ));
}

#[test]
fn native_evm_transfer_uses_the_quoted_network_and_empty_calldata() {
    let quote = prepared_quote(
        "evm-native",
        WalletChain::Evm,
        PreparedKind::NativeTransfer,
        "42",
        "recipient",
    );
    assert!(super::native_intent_matches_quote(
        &quote,
        &evm_intent("recipient", "42", "", 1)
    ));
    assert!(!super::native_intent_matches_quote(
        &quote,
        &evm_intent("recipient", "42", "0x", 1)
    ));
    assert!(!super::native_intent_matches_quote(
        &quote,
        &evm_intent("recipient", "42", "", 137)
    ));

    let mut default_network = quote;
    default_network.evm_network = None;
    assert!(super::native_intent_matches_quote(
        &default_network,
        &evm_intent(
            "recipient",
            "42",
            "",
            EvmNetwork::EthereumMainnet.chain_id()
        )
    ));
}

#[test]
fn native_evm_token_transfer_checks_contract_selector_recipient_and_uint256_amount() {
    let recipient = "0x1111111111111111111111111111111111111111";
    let contract = "0x2222222222222222222222222222222222222222";
    let mut quote = prepared_quote(
        "evm-token",
        WalletChain::Evm,
        PreparedKind::TokenTransfer,
        "42",
        recipient,
    );
    quote.token_address = Some(contract.into());
    let valid = evm_intent(
        &contract.to_ascii_uppercase().replacen("0X", "0x", 1),
        "0",
        &erc20_transfer_data(recipient, 42),
        1,
    );
    assert!(super::native_intent_matches_quote(&quote, &valid));

    assert!(!super::native_intent_matches_quote(
        &quote,
        &evm_intent(contract, "0", "0xdeadbeef", 1)
    ));
    assert!(!super::native_intent_matches_quote(
        &quote,
        &evm_intent(contract, "1", &erc20_transfer_data(recipient, 42), 1)
    ));
    assert!(!super::native_intent_matches_quote(
        &quote,
        &evm_intent(
            contract,
            "0",
            &erc20_transfer_data("0x3333333333333333333333333333333333333333", 42),
            1,
        )
    ));

    quote.amount_raw =
        "115792089237316195423570985008687907853269984665640564039457584007913129639936".into();
    assert!(!super::native_intent_matches_quote(
        &quote,
        &evm_intent(contract, "0", &erc20_transfer_data(recipient, 42), 1)
    ));
    quote.token_address = None;
    assert!(!super::native_intent_matches_quote(
        &quote,
        &evm_intent(contract, "0", &erc20_transfer_data(recipient, 42), 1)
    ));
}

#[test]
fn native_solana_and_tron_transfers_match_chain_specific_amounts() {
    let solana_quote = prepared_quote(
        "solana-native",
        WalletChain::Solana,
        PreparedKind::NativeTransfer,
        "1000",
        "recipient",
    );
    let solana_intent = NativeWalletExecutionIntent::Transaction {
        transaction: TransactionSpec::Solana {
            from: "sender".into(),
            to: "recipient".into(),
            lamports: 1000,
            recent_blockhash: "blockhash".into(),
        },
    };
    assert!(super::native_intent_matches_quote(
        &solana_quote,
        &solana_intent
    ));

    let tron_quote = prepared_quote(
        "tron-native",
        WalletChain::Tron,
        PreparedKind::NativeTransfer,
        "25",
        "recipient",
    );
    let tron_intent = NativeWalletExecutionIntent::Transaction {
        transaction: TransactionSpec::Tron {
            raw_data_hex: "00".into(),
            expected_to: "recipient".into(),
            expected_txid: "txid".into(),
            transfer: TronTransfer::Native { amount_sun: 25 },
        },
    };
    assert!(super::native_intent_matches_quote(
        &tron_quote,
        &tron_intent
    ));
    let wrong_amount = NativeWalletExecutionIntent::Transaction {
        transaction: TransactionSpec::Tron {
            raw_data_hex: "00".into(),
            expected_to: "recipient".into(),
            expected_txid: "txid".into(),
            transfer: TronTransfer::Native { amount_sun: 26 },
        },
    };
    assert!(!super::native_intent_matches_quote(
        &tron_quote,
        &wrong_amount
    ));
}

#[test]
fn native_token_signing_intents_reject_malformed_hex_and_wrong_quote_kinds() {
    let tron_quote = prepared_quote(
        "tron-token",
        WalletChain::Tron,
        PreparedKind::TokenTransfer,
        "25",
        "recipient",
    );
    let valid_tron = NativeWalletExecutionIntent::Transaction {
        transaction: TransactionSpec::Tron {
            raw_data_hex: "00".into(),
            expected_to: "recipient".into(),
            expected_txid: "txid".into(),
            transfer: TronTransfer::Trc20 {
                parameter_hex: "deadbeef".into(),
            },
        },
    };
    assert!(super::native_intent_matches_quote(&tron_quote, &valid_tron));
    for parameter_hex in ["", "abc", "0xgg"] {
        let malformed = NativeWalletExecutionIntent::Transaction {
            transaction: TransactionSpec::Tron {
                raw_data_hex: "00".into(),
                expected_to: "recipient".into(),
                expected_txid: "txid".into(),
                transfer: TronTransfer::Trc20 {
                    parameter_hex: parameter_hex.into(),
                },
            },
        };
        assert!(!super::native_intent_matches_quote(&tron_quote, &malformed));
    }

    let solana_token_quote = prepared_quote(
        "solana-token",
        WalletChain::Solana,
        PreparedKind::TokenTransfer,
        "1",
        "recipient",
    );
    assert!(super::native_intent_matches_quote(
        &solana_token_quote,
        &NativeWalletExecutionIntent::SolanaMessage {
            message_hex: "010203".into(),
        }
    ));
    for message_hex in ["", "abc", "01xz"] {
        assert!(!super::native_intent_matches_quote(
            &solana_token_quote,
            &NativeWalletExecutionIntent::SolanaMessage {
                message_hex: message_hex.into(),
            }
        ));
    }
    let wrong_kind = prepared_quote(
        "solana-native-kind",
        WalletChain::Solana,
        PreparedKind::NativeTransfer,
        "1",
        "recipient",
    );
    assert!(!super::native_intent_matches_quote(
        &wrong_kind,
        &NativeWalletExecutionIntent::SolanaMessage {
            message_hex: "010203".into(),
        }
    ));
}

fn web3_quote(
    kind: Web3QuoteKind,
    signing_chain: WalletChain,
    quote: serde_json::Value,
) -> Web3QuoteResponse {
    Web3QuoteResponse {
        quote_id: "quote-1".into(),
        kind,
        expires_at_ms: u64::MAX,
        signing_chain,
        quote,
    }
}

fn web3_request(
    quote_id: &str,
    operation: &str,
    transaction: Web3ExecutionIntent,
) -> AuthorizeWeb3ExecutionRequest {
    AuthorizeWeb3ExecutionRequest {
        quote_id: quote_id.into(),
        operation: operation.into(),
        transaction,
    }
}

fn web3_evm(
    network: &str,
    to: &str,
    data: Option<&str>,
    value: &str,
    chain_id: u64,
) -> Web3ExecutionIntent {
    Web3ExecutionIntent::Evm {
        network: network.into(),
        to: to.into(),
        data: data.map(str::to_owned),
        value: value.into(),
        nonce: 0,
        gas_limit: 21_000,
        gas_price_wei: "1".into(),
        chain_id,
    }
}

fn dapp_quote(address: &str) -> Web3QuoteResponse {
    web3_quote(
        Web3QuoteKind::DappCall,
        WalletChain::Evm,
        serde_json::json!({
            "network": "polygon_mainnet",
            "contractAddress": address,
            "calldata": "0xabcd",
            "valueRaw": "5"
        }),
    )
}

fn dapp_request(
    quote_id: &str,
    operation: &str,
    network: &str,
    to: &str,
    data: Option<&str>,
    value: &str,
    chain_id: u64,
) -> AuthorizeWeb3ExecutionRequest {
    web3_request(
        quote_id,
        operation,
        web3_evm(network, to, data, value, chain_id),
    )
}

#[test]
fn web3_dapp_call_matches_case_insensitive_address_and_exact_call_facts() {
    let address = "0x1111111111111111111111111111111111111111";
    let quote = dapp_quote(address);
    assert!(super::web3_approval_matches_request(
        &quote,
        &dapp_request(
            "quote-1",
            "dapp_call",
            "polygon_mainnet",
            &address.to_ascii_uppercase().replacen("0X", "0x", 1),
            Some("0xabcd"),
            "5",
            137,
        )
    ));
}

#[test]
fn web3_dapp_call_rejects_a_different_quote_operation_or_network() {
    let address = "0x1111111111111111111111111111111111111111";
    let quote = dapp_quote(address);
    for (quote_id, operation, network, chain_id) in [
        ("other-quote", "dapp_call", "polygon_mainnet", 137),
        ("quote-1", "swap", "polygon_mainnet", 137),
        ("quote-1", "dapp_call", "ethereum_mainnet", 137),
        ("quote-1", "dapp_call", "polygon_mainnet", 1),
    ] {
        assert!(!super::web3_approval_matches_request(
            &quote,
            &dapp_request(
                quote_id,
                operation,
                network,
                address,
                Some("0xabcd"),
                "5",
                chain_id,
            )
        ));
    }
}

#[test]
fn web3_dapp_call_rejects_changed_call_facts_and_missing_quote_fields() {
    let address = "0x1111111111111111111111111111111111111111";
    let quote = dapp_quote(address);
    for (data, value) in [(Some("0xabce"), "5"), (Some("0xabcd"), "6"), (None, "5")] {
        assert!(!super::web3_approval_matches_request(
            &quote,
            &dapp_request(
                "quote-1",
                "dapp_call",
                "polygon_mainnet",
                address,
                data,
                value,
                137,
            )
        ));
    }

    let missing_calldata = web3_quote(
        Web3QuoteKind::DappCall,
        WalletChain::Evm,
        serde_json::json!({
            "network": "polygon_mainnet",
            "contractAddress": address,
            "calldata": null,
            "valueRaw": "5"
        }),
    );
    assert!(!super::web3_approval_matches_request(
        &missing_calldata,
        &dapp_request(
            "quote-1",
            "dapp_call",
            "polygon_mainnet",
            address,
            None,
            "5",
            137,
        )
    ));
}

#[test]
fn web3_swap_and_bridge_compare_transaction_data_and_string_or_number_value() {
    for kind in [Web3QuoteKind::Swap, Web3QuoteKind::Bridge] {
        let quote = web3_quote(
            kind,
            WalletChain::Evm,
            serde_json::json!({
                "network": "polygon_mainnet",
                "tx": {
                    "to": "0x1111111111111111111111111111111111111111",
                    "data": "0x0102",
                    "value": 5
                }
            }),
        );
        let operation = if kind == Web3QuoteKind::Swap {
            "swap"
        } else {
            "bridge"
        };
        assert!(super::web3_approval_matches_request(
            &quote,
            &web3_request(
                "quote-1",
                operation,
                web3_evm(
                    "polygon_mainnet",
                    "0x1111111111111111111111111111111111111111",
                    Some("0x0102"),
                    "5",
                    137,
                ),
            )
        ));
        assert!(!super::web3_approval_matches_request(
            &quote,
            &web3_request(
                "quote-1",
                operation,
                web3_evm(
                    "polygon_mainnet",
                    "0x1111111111111111111111111111111111111111",
                    Some("0x0103"),
                    "5",
                    137
                ),
            )
        ));
    }

    let wrong_value_type = web3_quote(
        Web3QuoteKind::Swap,
        WalletChain::Evm,
        serde_json::json!({
            "network": "polygon_mainnet",
            "tx": {
                "to": "recipient",
                "data": "0x01",
                "value": true
            }
        }),
    );
    assert!(!super::web3_approval_matches_request(
        &wrong_value_type,
        &web3_request(
            "quote-1",
            "swap",
            web3_evm("polygon_mainnet", "recipient", Some("0x01"), "1", 137),
        )
    ));
}

#[test]
fn web3_solana_matches_exact_swap_or_bridge_transaction_bytes_only() {
    let quote = web3_quote(
        Web3QuoteKind::Bridge,
        WalletChain::Solana,
        serde_json::json!({ "tx": { "data": "010203" } }),
    );
    let request = web3_request(
        "quote-1",
        "bridge",
        Web3ExecutionIntent::Solana {
            tx_blob_hex: "010203".into(),
        },
    );
    assert!(super::web3_approval_matches_request(&quote, &request));
    let changed = web3_request(
        "quote-1",
        "bridge",
        Web3ExecutionIntent::Solana {
            tx_blob_hex: "010204".into(),
        },
    );
    assert!(!super::web3_approval_matches_request(&quote, &changed));

    let dapp_call = web3_quote(
        Web3QuoteKind::DappCall,
        WalletChain::Solana,
        serde_json::json!({ "tx": { "data": "010203" } }),
    );
    assert!(!super::web3_approval_matches_request(&dapp_call, &request));
    let wrong_signing_chain = web3_quote(
        Web3QuoteKind::Bridge,
        WalletChain::Evm,
        serde_json::json!({ "tx": { "data": "010203" } }),
    );
    assert!(!super::web3_approval_matches_request(
        &wrong_signing_chain,
        &request
    ));
}

#[test]
fn optional_quote_network_is_checked_only_when_it_is_a_string() {
    let contract = "0x1111111111111111111111111111111111111111";
    let quote = web3_quote(
        Web3QuoteKind::DappCall,
        WalletChain::Evm,
        serde_json::json!({
            "network": null,
            "contractAddress": contract,
            "calldata": "0x01",
            "valueRaw": "0"
        }),
    );
    let request = web3_request(
        "quote-1",
        "dapp_call",
        web3_evm("ethereum_mainnet", contract, Some("0x01"), "0", 1),
    );
    assert!(super::web3_approval_matches_request(&quote, &request));
}

const TRON_RECIPIENT: &str = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t";
const TRON_TEST_TOKEN_CONTRACT: &str = "TLyqzVGLV1srkB7dToTAEqgDSfPtXRJZYH";
// `encode_trc20_transfer_param(recipient, 12_345)` from the wallet's Tron
// producer: 12-byte ABI padding, the 20-byte Base58Check payload, then uint128.
const TRON_TRC20_PARAMETER: &str = "000000000000000000000000a614f803b6fd780986a42c78ec9c7f77e6ded13c0000000000000000000000000000000000000000000000000000000000003039";
const TRON_OTHER_RECIPIENT_PARAMETER: &str = "00000000000000000000000078c842ee63b253f8f0d2955bbc582c661a078c9d0000000000000000000000000000000000000000000000000000000000003039";

const SOLANA_TOKEN_OWNER: &str = "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk";
const SOLANA_TOKEN_RECIPIENT: &str = "Vote111111111111111111111111111111111111111";
const SOLANA_USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const SOLANA_SPL_TOKEN_PROGRAM_HEX: &str =
    "06ddf6e1d765a193d9cbe146ceeb79ac1cb485ed5f5b37913a8cf5857eff00a9";
// Golden legacy message emitted by the current producer for the accounts
// above, amount 1_000_000, and a 32-byte 0x04 blockhash.
const SOLANA_SPL_TRANSFER_MESSAGE: &str = concat!(
    "01000104",
    "f036276246a75b9de3349ed42b15e232f6518fc20f5fcd4f1d64e81f9bd258f7",
    "40d2f27c461f29e9feba1ab8acd39496a9c65b0ae62a713f01f941a993bf55ed",
    "a1cec0f53f78e59d435676236c81394f5240e8ff0163683daab51d3680258932",
    "06ddf6e1d765a193d9cbe146ceeb79ac1cb485ed5f5b37913a8cf5857eff00a9",
    "0404040404040404040404040404040404040404040404040404040404040404",
    "010303010200090340420f0000000000"
);

fn tron_token_quote() -> PreparedTransaction {
    let mut quote = prepared_quote(
        "tron-trc20-quote",
        WalletChain::Tron,
        PreparedKind::TokenTransfer,
        "12345",
        TRON_RECIPIENT,
    );
    quote.token_address = Some(TRON_TEST_TOKEN_CONTRACT.into());
    quote
}

fn tron_token_intent(expected_to: &str, parameter_hex: &str) -> NativeWalletExecutionIntent {
    NativeWalletExecutionIntent::Transaction {
        transaction: TransactionSpec::Tron {
            raw_data_hex: "00".into(),
            expected_to: expected_to.into(),
            expected_txid: "txid".into(),
            transfer: TronTransfer::Trc20 {
                parameter_hex: parameter_hex.into(),
            },
        },
    }
}

fn solana_token_quote() -> PreparedTransaction {
    let mut quote = prepared_quote(
        "solana-spl-quote",
        WalletChain::Solana,
        PreparedKind::TokenTransfer,
        "1000000",
        SOLANA_TOKEN_RECIPIENT,
    );
    quote.from_address = SOLANA_TOKEN_OWNER.into();
    quote.token_address = Some(SOLANA_USDC_MINT.into());
    quote
}

fn solana_message_intent(message_hex: &str) -> NativeWalletExecutionIntent {
    NativeWalletExecutionIntent::SolanaMessage {
        message_hex: message_hex.into(),
    }
}

fn replace_hex_byte(message_hex: &str, byte_offset: usize, replacement: u8) -> String {
    let start = byte_offset * 2;
    format!(
        "{}{:02x}{}",
        &message_hex[..start],
        replacement,
        &message_hex[start + 2..]
    )
}

#[test]
fn native_tron_trc20_intent_binds_contract_recipient_and_raw_amount() {
    let quote = tron_token_quote();
    assert_ne!(quote.to_address, quote.token_address.as_deref().unwrap());
    assert!(super::native_intent_matches_quote(
        &quote,
        &tron_token_intent(TRON_TEST_TOKEN_CONTRACT, TRON_TRC20_PARAMETER)
    ));

    // The transaction pays the token contract; the ABI parameter pays the
    // quoted recipient. Both facts must agree with the confirmed quote.
    assert!(!super::native_intent_matches_quote(
        &quote,
        &tron_token_intent(TRON_RECIPIENT, TRON_TRC20_PARAMETER)
    ));
    assert!(!super::native_intent_matches_quote(
        &quote,
        &tron_token_intent(TRON_RECIPIENT, TRON_OTHER_RECIPIENT_PARAMETER)
    ));
    assert!(!super::native_intent_matches_quote(
        &quote,
        &tron_token_intent(
            TRON_RECIPIENT,
            "000000000000000000000000a614f803b6fd780986a42c78ec9c7f77e6ded13c000000000000000000000000000000000000000000000000000000000000303a",
        )
    ));
    assert!(!super::native_intent_matches_quote(
        &quote,
        &tron_token_intent(TRON_RECIPIENT, "not-hex")
    ));
    assert!(!super::native_intent_matches_quote(
        &quote,
        &tron_token_intent(TRON_RECIPIENT, &TRON_TRC20_PARAMETER[..124])
    ));

    let mut wrong_chain = quote.clone();
    wrong_chain.chain = WalletChain::Evm;
    assert!(!super::native_intent_matches_quote(
        &wrong_chain,
        &tron_token_intent(TRON_RECIPIENT, TRON_TRC20_PARAMETER)
    ));
    let mut wrong_kind = quote;
    wrong_kind.kind = PreparedKind::NativeTransfer;
    assert!(!super::native_intent_matches_quote(
        &wrong_kind,
        &tron_token_intent(TRON_RECIPIENT, TRON_TRC20_PARAMETER)
    ));
}

#[test]
fn native_solana_spl_message_matches_the_quoted_transfer() {
    let quote = solana_token_quote();
    assert!(super::native_intent_matches_quote(
        &quote,
        &solana_message_intent(SOLANA_SPL_TRANSFER_MESSAGE)
    ));
}

#[test]
fn native_solana_spl_message_rejects_changes_to_the_quoted_transfer() {
    let quote = solana_token_quote();
    let message = SOLANA_SPL_TRANSFER_MESSAGE;

    let mut changed_recipient = quote.clone();
    changed_recipient.to_address = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM".into();
    assert!(!super::native_intent_matches_quote(
        &changed_recipient,
        &solana_message_intent(message)
    ));

    let mut changed_amount = quote.clone();
    changed_amount.amount_raw = "1000001".into();
    assert!(!super::native_intent_matches_quote(
        &changed_amount,
        &solana_message_intent(message)
    ));

    let mut changed_mint = quote.clone();
    changed_mint.token_address = Some("So11111111111111111111111111111111111111112".into());
    assert!(!super::native_intent_matches_quote(
        &changed_mint,
        &solana_message_intent(message)
    ));

    let wrong_signer = message.replacen(
        "f036276246a75b9de3349ed42b15e232f6518fc20f5fcd4f1d64e81f9bd258f7",
        "0101010101010101010101010101010101010101010101010101010101010101",
        1,
    );
    assert!(!super::native_intent_matches_quote(
        &quote,
        &solana_message_intent(&wrong_signer)
    ));

    let wrong_token_program = message.replacen(
        SOLANA_SPL_TOKEN_PROGRAM_HEX,
        "0000000000000000000000000000000000000000000000000000000000000000",
        1,
    );
    assert!(!super::native_intent_matches_quote(
        &quote,
        &solana_message_intent(&wrong_token_program)
    ));

    let wrong_opcode = message.replacen(
        "010303010200090340420f0000000000",
        "010303010200090240420f0000000000",
        1,
    );
    assert!(!super::native_intent_matches_quote(
        &quote,
        &solana_message_intent(&wrong_opcode)
    ));

    let wrong_amount_bytes = message.replacen(
        "010303010200090340420f0000000000",
        "010303010200090341420f0000000000",
        1,
    );
    assert!(!super::native_intent_matches_quote(
        &quote,
        &solana_message_intent(&wrong_amount_bytes)
    ));

    let wrong_program_index = message.replacen("0103030102000903", "0102030102000903", 1);
    assert!(!super::native_intent_matches_quote(
        &quote,
        &solana_message_intent(&wrong_program_index)
    ));

    let wrong_accounts = message.replacen("0103030102000903", "0103030002000903", 1);
    assert!(!super::native_intent_matches_quote(
        &quote,
        &solana_message_intent(&wrong_accounts)
    ));

    let wrong_header = replace_hex_byte(message, 0, 0);
    assert!(!super::native_intent_matches_quote(
        &quote,
        &solana_message_intent(&wrong_header)
    ));

    // The instruction count byte follows the header, 4-key vector, and blockhash.
    let mut extra_instruction = replace_hex_byte(message, 164, 2);
    extra_instruction.push_str("030000");
    assert!(!super::native_intent_matches_quote(
        &quote,
        &solana_message_intent(&extra_instruction)
    ));

    assert!(!super::native_intent_matches_quote(
        &quote,
        &solana_message_intent(&message[..message.len() - 2])
    ));
    assert!(!super::native_intent_matches_quote(
        &quote,
        &solana_message_intent(&format!("{message}00"))
    ));
    assert!(!super::native_intent_matches_quote(
        &quote,
        &solana_message_intent("not-hex")
    ));

    let mut wrong_chain = quote.clone();
    wrong_chain.chain = WalletChain::Evm;
    assert!(!super::native_intent_matches_quote(
        &wrong_chain,
        &solana_message_intent(message)
    ));
    let mut wrong_kind = quote;
    wrong_kind.kind = PreparedKind::NativeTransfer;
    assert!(!super::native_intent_matches_quote(
        &wrong_kind,
        &solana_message_intent(message)
    ));
}
