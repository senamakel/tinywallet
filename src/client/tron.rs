//! Tron balance query against the TronGrid REST API.

use serde_json::json;

use super::{Error, Result, network_id};
use crate::asset::Network;
use crate::rpc::Transport;

/// Native balance in sun (1 TRX = 1_000_000 sun).
///
/// TronGrid wants the hex address form, not the base58check one the user sees.
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
