//! Unit tests for Bitcoin address validation.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{validate, validate_sender};
use crate::{Chain, Error};

/// P2WPKH — native segwit. The only type valid as a sender.
const P2WPKH: &str = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";
/// P2PKH — legacy. A fine recipient.
const P2PKH: &str = "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2";
/// P2SH — wrapped segwit or multisig. A fine recipient.
const P2SH: &str = "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy";
/// P2WSH — native segwit script hash. A fine recipient.
const P2WSH: &str = "bc1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3qccfmv3";

#[test]
fn accepts_every_mainnet_address_type_as_a_recipient() {
    for addr in [P2WPKH, P2PKH, P2SH, P2WSH] {
        assert!(validate(addr).is_ok(), "{addr} should validate");
    }
}

#[test]
fn trims_surrounding_whitespace() {
    assert_eq!(validate(&format!("  {P2WPKH}\n")).unwrap(), P2WPKH);
}

#[test]
fn rejects_an_empty_address() {
    assert_eq!(
        validate("  ").unwrap_err(),
        Error::EmptyAddress { chain: Chain::Btc }
    );
}

#[test]
fn rejects_a_malformed_address() {
    assert!(matches!(
        validate("not-an-address").unwrap_err(),
        Error::InvalidAddress { .. }
    ));
}

#[test]
fn rejects_a_mistyped_address_via_its_checksum() {
    // Bitcoin addresses are checksummed, so a single changed character is
    // caught rather than naming a different account.
    let mut chars: Vec<char> = P2PKH.chars().collect();
    chars[5] = if chars[5] == 'a' { 'b' } else { 'a' };
    let typo: String = chars.into_iter().collect();
    assert_ne!(typo, P2PKH, "the fixture must actually differ");
    assert!(
        validate(&typo).is_err(),
        "a checksum failure must be caught"
    );
}

#[test]
fn rejects_a_testnet_address_as_the_wrong_network() {
    // Well-formed, but on the wrong network — a distinct variant because it is
    // the failure a caller is likely to handle rather than merely report.
    let testnet = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";
    match validate(testnet).unwrap_err() {
        Error::WrongNetwork {
            chain,
            address,
            expected,
            ..
        } => {
            assert_eq!(chain, Chain::Btc);
            assert_eq!(address, testnet);
            assert_eq!(expected, "mainnet");
        }
        other => panic!("expected WrongNetwork, got {other:?}"),
    }
}

#[test]
fn accepts_p2wpkh_as_a_sender() {
    assert_eq!(validate_sender(P2WPKH).unwrap(), P2WPKH);
}

#[test]
fn rejects_every_non_p2wpkh_type_as_a_sender() {
    // These are all valid recipients. The sender rule is strictly narrower
    // because signing is only implemented for P2WPKH.
    for addr in [P2PKH, P2SH, P2WSH] {
        assert!(
            validate(addr).is_ok(),
            "{addr} must remain a valid recipient"
        );
        match validate_sender(addr).unwrap_err() {
            Error::UnsupportedAddressType { chain, address, .. } => {
                assert_eq!(chain, Chain::Btc);
                assert_eq!(address, addr);
            }
            other => panic!("expected UnsupportedAddressType for {addr}, got {other:?}"),
        }
    }
}

#[test]
fn sender_validation_still_reports_the_underlying_failure_first() {
    // A malformed or wrong-network address should not be reported as an
    // unsupported *type* — that would point at the wrong fix.
    assert!(matches!(
        validate_sender("garbage").unwrap_err(),
        Error::InvalidAddress { .. }
    ));
    assert!(matches!(
        validate_sender("tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx").unwrap_err(),
        Error::WrongNetwork { .. }
    ));
    assert!(matches!(
        validate_sender("   ").unwrap_err(),
        Error::EmptyAddress { .. }
    ));
}
