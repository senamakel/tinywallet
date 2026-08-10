//! EVM balance query: `eth_getBalance`.

use serde_json::json;

use super::{Error, Result, network_id, parse_hex_u128};
use crate::asset::{EvmNetwork, Network};
use crate::rpc::Transport;

/// Native balance in wei.
///
/// Queried at the `latest` block rather than `pending`: a pending balance
/// reflects transactions that may still be dropped, which would show a user a
/// number that can go backwards without anything having failed.
pub(super) async fn balance(
    transport: &dyn Transport,
    network: EvmNetwork,
    address: &str,
) -> Result<u128> {
    let id = network_id(Network::Evm(network));
    let raw = transport
        .json_rpc(id, "eth_getBalance", json!([address, "latest"]))
        .await?;
    let hex = raw.as_str().ok_or_else(|| Error::MalformedResponse {
        network: id,
        operation: "eth_getBalance",
        detail: format!("expected a hex string, got {raw}"),
    })?;
    parse_hex_u128(id, "eth_getBalance", hex)
}
