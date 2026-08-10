//! Minimal RLP encoder — enough to serialise an Ethereum legacy transaction.
//!
//! RLP encodes exactly two things: a byte string and a list of items. That is
//! the whole specification, and a legacy transaction is one list of nine byte
//! strings, so this is deliberately small rather than a general-purpose codec.
//!
//! ## Why this is hand-written rather than a dependency
//!
//! An RLP crate would arrive with an entire Ethereum type stack behind it. The
//! encoder is under a hundred lines and is pinned against the published EIP-155
//! vector in [`super::evm`], which exercises every branch below — so the
//! trade is a small, tested, dependency-free encoder against a large
//! transitive graph.
//!
//! ## The integer rule is where RLP implementations go wrong
//!
//! RLP has no integer type. Numbers are encoded as **big-endian byte strings
//! with no leading zeros**, and zero is the *empty* string rather than `0x00`.
//! Get that wrong and the encoding is still well-formed RLP — it simply hashes
//! to a different value, so the signature is valid for a transaction nobody
//! meant to send. [`encode_uint`] is the only place that rule lives.

/// Encode a byte string.
///
/// Three cases, per the specification:
/// - a single byte below `0x80` is itself, with no prefix;
/// - up to 55 bytes take a `0x80 + len` prefix;
/// - longer takes `0xb7 + len_of_len`, then the length, then the payload.
pub(super) fn encode_bytes(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() == 1 && bytes[0] < 0x80 {
        return vec![bytes[0]];
    }
    let mut out = encode_length(bytes.len(), 0x80);
    out.extend_from_slice(bytes);
    out
}

/// Encode a list of already-encoded items.
///
/// Takes encoded items rather than raw ones because RLP lists are defined over
/// encoded payloads — the length prefix covers the concatenated *encodings*,
/// not the values.
pub(super) fn encode_list(items: &[Vec<u8>]) -> Vec<u8> {
    let payload: Vec<u8> = items.concat();
    let mut out = encode_length(payload.len(), 0xc0);
    out.extend_from_slice(&payload);
    out
}

/// Encode an unsigned integer as RLP's canonical big-endian, no-leading-zeros
/// byte string.
///
/// Zero encodes as the empty string — **not** as `0x00`. See the module docs:
/// this is the rule that silently changes what a signature commits to.
pub(super) fn encode_uint(value: u128) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    let first = bytes.iter().position(|b| *b != 0).unwrap_or(bytes.len());
    encode_bytes(&bytes[first..])
}

/// Encode a big-endian byte slice as an integer, stripping leading zeros.
///
/// Used for signature `r` and `s`, which are 32-byte values that must follow
/// the same no-leading-zeros rule as any other RLP integer.
pub(super) fn encode_uint_bytes(bytes: &[u8]) -> Vec<u8> {
    let first = bytes.iter().position(|b| *b != 0).unwrap_or(bytes.len());
    encode_bytes(&bytes[first..])
}

/// Build the length prefix for a payload, given the offset that distinguishes
/// strings (`0x80`) from lists (`0xc0`).
fn encode_length(len: usize, offset: u8) -> Vec<u8> {
    if len <= 55 {
        // `len` is at most 55 here, so this cannot truncate.
        #[allow(clippy::cast_possible_truncation)]
        return vec![offset + len as u8];
    }
    let len_bytes = len.to_be_bytes();
    let first = len_bytes
        .iter()
        .position(|b| *b != 0)
        .unwrap_or(len_bytes.len());
    let significant = &len_bytes[first..];
    // `significant.len()` is at most 8 (usize), well inside u8.
    #[allow(clippy::cast_possible_truncation)]
    let mut out = vec![offset + 55 + significant.len() as u8];
    out.extend_from_slice(significant);
    out
}

#[cfg(test)]
mod test {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::{encode_bytes, encode_list, encode_uint, encode_uint_bytes};

    #[test]
    fn a_single_low_byte_is_itself() {
        assert_eq!(encode_bytes(&[0x00]), vec![0x00]);
        assert_eq!(encode_bytes(&[0x7f]), vec![0x7f]);
    }

    #[test]
    fn a_single_high_byte_takes_a_prefix() {
        // 0x80 is not below 0x80, so it is a one-byte string, not a bare byte.
        assert_eq!(encode_bytes(&[0x80]), vec![0x81, 0x80]);
    }

    #[test]
    fn an_empty_string_is_0x80() {
        assert_eq!(encode_bytes(&[]), vec![0x80]);
    }

    #[test]
    fn short_strings_take_a_single_length_prefix() {
        // "dog" — the specification's own example.
        assert_eq!(encode_bytes(b"dog"), vec![0x83, b'd', b'o', b'g']);
    }

    #[test]
    fn strings_longer_than_55_bytes_take_a_length_of_length() {
        let payload = vec![0xaa_u8; 56];
        let encoded = encode_bytes(&payload);
        assert_eq!(encoded[0], 0xb8, "0xb7 + 1 length byte");
        assert_eq!(encoded[1], 56);
        assert_eq!(encoded.len(), 58);

        let long = vec![0xbb_u8; 1024];
        let encoded = encode_bytes(&long);
        assert_eq!(encoded[0], 0xb9, "0xb7 + 2 length bytes");
        assert_eq!(&encoded[1..3], &[0x04, 0x00]);
    }

    #[test]
    fn zero_encodes_as_the_empty_string_not_as_a_zero_byte() {
        // The rule that silently changes what a signature commits to.
        assert_eq!(encode_uint(0), vec![0x80]);
        assert_ne!(encode_uint(0), vec![0x00]);
    }

    #[test]
    fn integers_carry_no_leading_zeros() {
        assert_eq!(encode_uint(1), vec![0x01]);
        assert_eq!(encode_uint(127), vec![0x7f]);
        assert_eq!(encode_uint(128), vec![0x81, 0x80]);
        assert_eq!(encode_uint(1024), vec![0x82, 0x04, 0x00]);
        // 20 gwei, from the EIP-155 vector.
        assert_eq!(
            encode_uint(20_000_000_000),
            vec![0x85, 0x04, 0xa8, 0x17, 0xc8, 0x00]
        );
    }

    #[test]
    fn integer_byte_slices_are_stripped_the_same_way() {
        let mut padded = [0u8; 32];
        padded[31] = 1;
        assert_eq!(encode_uint_bytes(&padded), vec![0x01]);
        assert_eq!(
            encode_uint_bytes(&[0u8; 32]),
            vec![0x80],
            "all-zero is empty"
        );
    }

    #[test]
    fn an_empty_list_is_0xc0() {
        assert_eq!(encode_list(&[]), vec![0xc0]);
    }

    #[test]
    fn a_list_prefixes_the_concatenated_encodings() {
        // ["cat", "dog"] from the specification.
        let items = vec![encode_bytes(b"cat"), encode_bytes(b"dog")];
        assert_eq!(
            encode_list(&items),
            vec![0xc8, 0x83, b'c', b'a', b't', 0x83, b'd', b'o', b'g']
        );
    }

    #[test]
    fn a_long_list_takes_a_length_of_length() {
        let items: Vec<Vec<u8>> = (0..30).map(|_| encode_bytes(&[0xcc_u8; 2])).collect();
        let encoded = encode_list(&items);
        assert_eq!(encoded[0], 0xf8, "0xf7 + 1 length byte");
        assert_eq!(encoded[1], 90, "30 items x 3 bytes each");
    }
}
