//! Pure Solana codecs shared by the wallet producer and intent verifier.
//!
//! These helpers perform no network or wallet operation. They decode public
//! addresses, derive Associated Token Accounts, and validate the one legacy SPL
//! transfer message shape currently produced by `TinyWallet`.

use curve25519_dalek::edwards::CompressedEdwardsY;
use sha2::{Digest, Sha256};

const TOKEN_PROGRAM_B58: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const ASSOCIATED_TOKEN_PROGRAM_B58: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
const PDA_MARKER: &[u8] = b"ProgramDerivedAddress";

/// Decode a base58 Solana public key into its 32-byte wire form.
///
/// # Errors
///
/// Returns an error if the address is invalid base58 or does not decode to
/// exactly 32 bytes.
pub fn pubkey_from_base58(address: &str) -> Result<[u8; 32], String> {
    let bytes = bs58::decode(address)
        .into_vec()
        .map_err(|error| format!("invalid base58 '{address}': {error}"))?;
    <[u8; 32]>::try_from(bytes.as_slice())
        .map_err(|_| format!("expected 32-byte pubkey, got {}", bytes.len()))
}

/// Encode a 32-byte Solana public key in base58.
#[must_use]
pub fn pubkey_to_base58(pubkey: &[u8; 32]) -> String {
    bs58::encode(pubkey).into_string()
}

/// Return the canonical SPL Token program public key.
///
/// # Errors
///
/// Returns an error if the checked-in program identifier is not a 32-byte
/// base58 public key.
pub fn token_program_id() -> Result<[u8; 32], String> {
    pubkey_from_base58(TOKEN_PROGRAM_B58)
}

/// Derive the canonical Associated Token Account for `owner` and `mint`.
///
/// # Errors
///
/// Returns an error if a checked-in program identifier is invalid or no
/// off-curve PDA bump can be found.
pub fn associated_token_address(owner: &[u8; 32], mint: &[u8; 32]) -> Result<[u8; 32], String> {
    let token_program = token_program_id()?;
    let associated_token_program = pubkey_from_base58(ASSOCIATED_TOKEN_PROGRAM_B58)?;
    find_program_address(
        &[&owner[..], &token_program[..], &mint[..]],
        &associated_token_program,
    )
}

/// Decode hexadecimal serialized message bytes.
#[must_use]
pub fn decode_message_hex(value: &str) -> Option<Vec<u8>> {
    if value.is_empty() || value.len() % 2 != 0 {
        return None;
    }

    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| Some((hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?))
        .collect()
}

/// Check that `message` is exactly the legacy SPL `Transfer` built for the
/// quoted owner, recipient, mint, and amount.
///
/// The current producer emits one signer, four account keys, one instruction,
/// and no address table or additional instruction. The recent blockhash is
/// deliberately not checked because it must be fetched anew for each turn.
#[must_use]
pub fn matches_legacy_spl_transfer_message(
    message: &[u8],
    owner: &[u8; 32],
    recipient: &[u8; 32],
    mint: &[u8; 32],
    amount: u64,
) -> bool {
    let Ok(source_ata) = associated_token_address(owner, mint) else {
        return false;
    };
    let Ok(destination_ata) = associated_token_address(recipient, mint) else {
        return false;
    };
    let Ok(token_program) = token_program_id() else {
        return false;
    };

    if message.get(..3) != Some(&[1, 0, 1]) {
        return false;
    }
    let mut cursor = 3;
    if !read_count(message, &mut cursor, 4) {
        return false;
    }
    let Some(actual_owner) = take_array::<32>(message, &mut cursor) else {
        return false;
    };
    let Some(actual_source_ata) = take_array::<32>(message, &mut cursor) else {
        return false;
    };
    let Some(actual_destination_ata) = take_array::<32>(message, &mut cursor) else {
        return false;
    };
    let Some(actual_token_program) = take_array::<32>(message, &mut cursor) else {
        return false;
    };
    if actual_owner != *owner
        || actual_source_ata != source_ata
        || actual_destination_ata != destination_ata
        || actual_token_program != token_program
        || take_array::<32>(message, &mut cursor).is_none()
        || !read_count(message, &mut cursor, 1)
    {
        return false;
    }

    if message.get(cursor) != Some(&3) {
        return false;
    }
    cursor += 1;
    if !read_count(message, &mut cursor, 3)
        || take_array::<3>(message, &mut cursor) != Some([1, 2, 0])
        || !read_count(message, &mut cursor, 9)
    {
        return false;
    }
    let Some(instruction) = take_array::<9>(message, &mut cursor) else {
        return false;
    };
    let Ok(amount_bytes) = <[u8; 8]>::try_from(&instruction[1..]) else {
        return false;
    };
    instruction[0] == 3 && u64::from_le_bytes(amount_bytes) == amount && cursor == message.len()
}

fn find_program_address(seeds: &[&[u8]], program_id: &[u8; 32]) -> Result<[u8; 32], String> {
    for bump in (0u8..=255).rev() {
        let mut hasher = Sha256::new();
        for seed in seeds {
            hasher.update(seed);
        }
        hasher.update([bump]);
        hasher.update(program_id);
        hasher.update(PDA_MARKER);
        let candidate: [u8; 32] = hasher.finalize().into();
        if CompressedEdwardsY(candidate).decompress().is_none() {
            return Ok(candidate);
        }
    }
    Err("no off-curve PDA found".to_string())
}

fn read_count(message: &[u8], cursor: &mut usize, expected: u8) -> bool {
    let Some(&count) = message.get(*cursor) else {
        return false;
    };
    *cursor += 1;
    count == expected
}

fn take_array<const N: usize>(message: &[u8], cursor: &mut usize) -> Option<[u8; N]> {
    let end = cursor.checked_add(N)?;
    let bytes = message.get(*cursor..end)?;
    *cursor = end;
    bytes.try_into().ok()
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
#[path = "solana_tests.rs"]
mod tests;
