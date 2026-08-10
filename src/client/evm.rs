//! EVM balance query: `eth_getBalance`.

use serde_json::json;

use super::{Error, Result, network_id, parse_hex_u128};
use crate::asset::{EvmNetwork, Network};
use crate::rpc::Transport;
use crate::tx::evm::LegacyTransaction;

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

/// Fetch the fee and nonce parameters a transfer needs, build it, sign it, and
/// broadcast it.
///
/// Returns the transaction hash the network assigned.
///
/// ## The chain id is verified against the endpoint before anything is signed
///
/// EIP-155 binds a signature to a chain id, so signing for chain 1 and
/// broadcasting to Base produces a transaction the node rejects — but the
/// dangerous case is the reverse: a host whose endpoint config points at the
/// wrong network would otherwise sign a *valid* transaction for a chain the
/// user did not choose. `eth_chainId` is queried first and compared, so a
/// misrouted endpoint fails before a signature exists.
///
/// ## The nonce is read at `pending`, unlike a balance
///
/// A balance is read at `latest` because pending transactions can be dropped.
/// A nonce is the opposite: reading it at `latest` ignores transactions already
/// submitted but not yet mined, so two transfers in quick succession would be
/// assigned the same nonce and the second would replace the first.
///
/// # Errors
///
/// [`Error::Transport`] for a network failure or a node error,
/// [`Error::MalformedResponse`] for an unusable answer, [`Error::Address`] for
/// an invalid recipient, and [`Error::ChainIdMismatch`] if the endpoint serves
/// a different network than `network` names.
pub(super) async fn send(
    transport: &dyn Transport,
    network: EvmNetwork,
    from: &str,
    to: &str,
    value: u128,
    data: Vec<u8>,
    secret_key: &[u8],
) -> Result<String> {
    let id = network_id(Network::Evm(network));

    // Confirm the endpoint really serves this network before signing.
    let reported = hex_field(transport, id, "eth_chainId", json!([])).await?;
    let expected = u128::from(network.chain_id());
    if reported != expected {
        return Err(Error::ChainIdMismatch {
            network: id,
            expected: network.chain_id(),
            reported,
        });
    }

    // `pending`, so back-to-back transfers do not collide on a nonce.
    let nonce = hex_field(
        transport,
        id,
        "eth_getTransactionCount",
        json!([from, "pending"]),
    )
    .await?;
    let gas_price = hex_field(transport, id, "eth_gasPrice", json!([])).await?;

    let mut estimate = json!({ "from": from, "to": to, "value": format!("{value:#x}") });
    if !data.is_empty() {
        estimate["data"] = json!(format!("0x{}", hex_string(&data)));
    }
    let gas_limit = hex_field(transport, id, "eth_estimateGas", json!([estimate])).await?;

    let tx = LegacyTransaction {
        nonce,
        gas_price,
        gas_limit,
        to: Some(to.to_string()),
        value,
        data,
        chain_id: network.chain_id(),
    };
    let signed = tx.sign(secret_key).map_err(Error::Tx)?;

    let raw = transport
        .json_rpc(
            id,
            "eth_sendRawTransaction",
            json!([format!("0x{}", hex_string(&signed))]),
        )
        .await?;
    raw.as_str()
        .map(ToString::to_string)
        .ok_or_else(|| Error::MalformedResponse {
            network: id,
            operation: "eth_sendRawTransaction",
            detail: format!("expected a transaction hash, got {raw}"),
        })
}

/// Read a `0x`-prefixed hex quantity from a JSON-RPC method.
async fn hex_field(
    transport: &dyn Transport,
    id: crate::rpc::NetworkId,
    method: &'static str,
    params: serde_json::Value,
) -> Result<u128> {
    let raw = transport.json_rpc(id, method, params).await?;
    let hex = raw.as_str().ok_or_else(|| Error::MalformedResponse {
        network: id,
        operation: method,
        detail: format!("expected a hex string, got {raw}"),
    })?;
    parse_hex_u128(id, method, hex)
}

fn hex_string(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, b| {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
        out
    })
}
