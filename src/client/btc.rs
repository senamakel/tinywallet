//! Bitcoin balance query against an Esplora REST API.

use serde::Deserialize;

use super::{Error, Result, network_id};
use crate::tx::btc::{Transfer, Utxo};
use crate::asset::Network;
use crate::rpc::Transport;

/// Esplora reports funded and spent totals separately, for confirmed and
/// mempool activity. A balance is the difference, summed across both.
#[derive(Deserialize)]
struct AddressInfo {
    chain_stats: Stats,
    mempool_stats: Stats,
}

#[derive(Deserialize)]
struct Stats {
    funded_txo_sum: u64,
    spent_txo_sum: u64,
}

impl Stats {
    /// Saturating because a spent total should never exceed a funded one; if a
    /// malformed or hostile response says otherwise, clamping to zero is far
    /// better than underflowing to a balance near `u64::MAX`.
    const fn net(&self) -> u64 {
        self.funded_txo_sum.saturating_sub(self.spent_txo_sum)
    }
}

/// Native balance in satoshis, including unconfirmed mempool activity.
///
/// Mempool is included because that is what a block explorer shows as
/// spendable and what a user expects to see immediately after receiving.
pub(super) async fn balance(transport: &dyn Transport, address: &str) -> Result<u128> {
    let id = network_id(Network::Btc);
    let body = transport
        .rest_get(id, &format!("address/{address}"))
        .await?;
    let info: AddressInfo = serde_json::from_str(&body).map_err(|e| Error::MalformedResponse {
        network: id,
        operation: "address",
        detail: format!("not an Esplora address response: {e}"),
    })?;
    Ok(u128::from(info.chain_stats.net()) + u128::from(info.mempool_stats.net()))
}

/// List the unspent outputs an address controls.
///
/// # Errors
///
/// [`Error::Transport`] on a network failure, [`Error::MalformedResponse`] if
/// the body is not an Esplora UTXO list.
pub(super) async fn utxos(transport: &dyn Transport, address: &str) -> Result<Vec<Utxo>> {
    #[derive(Deserialize)]
    struct EsploraUtxo {
        txid: String,
        vout: u32,
        value: u64,
    }

    let id = network_id(Network::Btc);
    let body = transport
        .rest_get(id, &format!("address/{address}/utxo"))
        .await?;
    let listed: Vec<EsploraUtxo> =
        serde_json::from_str(&body).map_err(|e| Error::MalformedResponse {
            network: id,
            operation: "address/utxo",
            detail: format!("not an Esplora UTXO list: {e}"),
        })?;
    Ok(listed
        .into_iter()
        .map(|u| Utxo {
            txid: u.txid,
            vout: u.vout,
            value: u.value,
        })
        .collect())
}

/// Fetch UTXOs, build, sign and broadcast a transfer. Returns the txid.
///
/// The UTXO set is fetched immediately before building, because coin selection
/// is only valid against the set it was computed from: spending an output that
/// another transaction already consumed produces a transaction the network
/// rejects as a double-spend.
///
/// # Errors
///
/// See [`Error`].
pub(super) async fn send(
    transport: &dyn Transport,
    from: &str,
    to: &str,
    amount: u64,
    fee: u64,
    secret_key: &[u8],
) -> Result<String> {
    let id = network_id(Network::Btc);
    let available = utxos(transport, from).await?;

    let transfer = Transfer {
        from: from.to_string(),
        to: to.to_string(),
        amount,
        fee,
    };
    let raw_hex = transfer.sign(&available, secret_key).map_err(Error::Tx)?;

    transport
        .rest_post(id, "tx", raw_hex, "text/plain")
        .await
        .map(|txid| txid.trim().to_string())
        .map_err(Error::Transport)
}
