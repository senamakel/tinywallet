//! Tests for transport-free Solana public-key and transfer-message codecs.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{
    associated_token_address, decode_message_hex, matches_legacy_spl_transfer_message,
    pubkey_from_base58, pubkey_to_base58, token_program_id,
};

const OWNER: &str = "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk";
const RECIPIENT: &str = "Vote111111111111111111111111111111111111111";
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

#[test]
fn public_keys_round_trip_through_base58() {
    let owner = pubkey_from_base58(OWNER).unwrap();
    assert_eq!(pubkey_to_base58(&owner), OWNER);
    assert_eq!(
        pubkey_from_base58("tooShort").unwrap_err(),
        "expected 32-byte pubkey, got 6"
    );
    assert!(
        pubkey_from_base58("0OIl")
            .unwrap_err()
            .starts_with("invalid base58")
    );
}

#[test]
fn associated_token_addresses_match_the_canonical_program_derivation() {
    let owner = pubkey_from_base58(OWNER).unwrap();
    let recipient = pubkey_from_base58(RECIPIENT).unwrap();
    let mint = pubkey_from_base58(USDC_MINT).unwrap();

    assert_eq!(
        pubkey_to_base58(&associated_token_address(&owner, &mint).unwrap()),
        "5N3f1tj9v1vc5TUZ8S7mCAnVmjVKrfnzXWhxLaxyZAgt"
    );
    assert_eq!(
        pubkey_to_base58(&associated_token_address(&recipient, &mint).unwrap()),
        "BtdVcnjAZ3eMPJnFJmhyav1BzsTr4rSwCiGJxVUkhT21"
    );
    assert_eq!(
        pubkey_to_base58(&token_program_id().unwrap()),
        "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
    );
}

#[test]
fn message_hex_decoder_accepts_case_and_prefix_but_rejects_malformed_input() {
    assert_eq!(decode_message_hex("00aF"), Some(vec![0, 0xaf]));
    assert_eq!(decode_message_hex("0x00aF"), None);
    assert_eq!(decode_message_hex(""), None);
    assert_eq!(decode_message_hex("0"), None);
    assert_eq!(decode_message_hex("zz"), None);
}

#[test]
fn legacy_spl_transfer_validator_rejects_unrelated_message_bytes() {
    let owner = pubkey_from_base58(OWNER).unwrap();
    let recipient = pubkey_from_base58(RECIPIENT).unwrap();
    let mint = pubkey_from_base58(USDC_MINT).unwrap();
    assert!(!matches_legacy_spl_transfer_message(
        &[],
        &owner,
        &recipient,
        &mint,
        1_000_000
    ));
}
