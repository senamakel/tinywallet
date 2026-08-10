//! Bitcoin balance query against an Esplora REST API.

use serde::Deserialize;

use super::{Error, Result, network_id};
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
