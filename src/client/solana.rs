//! Solana balance query: `getBalance`.

use serde::Deserialize;
use serde_json::json;

use super::{Result, decode, network_id};
use crate::asset::{Network, SolanaCluster};
use crate::rpc::Transport;

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
