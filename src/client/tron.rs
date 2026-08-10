//! Tron balance query against the `TronGrid` REST API.

use serde_json::{Value, json};

use super::{Error, Result, network_id};
use crate::asset::Network;
use crate::rpc::Transport;

/// Native balance in sun (1 TRX = `1_000_000` sun).
///
/// `TronGrid` wants the hex address form, not the base58check one the user sees.
/// An account that has never been funded is returned as `{}` rather than a
/// balance of zero, so a missing `balance` field means zero here — unlike the
/// other chains, where a missing field would be malformed.
pub(super) async fn balance(transport: &dyn Transport, address: &str) -> Result<u128> {
    let id = network_id(Network::Tron);
    let hex = crate::address::tron::to_hex(address)?;
    let body = json!({ "address": hex, "visible": false }).to_string();

    let raw = transport
        .rest_post(id, "wallet/getaccount", body, "application/json")
        .await?;
    let parsed: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| Error::MalformedResponse {
            network: id,
            operation: "wallet/getaccount",
            detail: format!("not JSON: {e}"),
        })?;

    Ok(parsed
        .get("balance")
        .and_then(serde_json::Value::as_u64)
        .map_or(0, u128::from))
}

/// Build a transfer on the node, verify it, sign it and broadcast it.
///
/// Returns the transaction id.
///
/// Tron has the node build the transaction, so the returned `raw_data` is
/// checked against what was requested *before* it is signed — see
/// [`crate::tx::tron::verify_transfer`]. Signing blind would let a compromised
/// endpoint have its own transfer authorised.
///
/// # Errors
///
/// See [`Error`]. [`crate::tx::Error::UntrustedResponse`] surfaces as
/// [`Error::Tx`] when the node's answer does not match the request.
pub(super) async fn send(
    transport: &dyn Transport,
    from: &str,
    to: &str,
    amount: u64,
    secret_key: &[u8],
) -> Result<String> {
    let id = network_id(Network::Tron);
    let owner_hex = crate::address::tron::to_hex(from)?;
    let to_hex = crate::address::tron::to_hex(to)?;

    let create = json!({
        "owner_address": owner_hex,
        "to_address": to_hex,
        "amount": amount,
        "visible": false,
    })
    .to_string();
    let body = transport
        .rest_post(id, "wallet/createtransaction", create, "application/json")
        .await?;
    let built: Value = serde_json::from_str(&body).map_err(|e| Error::MalformedResponse {
        network: id,
        operation: "wallet/createtransaction",
        detail: format!("not JSON: {e}"),
    })?;

    let raw_data_hex = built
        .get("raw_data_hex")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::MalformedResponse {
            network: id,
            operation: "wallet/createtransaction",
            detail: format!("no raw_data_hex in {built}"),
        })?;
    let txid =
        built
            .get("txID")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::MalformedResponse {
                network: id,
                operation: "wallet/createtransaction",
                detail: format!("no txID in {built}"),
            })?;

    // Verify before signing — the node built this, not us.
    crate::tx::tron::verify_transfer(raw_data_hex, to, txid).map_err(Error::Tx)?;

    let signature = crate::tx::tron::sign(raw_data_hex, secret_key).map_err(Error::Tx)?;
    let mut signed = built.clone();
    signed["signature"] = json!([crate::tx::tron::signature_hex(&signature)]);

    let body = transport
        .rest_post(
            id,
            "wallet/broadcasttransaction",
            signed.to_string(),
            "application/json",
        )
        .await?;
    let result: Value = serde_json::from_str(&body).map_err(|e| Error::MalformedResponse {
        network: id,
        operation: "wallet/broadcasttransaction",
        detail: format!("not JSON: {e}"),
    })?;

    // TronGrid answers 200 with {"result": false, "message": "..."} on a
    // rejection, so a successful HTTP status is not a successful broadcast.
    if result.get("result").and_then(Value::as_bool) == Some(true) {
        Ok(txid.to_string())
    } else {
        Err(Error::Transport(crate::rpc::TransportError::Rpc {
            network: id,
            message: format!("broadcast rejected: {result}"),
        }))
    }
}
