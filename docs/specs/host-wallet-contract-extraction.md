# Host-owned wallet contract data and intent matching

## Purpose and boundary

The `tinywallet-bus` crate owns reusable, pure contract facts shared by TinyWallet
and its hosts. It may expose the wallet's typed storage requirements, map a
chain selection to an RPC endpoint key, and compare a prepared quote with the
transaction intent supplied for execution.

The bus remains data-only: it does not open storage, resolve an endpoint URL,
send an RPC request, execute a transaction, retain an approval permit, or make
an authorization decision. OpenHuman keeps permit ownership, approval policy,
execution and provider callbacks. The wallet implementation keeps ledger
persistence and quote construction. This extraction moves only immutable
declarations and deterministic comparisons; it does not change their
behavior.

## Storage requirements

The bus publishes the x402 ledger's host-facing `StorageNeeds` declaration
using the canonical TinyStorageDrivers bus type. Its requirements are derived
from the ledger implementation's actual stores:

- a document collection named `x402_budget`;
- a stream prefix named `x402.payments`.

The declaration adds no indexes, retention policy, time-to-live, or fabricated
resource identifiers. It describes the minimum storage capability needed by
the ledger; it does not create or access those resources. The x402 ledger uses
the shared declaration so its resource names cannot drift from the host
contract. Existing implementation constant names remain available for the
compatibility window and resolve to the same values.

## Endpoint key mapping

`chain::endpoint_key` is a pure mapping from the shared `Chain` enum and an
optional EVM chain ID to `RpcEndpointKey`:

- Bitcoin, Solana, and Tron map to their matching endpoint keys, regardless of
  the optional EVM chain ID.
- EVM maps only when the chain ID matches a member of
  `EvmNetwork::ALL`; an absent or unknown ID returns `None`.
- Any future or unsupported `Chain` variant returns `None`.

The function returns a key only. Endpoint URLs, overrides, credentials,
transport and failure handling stay in the host.

## Native intent matching

`intent::native_intent_matches_quote` compares a prepared transaction with a
native-wallet execution intent, without consuming permission or invoking the
wallet. It preserves the existing host comparison rules:

- Bitcoin and Solana transfers require the matching quote chain and
  `NativeTransfer` kind, exact sender and recipient strings, and an amount that
  parses as `u64` and equals the transaction amount.
- An EVM intent requires the quote's EVM chain ID. A missing quote network uses
  Ethereum mainnet, matching existing behavior. Native transfers require the
  exact destination and decimal value and empty data. Token transfers require
  a present token contract, case-insensitive contract equality, zero native
  value, and ERC-20 `transfer(address,uint256)` calldata whose selector,
  recipient and 256-bit decimal amount match. Invalid addresses, non-decimal or
  overflowing amounts, and any other calldata return `false`.
- A Tron native transfer requires the matching chain, exact recipient and
  equal parsed sun amount. A TRC-20 transfer requires the matching chain,
  exact recipient and non-empty, even-length hexadecimal parameter bytes.
- A Solana message intent matches only a Solana token-transfer quote and only
  when its message is non-empty, even-length hexadecimal data.
- Unsupported transaction kinds and mismatched quote kinds return `false`.

## Web3 intent matching

`intent::web3_approval_matches_request` checks the request against a previously
constructed quote. It is a comparison only; OpenHuman continues to consume
the quote-bound permit and decide whether execution is authorized.

- Quote ID must match exactly, and operation must correspond exactly to quote
  kind: `swap`, `bridge`, or `dapp_call`.
- EVM execution requires an EVM signing chain, a known network name and chain
  ID pair, and agreement with a quote `network` string when it is a string.
  Missing, null, or non-string quote network values are treated as absent.
- Dapp calls compare quote `contractAddress` case-insensitively, `calldata`
  byte-for-byte, and `valueRaw` byte-for-byte against the request.
- Swaps and bridges compare `tx.to` case-insensitively, `tx.data` exactly, and
  `tx.value` when represented as either a JSON string or number. For `tx.data`,
  absent or non-string quote data matches only an absent request data value;
  other JSON types do not match a string request value. Other value JSON types
  never match.
- Solana execution matches only swap or bridge quotes and requires an exact
  match with `tx.data`.
- Missing required quote fields, wrong signing chain, quote kind, operation,
  network, recipient, calldata or value return `false`; optional network and
  swap/bridge data follow the absence rules above. Nonce and fee fields are
  intentionally outside this quote-intent comparison, as in the existing host
  implementation.

## Compatibility and regression coverage

The extracted functions preserve the host's existing comparison matrix. Tests
cover successful matches and rejected changes to chain, quote kind, operation,
recipient, amount, token contract, calldata selector and payload, missing or
wrongly typed quote fields, malformed hexadecimal, invalid decimal values,
and decimal overflow. Storage declaration tests compare the exported
requirements with the names consumed by the ledger stores. Approval-permit
consumption and authorization tests remain with the host that owns those
behaviors.
