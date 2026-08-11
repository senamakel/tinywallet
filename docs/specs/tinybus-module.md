# The TinyWallet TinyBus module

Status: Implemented

## Problem

A wallet's transaction encoders are heavy and a host mostly does something
else. Signing four chains costs `bitcoin` (and therefore a native `secp256k1`
C build), plus a full Ethereum stack for EIP-712 — measured at **51 crates** in
one embedding host, for a capability most sessions never invoke.

Gating them helps the builds that turn them off and does nothing for the build
that ships, which turns them on. What is needed is a boundary that survives
compilation: the capability present, the dependencies absent.

## Goals

- Run transaction building as a compiled artifact loaded at runtime.
- Keep the host's private keys **in the host**, always.
- Produce byte-identical transactions to the in-process library.
- Leave the published `tinywallet` crate bus-agnostic and free of tinybus.

## Non-goals

- Running untrusted code. A module is first-party code that ships separately.
- Key storage, key transport, or any form of remote signing.
- Unloading. tinybus never unloads a library.

## Interface

`ai.tinyhumans.tinywallet.Wallet` at `/ai/tinyhumans/tinywallet/Wallet`:

```text
BuildUnsigned(SigningRequest)   -> UnsignedTransaction
AttachSignature(AttachRequest)  -> SignedTransaction
```

Both argument and return types are `tinywallet::wire`, which is outside every
chain gate and depends on nothing but `serde` — so a host takes the crate with
`default-features = false`, shares one definition of the contract, and links no
chain library.

## The two-call split is the security property

The host holds the key. It asks what needs signing, signs locally, and hands
back only a signature. **No method accepts key material**, so there is nothing
in the module a leak could disclose.

`AttachSignature` re-sends the transaction fields rather than a handle, so the
module keeps no state between calls — no store, no bound on it, no expiry for
callers that never return. Rebuilding is safe because building is
deterministic: the same fields yield the transaction the digests were computed
over.

A loaded module shares the host's address space, so this is not a hard
isolation boundary and is not claimed as one. It is a refusal to widen what
crosses a boundary that already exists, which is worth doing on its own terms.

## Signing schemes

| Chain | Payload | Scheme |
| --- | --- | --- |
| Bitcoin | one BIP-143 sighash **per selected input**, in input order | secp256k1 prehash |
| EVM | keccak of the EIP-155 signing payload | secp256k1 prehash |
| Tron | `sha256(raw_data)`, which is also the `txID` | secp256k1 prehash |
| Solana | the **whole serialized message** | ed25519 |

Two rules a host must not get wrong, both named in the wire types:

- A `secp256k1_prehash` payload is **already hashed**. Sign it with a prehash
  entry point; hashing again produces a valid signature over the wrong thing.
- An `ed25519` payload is **not** hashed — ed25519 hashes internally.

Signatures must be low-`s` normalized. Bitcoin enforces it as relay policy
(BIP-146) and Ethereum as consensus (EIP-2), so a high-`s` signature yields a
transaction that is rejected rather than merely unusual. `k256` and `secp256k1`
both normalize by default; the Bitcoin path normalizes again on reassembly.

## Everything travels inline

Unlike the `tinydocs` module, there are no streams, no chunking and no held
outputs. A tinybus frame is JSON capped at 16 MiB where a byte array costs
about 3.5 bytes per byte — a real constraint for a generated document, and
irrelevant here. The largest payload is a Bitcoin spend's UTXO list; a wallet
with a thousand of them is still tens of kilobytes.

## Errors

| Wire name | Meaning |
| --- | --- |
| `…Error.InvalidInput` | The request was wrong. A caller can fix it. |
| `…Error.BuildFailed` | Building or assembling failed. A caller cannot. |
| `…Error.UnsupportedChain` | The chain is not in this build. |

An unrecognised name must be treated as `BuildFailed`. Telling a model its
input was wrong when it was not sends it into a rewrite loop over something
already correct.

## Verification

`crates/tinywallet-module/tests/module_e2e.rs` loads the built `cdylib` through
the real loader and broker and asserts that a transaction signed through the
module equals the library's own output byte-for-byte, on EVM, a multi-input
Bitcoin spend, and Solana. That equivalence is the claim the module rests on.

The test is `#[ignore]`d and needs `TINYWALLET_TEST_MODULE` pointing at the
artifact, because tinybus never unloads a module and a second load of the same
artifact would collide on the well-known name.

## Open questions

**A reply-stream seam in tinybus** would matter if a future method ever
returned something large. Nothing here does.

**Per-interface method lists in `module_export!`** would let one module serve
several fully-declared interfaces; today the macro attaches its method list to
the first entry in `provides`.
