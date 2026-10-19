//! The payment record as it is stored, and the v1 shape it migrates from.

use serde::{Deserialize, Deserializer, Serialize};

/// Where a stored payment stands. The same four states, spelled the same way,
/// as the implementation's public status type, which converts to and from this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoredPaymentStatus {
    /// Signed and sent, not yet settled.
    Pending,
    /// A trusted settlement receipt confirms payment.
    Settled,
    /// A trusted result established that the payment did not settle.
    Failed,
    /// Refused before anything was signed.
    Denied,
}

/// One payment as stored in `wallet.payments`: `snake_case`, every instant an
/// epoch-millisecond `*_ms` field, no wrapper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredPayment {
    /// Unique id of the attempt.
    pub id: String,
    /// The URL the payment bought access to.
    pub url: String,
    /// The asset paid in.
    pub asset: String,
    /// The amount in atomic units of `asset`.
    pub amount_atomic: u64,
    /// The amount formatted for people.
    pub amount_display: String,
    /// Who was paid.
    pub recipient: String,
    /// The network, in CAIP-2 form.
    pub network: String,
    /// The settlement transaction, once reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_signature: Option<String>,
    /// Where the payment stands.
    pub status: StoredPaymentStatus,
    /// When this state was written, in epoch milliseconds.
    pub timestamp_ms: i64,
    /// When an exported authorization becomes unusable, in epoch milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_valid_until_ms: Option<i64>,
    /// Solana's `lastValidBlockHeight` for the exported transaction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solana_last_valid_block_height: Option<u64>,
    /// When the authorization was observed expired, in epoch milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_expired_at_ms: Option<i64>,
    /// The block height at which the expiry was observed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_expiry_observed_height: Option<u64>,
    /// The session that made the payment.
    pub session_id: String,
    /// The conversation thread, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// The budget hold this entry committed, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committed_hold: Option<String>,
}

/// A v1 `x402.payments` entry: the camelCase wire record with RFC 3339
/// instants, and the hold metadata beside it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyPayment {
    id: String,
    url: String,
    asset: String,
    amount_atomic: u64,
    amount_display: String,
    recipient: String,
    network: String,
    #[serde(default)]
    tx_signature: Option<String>,
    status: StoredPaymentStatus,
    #[serde(deserialize_with = "instant_ms")]
    timestamp: i64,
    #[serde(default, deserialize_with = "optional_instant_ms")]
    authorization_valid_until: Option<i64>,
    #[serde(default)]
    solana_last_valid_block_height: Option<u64>,
    #[serde(default, deserialize_with = "optional_instant_ms")]
    authorization_expired_at: Option<i64>,
    #[serde(default)]
    authorization_expiry_observed_height: Option<u64>,
    session_id: String,
    #[serde(default)]
    thread_id: Option<String>,
    #[serde(default, rename = "committed_hold")]
    committed_hold: Option<String>,
}

impl From<LegacyPayment> for StoredPayment {
    fn from(legacy: LegacyPayment) -> Self {
        Self {
            id: legacy.id,
            url: legacy.url,
            asset: legacy.asset,
            amount_atomic: legacy.amount_atomic,
            amount_display: legacy.amount_display,
            recipient: legacy.recipient,
            network: legacy.network,
            tx_signature: legacy.tx_signature,
            status: legacy.status,
            timestamp_ms: legacy.timestamp,
            authorization_valid_until_ms: legacy.authorization_valid_until,
            solana_last_valid_block_height: legacy.solana_last_valid_block_height,
            authorization_expired_at_ms: legacy.authorization_expired_at,
            authorization_expiry_observed_height: legacy.authorization_expiry_observed_height,
            session_id: legacy.session_id,
            thread_id: legacy.thread_id,
            committed_hold: legacy.committed_hold,
        }
    }
}

fn instant_ms<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
    let text = String::deserialize(deserializer)?;
    parse_rfc3339_ms(&text)
        .ok_or_else(|| serde::de::Error::custom(format!("`{text}` is not an RFC 3339 instant")))
}

fn optional_instant_ms<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<i64>, D::Error> {
    Option::<String>::deserialize(deserializer)?
        .map(|text| {
            parse_rfc3339_ms(&text).ok_or_else(|| {
                serde::de::Error::custom(format!("`{text}` is not an RFC 3339 instant"))
            })
        })
        .transpose()
}

/// An RFC 3339 instant (`2026-03-15T12:00:00.250Z`, or with a `+hh:mm`
/// offset) as epoch milliseconds, by integer arithmetic only. Fractions finer
/// than a millisecond are truncated.
pub(super) fn parse_rfc3339_ms(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    let digits = |from: usize, len: usize| -> Option<i64> {
        let part = bytes.get(from..from + len)?;
        if !part.iter().all(u8::is_ascii_digit) {
            return None;
        }
        part.iter()
            .try_fold(0_i64, |n, d| Some(n * 10 + i64::from(d - b'0')))
    };
    let separator = |at: usize, want: &[u8]| bytes.get(at).is_some_and(|b| want.contains(b));
    if !(separator(4, b"-")
        && separator(7, b"-")
        && separator(10, b"Tt ")
        && separator(13, b":")
        && separator(16, b":"))
    {
        return None;
    }
    let (year, month, day) = (digits(0, 4)?, digits(5, 2)?, digits(8, 2)?);
    let (hour, minute, second) = (digits(11, 2)?, digits(14, 2)?, digits(17, 2)?);
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        2 if leap_year => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return None,
    };
    if !(1..=days_in_month).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let mut at = 19;
    let mut millis = 0;
    if bytes.get(at) == Some(&b'.') {
        let start = at + 1;
        let mut end = start;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        if end == start {
            return None;
        }
        for (place, digit) in bytes[start..end].iter().take(3).enumerate() {
            millis += i64::from(digit - b'0') * [100, 10, 1][place];
        }
        at = end;
    }
    let offset_minutes = match bytes.get(at)? {
        b'Z' | b'z' if at + 1 == bytes.len() => 0,
        sign @ (b'+' | b'-') if at + 6 == bytes.len() && separator(at + 3, b":") => {
            let hours = digits(at + 1, 2)?;
            let minute_part = digits(at + 4, 2)?;
            if hours > 23 || minute_part > 59 {
                return None;
            }
            let minutes = hours * 60 + minute_part;
            if *sign == b'-' { -minutes } else { minutes }
        }
        _ => return None,
    };
    // Days from civil (Howard Hinnant's algorithm), proleptic Gregorian.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second - offset_minutes * 60;
    Some(seconds * 1_000 + millis)
}
