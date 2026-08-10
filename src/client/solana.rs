//! Solana balance query: `getBalance`.

use serde::Deserialize;
use serde_json::json;

use super::{Error, Result, decode, network_id};
use crate::asset::{Network, SolanaCluster};
use crate::rpc::Transport;
use crate::tx::solana::NativeTransfer;

/// Solana wraps most results in a context envelope; only `value` is wanted.
#[derive(Deserialize)]
struct BalanceResult {
    value: u64,
}

/// Native balance in lamports.
pub(super) async fn balance(transport: &dyn Transport, address: &str) -> Result<u128> {
    // The cluster does not change the request, only which endpoint the host
    // routes it to, so any cluster gives the same NetworkId here.
    let id = network_id(Network::Solana(SolanaCluster::Mainnet));
    let raw = transport
        .json_rpc(id, "getBalance", json!([address]))
        .await?;
    let parsed: BalanceResult = decode(id, "getBalance", raw)?;
    Ok(u128::from(parsed.value))
}

/// Fetch a recent blockhash, build, sign and broadcast a native SOL transfer.
///
/// Returns the transaction signature, which is what Solana uses as a
/// transaction id.
///
/// The blockhash is fetched immediately before signing rather than passed in,
/// because it doubles as the transaction's expiry: Solana rejects a
/// transaction whose blockhash is more than a few minutes old, so a stale one
/// produces a transaction that is valid, signed, and permanently unlandable.
///
/// # Errors
///
/// [`Error::Transport`] for a network failure, [`Error::MalformedResponse`] if
/// the blockhash response is unusable, and [`Error::Tx`] if signing fails.
pub(super) async fn send(
    transport: &dyn Transport,
    cluster: SolanaCluster,
    from: &str,
    to: &str,
    lamports: u64,
    secret_key: &[u8],
) -> Result<String> {
    let id = network_id(Network::Solana(cluster));

    let raw = transport
        .json_rpc(
            id,
            "getLatestBlockhash",
            serde_json::json!([{ "commitment": "finalized" }]),
        )
        .await?;
    let blockhash = raw
        .get("value")
        .and_then(|v| v.get("blockhash"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| Error::MalformedResponse {
            network: id,
            operation: "getLatestBlockhash",
            detail: format!("no value.blockhash in {raw}"),
        })?;

    let transfer = NativeTransfer {
        from: from.to_string(),
        to: to.to_string(),
        lamports,
        recent_blockhash: blockhash.to_string(),
    };
    let signed = transfer.sign(secret_key).map_err(Error::Tx)?;

    // Solana takes the wire transaction base64-encoded; the default encoding
    // is base58, which is both slower and size-limited.
    let encoded = base64_encode(&signed);
    let raw = transport
        .json_rpc(
            id,
            "sendTransaction",
            serde_json::json!([encoded, { "encoding": "base64" }]),
        )
        .await?;
    raw.as_str()
        .map(ToString::to_string)
        .ok_or_else(|| Error::MalformedResponse {
            network: id,
            operation: "sendTransaction",
            detail: format!("expected a signature, got {raw}"),
        })
}

/// Minimal standard base64 encoder.
///
/// Hand-written to avoid a dependency for one call site: this is the only
/// place in the crate that needs base64, and the alphabet plus padding rules
/// are a dozen lines.
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = chunk.get(1).copied().map_or(0, u32::from);
        let b2 = chunk.get(2).copied().map_or(0, u32::from);
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(triple >> 18) as usize & 0x3f] as char);
        out.push(ALPHABET[(triple >> 12) as usize & 0x3f] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(triple >> 6) as usize & 0x3f] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[triple as usize & 0x3f] as char
        } else {
            '='
        });
    }
    out
}
