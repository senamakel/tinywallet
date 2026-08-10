//! Transaction status lookup, normalised across four very different APIs.

use serde_json::json;

use super::{Error, Result, network_id};
use crate::asset::Network;
use crate::rpc::Transport;

/// Where a transaction has got to.
///
/// Deliberately coarse. Each chain reports progress in its own vocabulary —
/// Solana has commitment levels, Bitcoin has confirmation depth, EVM has a
/// receipt status byte — and a caller almost always wants the same three
/// answers, so the detail is flattened rather than leaked as a union of four
/// chains' concepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxState {
    /// Accepted by the network but not yet in a block.
    ///
    /// Also the answer for a transaction the network has never seen, because
    /// **no chain here distinguishes the two**. A node cannot tell "not
    /// broadcast" from "broadcast and not yet mined" — both are simply absent
    /// — so reporting anything more definite would be a guess. A caller that
    /// needs certainty must track what it broadcast itself.
    Pending,
    /// Included in a block and executed successfully.
    Confirmed,
    /// Included in a block and reverted.
    ///
    /// Distinct from `Pending` because it is terminal: the fee was spent and
    /// retrying the identical transaction will fail identically.
    Failed,
}

/// A transaction's progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxStatus {
    /// Coarse state.
    pub state: TxState,
    /// Confirmation depth, when the chain reports one.
    pub confirmations: Option<u64>,
    /// Block height or slot, once included.
    pub block: Option<u64>,
}

impl TxStatus {
    const PENDING: Self = Self {
        state: TxState::Pending,
        confirmations: None,
        block: None,
    };
}

/// Look up `tx_hash` on `network`.
///
/// A transaction the network has not seen reports [`TxState::Pending`] rather
/// than an error — see [`TxState::Pending`] for why absence is indistinguishable
/// from in-flight.
///
/// # Errors
///
/// [`Error::Transport`] on a network failure, [`Error::MalformedResponse`] if
/// the answer cannot be read.
pub async fn status(
    transport: &dyn Transport,
    network: Network,
    tx_hash: &str,
) -> Result<TxStatus> {
    let id = network_id(network);
    let hash = tx_hash.trim();

    match network {
        Network::Evm(_) => {
            let receipt = transport
                .json_rpc(id, "eth_getTransactionReceipt", json!([hash]))
                .await?;
            if receipt.is_null() {
                return Ok(TxStatus::PENDING);
            }
            // The receipt's `status` is 0x1 for success and 0x0 for a revert.
            // A reverted transaction still has a receipt and still spent its
            // fee, so this is not an error path.
            let state = match receipt.get("status").and_then(serde_json::Value::as_str) {
                Some("0x0") => TxState::Failed,
                _ => TxState::Confirmed,
            };
            let block = receipt
                .get("blockNumber")
                .and_then(serde_json::Value::as_str)
                .and_then(|b| u64::from_str_radix(b.trim_start_matches("0x"), 16).ok());
            Ok(TxStatus {
                state,
                confirmations: None,
                block,
            })
        }

        Network::Solana(_) => {
            let raw = transport
                .json_rpc(id, "getSignatureStatuses", json!([[hash]]))
                .await?;
            let entry = raw.get("value").and_then(|v| v.get(0));
            let Some(entry) = entry.filter(|e| !e.is_null()) else {
                return Ok(TxStatus::PENDING);
            };
            // `err` is null on success and an object describing the failure
            // otherwise — presence, not shape, is what matters here.
            let state = if entry.get("err").is_some_and(|e| !e.is_null()) {
                TxState::Failed
            } else {
                TxState::Confirmed
            };
            Ok(TxStatus {
                state,
                confirmations: entry.get("confirmations").and_then(serde_json::Value::as_u64),
                block: entry.get("slot").and_then(serde_json::Value::as_u64),
            })
        }

        Network::Btc => {
            let body = transport.rest_get(id, &format!("tx/{hash}/status")).await?;
            let parsed: serde_json::Value =
                serde_json::from_str(&body).map_err(|e| Error::MalformedResponse {
                    network: id,
                    operation: "tx/status",
                    detail: format!("not JSON: {e}"),
                })?;
            // Bitcoin has no failed state: a transaction is either in a block
            // or it is not. There is nothing to map onto TxState::Failed.
            if parsed.get("confirmed").and_then(serde_json::Value::as_bool) == Some(true) {
                Ok(TxStatus {
                    state: TxState::Confirmed,
                    confirmations: None,
                    block: parsed
                        .get("block_height")
                        .and_then(serde_json::Value::as_u64),
                })
            } else {
                Ok(TxStatus::PENDING)
            }
        }

        Network::Tron => {
            let body = transport
                .rest_post(
                    id,
                    "wallet/gettransactioninfobyid",
                    json!({ "value": hash }).to_string(),
                    "application/json",
                )
                .await?;
            let parsed: serde_json::Value =
                serde_json::from_str(&body).map_err(|e| Error::MalformedResponse {
                    network: id,
                    operation: "wallet/gettransactioninfobyid",
                    detail: format!("not JSON: {e}"),
                })?;
            // An unknown transaction comes back as `{}`, not as an error.
            let Some(block) = parsed.get("blockNumber").and_then(serde_json::Value::as_u64) else {
                return Ok(TxStatus::PENDING);
            };
            let state = match parsed.get("receipt").and_then(|r| r.get("result")) {
                Some(result) if result.as_str() == Some("SUCCESS") => TxState::Confirmed,
                // A missing `result` on a mined native transfer means success:
                // TronGrid only populates it for contract calls.
                None => TxState::Confirmed,
                _ => TxState::Failed,
            };
            Ok(TxStatus {
                state,
                confirmations: None,
                block: Some(block),
            })
        }
    }
}
