//! Integration tests for the public crate surface.
//!
//! These tests link against the crate as a downstream consumer would: they can
//! only use what `src/lib.rs` re-exports. Treat them as the regression suite
//! for the crate's public contract — if a change breaks a test here, it is a
//! breaking change for users.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::str::FromStr;

use tinywallet::{Chain, Error, address};

#[test]
fn consumers_can_validate_via_chain_generic_dispatch() {
    let addr = address::validate(Chain::Evm, "0x52908400098527886E0F7030069857D2E4169EE7").unwrap();
    assert_eq!(addr, "0x52908400098527886E0F7030069857D2E4169EE7");
}

#[test]
fn consumers_can_reach_each_chain_module_directly() {
    assert!(address::btc::validate("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4").is_ok());
    assert!(address::evm::validate("0x52908400098527886E0F7030069857D2E4169EE7").is_ok());
    assert!(address::solana::validate("11111111111111111111111111111111").is_ok());
    assert!(address::tron::validate("TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t").is_ok());
}

#[test]
fn the_btc_sender_rule_is_reachable_and_narrower_than_the_recipient_rule() {
    // The distinction only matters if consumers can actually get at it.
    let legacy = "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2";
    assert!(address::btc::validate(legacy).is_ok());
    assert!(matches!(
        address::btc::validate_sender(legacy),
        Err(Error::UnsupportedAddressType { .. })
    ));
}

#[test]
fn consumers_can_match_on_error_variants() {
    // Errors are the API a caller reacts to, so the variants and their fields
    // must be public and structurally matchable.
    match address::validate(Chain::Btc, "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx") {
        Err(Error::WrongNetwork {
            chain, expected, ..
        }) => {
            assert_eq!(chain, Chain::Btc);
            assert_eq!(expected, "mainnet");
        }
        other => panic!("expected WrongNetwork, got {other:?}"),
    }
}

#[test]
fn consumers_can_convert_encodings() {
    let bytes = address::solana::decode("11111111111111111111111111111111").unwrap();
    assert_eq!(
        address::solana::encode(&bytes),
        "11111111111111111111111111111111"
    );

    let hex = address::tron::to_hex("TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t").unwrap();
    assert!(hex.starts_with("41"));
}

#[test]
fn consumers_can_parse_and_enumerate_chains() {
    assert_eq!(Chain::from_str("ethereum").unwrap(), Chain::Evm);
    assert!(Chain::ALL.contains(&Chain::Solana));
}

#[test]
fn consumers_can_check_eip55_checksums() {
    let checksummed = "0x52908400098527886E0F7030069857D2E4169EE7";
    assert!(address::evm::is_checksum_valid(checksummed).unwrap());
    assert_eq!(
        address::evm::to_checksummed(&checksummed.to_lowercase()).unwrap(),
        checksummed
    );
}
