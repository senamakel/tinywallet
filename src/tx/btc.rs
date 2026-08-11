//! Bitcoin P2WPKH transaction building and BIP-143 signing.
//!
//! Native segwit only — the same restriction
//! `address::btc::validate_sender` enforces, and for the same reason: this is
//! the one script type the signing path below implements.
//!
//! ## Bitcoin is the one chain where the fee is implicit
//!
//! On every other chain the fee is a field. Here it is
//! `sum(inputs) - sum(outputs)`, which means **a forgotten change output is
//! not a rounding error — it is the entire remaining balance paid to miners.**
//! [`Transfer::build`] therefore computes change explicitly and returns
//! [`Error::InvalidField`] rather than ever silently letting a surplus fall
//! through into the fee.
//!
//! ## Dust
//!
//! A change output below the dust threshold cannot be economically spent, so
//! nodes reject the transaction outright. Change under [`DUST_THRESHOLD`] is
//! dropped into the fee instead — the one case where an implicit fee increase
//! is correct, because the alternative is an unrelayable transaction.
//!
//! ## BIP-143 signs the input's value, and that is the security property
//!
//! Legacy sighash did not commit to the amount being spent, which let a
//! hardware wallet be lied to about input values and tricked into paying an
//! enormous fee. BIP-143 fixes it by including each input's value in its
//! sighash — which is why [`Utxo::value`] must be the *real* on-chain value.
//! A wrong one produces an invalid signature rather than a wrong transfer, so
//! this fails safe, but it fails.

use std::str::FromStr;

use bitcoin::absolute::LockTime;
use bitcoin::hashes::Hash;
use bitcoin::key::{CompressedPublicKey, PrivateKey};
use bitcoin::secp256k1::{Message, Secp256k1};
use bitcoin::sighash::{EcdsaSighashType, SighashCache};
use bitcoin::transaction::Version;
use bitcoin::{
    Address, Amount, Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
};

use super::{Error, Result};

/// Outputs below this many satoshis are unspendable dust and get folded into
/// the fee. 546 is the standard relay threshold for a P2WPKH output.
pub const DUST_THRESHOLD: u64 = 546;

/// An unspent output available to fund a transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Utxo {
    /// Transaction id holding the output.
    pub txid: String,
    /// Index of the output within that transaction.
    pub vout: u32,
    /// Value in satoshis.
    ///
    /// Must be the real on-chain value: BIP-143 signs it, so a wrong value
    /// yields a signature that will not verify. See the module docs.
    pub value: u64,
}

/// A P2WPKH transfer, before coin selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transfer {
    /// Sender's `bc1q…` address. Must be P2WPKH.
    pub from: String,
    /// Recipient's address. Any mainnet type is fine.
    pub to: String,
    /// Amount to send, in satoshis.
    pub amount: u64,
    /// Absolute fee to pay, in satoshis.
    pub fee: u64,
}

/// The coins chosen to fund a transfer, and the change they leave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// The UTXOs to spend.
    pub inputs: Vec<Utxo>,
    /// Change returning to the sender, in satoshis. Zero when the surplus was
    /// below [`DUST_THRESHOLD`] and folded into the fee.
    pub change: u64,
}

/// Choose UTXOs to cover `target`, largest first.
///
/// Largest-first keeps the input count — and so the transaction size and the
/// fee — small. It is not privacy-optimal, but a wallet that consolidates
/// predictably is easier to reason about than one that does not.
///
/// # Errors
///
/// [`Error::InsufficientFunds`] when the available total cannot cover
/// `target`.
pub fn select_coins(utxos: &[Utxo], target: u64) -> Result<Selection> {
    let mut sorted = utxos.to_vec();
    sorted.sort_by_key(|u| std::cmp::Reverse(u.value));

    let mut total: u64 = 0;
    let mut chosen = Vec::new();
    for utxo in sorted {
        total = total
            .checked_add(utxo.value)
            .ok_or_else(|| Error::InvalidField {
                field: "utxos",
                reason: "total value overflows".to_string(),
            })?;
        chosen.push(utxo);
        if total >= target {
            let surplus = total - target;
            // Dust change would make the transaction unrelayable, so it goes
            // to the fee instead. The only correct implicit fee increase here.
            let change = if surplus > DUST_THRESHOLD { surplus } else { 0 };
            return Ok(Selection {
                inputs: chosen,
                change,
            });
        }
    }
    Err(Error::InsufficientFunds {
        available: total,
        required: target,
    })
}

impl Transfer {
    /// Select coins and build the unsigned transaction.
    ///
    /// # Errors
    ///
    /// [`Error::Address`] for an invalid or non-P2WPKH sender,
    /// [`Error::InsufficientFunds`] if the UTXOs cannot cover amount plus fee,
    /// [`Error::InvalidField`] for a malformed txid or an arithmetic overflow.
    pub fn build(&self, utxos: &[Utxo]) -> Result<(Transaction, Selection)> {
        // Sender must be P2WPKH — the only type signed below.
        let from = crate::address::btc::validate_sender(&self.from).map_err(Error::Address)?;
        let to = crate::address::btc::validate(&self.to).map_err(Error::Address)?;

        let target = self
            .amount
            .checked_add(self.fee)
            .ok_or_else(|| Error::InvalidField {
                field: "amount",
                reason: "amount + fee overflows".to_string(),
            })?;
        let selection = select_coins(utxos, target)?;

        let from_spk = script_pubkey(&from)?;
        let to_spk = script_pubkey(&to)?;

        let mut input = Vec::with_capacity(selection.inputs.len());
        for utxo in &selection.inputs {
            let txid = bitcoin::Txid::from_str(&utxo.txid).map_err(|e| Error::InvalidField {
                field: "utxo.txid",
                reason: format!("{}: {e}", utxo.txid),
            })?;
            input.push(TxIn {
                previous_output: OutPoint {
                    txid,
                    vout: utxo.vout,
                },
                script_sig: ScriptBuf::new(),
                // Opt into RBF so a transfer stuck at a low fee can be bumped
                // rather than sitting in the mempool until it expires.
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::default(),
            });
        }

        let mut output = vec![TxOut {
            value: Amount::from_sat(self.amount),
            script_pubkey: to_spk,
        }];
        if selection.change > 0 {
            output.push(TxOut {
                value: Amount::from_sat(selection.change),
                script_pubkey: from_spk,
            });
        }

        Ok((
            Transaction {
                version: Version::TWO,
                lock_time: LockTime::ZERO,
                input,
                output,
            },
            selection,
        ))
    }

    /// Build and sign, returning the raw transaction hex for broadcast.
    ///
    /// # Errors
    ///
    /// As [`Transfer::build`], plus [`Error::Signing`] if the key is invalid
    /// or does not control `from`.
    pub fn sign(&self, utxos: &[Utxo], secret_key: &[u8]) -> Result<String> {
        let secret =
            bitcoin::secp256k1::SecretKey::from_slice(secret_key).map_err(|_| Error::Signing {
                reason: "not a valid secp256k1 secret key".to_string(),
            })?;
        let secp = Secp256k1::new();
        let private = PrivateKey::new(secret, Network::Bitcoin);
        let public =
            CompressedPublicKey::from_private_key(&secp, &private).map_err(|_| Error::Signing {
                reason: "could not derive the public key".to_string(),
            })?;

        // Catch a key/address mismatch here rather than after broadcasting an
        // unspendable transaction.
        let derived = Address::p2wpkh(&public, Network::Bitcoin).to_string();
        if derived != self.from.trim() {
            return Err(Error::Signing {
                reason: "secret key does not control the `from` address".to_string(),
            });
        }

        let compressed = public.to_bytes();
        let signatures = self
            .sighashes(utxos, &compressed)?
            .1
            .into_iter()
            .map(|sighash| {
                let message = Message::from_digest(sighash);
                secp.sign_ecdsa(&message, &private.inner)
                    .serialize_compact()
            })
            .collect::<Vec<_>>();

        // Routed through the same reassembly the split-signing path uses, so
        // witness layout and input ordering have exactly one implementation.
        self.attach_signatures(utxos, &compressed, &signatures)
    }

    /// The per-input digests to sign, without needing the key.
    ///
    /// Returns the coin selection alongside them because the caller needs to
    /// know how many signatures to produce and in which order: **one per
    /// selected input, in input order**. Bitcoin is the only chain here that
    /// needs more than one signature for a single transaction.
    ///
    /// Each digest is a BIP-143 P2WPKH sighash, already hashed — sign it with a
    /// "prehash" entry point.
    ///
    /// # Errors
    ///
    /// As [`Transfer::build`], plus [`Error::Signing`] if `public_key` does not
    /// control `from`.
    pub fn sighashes(
        &self,
        utxos: &[Utxo],
        public_key: &[u8; 33],
    ) -> Result<(Selection, Vec<[u8; 32]>)> {
        self.check_controls_from(public_key)?;

        let (mut tx, selection) = self.build(utxos)?;
        let from_spk = script_pubkey(
            &crate::address::btc::validate_sender(&self.from).map_err(Error::Address)?,
        )?;

        let mut cache = SighashCache::new(&mut tx);
        let mut digests = Vec::with_capacity(selection.inputs.len());
        for (index, utxo) in selection.inputs.iter().enumerate() {
            // BIP-143 commits to this input's value — see the module docs.
            let sighash = cache
                .p2wpkh_signature_hash(
                    index,
                    &from_spk,
                    Amount::from_sat(utxo.value),
                    EcdsaSighashType::All,
                )
                .map_err(|e| Error::Signing {
                    reason: format!("sighash failed: {e}"),
                })?;
            digests.push(sighash.to_byte_array());
        }
        Ok((selection, digests))
    }

    /// Assemble the raw transaction from signatures over [`Self::sighashes`].
    ///
    /// `signatures` must hold one 64-byte compact signature per selected input,
    /// in the same order [`Self::sighashes`] returned the digests.
    ///
    /// # Errors
    ///
    /// As [`Transfer::build`], plus [`Error::Signing`] if `public_key` does not
    /// control `from`, if the signature count does not match the input count,
    /// or if a signature is not a valid secp256k1 `(r, s)` pair.
    pub fn attach_signatures(
        &self,
        utxos: &[Utxo],
        public_key: &[u8; 33],
        signatures: &[[u8; 64]],
    ) -> Result<String> {
        self.check_controls_from(public_key)?;

        let (mut tx, selection) = self.build(utxos)?;
        if signatures.len() != selection.inputs.len() {
            return Err(Error::Signing {
                reason: format!(
                    "expected {} signatures for {} inputs, got {}",
                    selection.inputs.len(),
                    selection.inputs.len(),
                    signatures.len()
                ),
            });
        }

        for (input, compact) in tx.input.iter_mut().zip(signatures) {
            let mut signature = bitcoin::secp256k1::ecdsa::Signature::from_compact(compact)
                .map_err(|_| Error::Signing {
                    reason: "signature is not a valid secp256k1 (r, s) pair".to_string(),
                })?;
            // Bitcoin enforces low-`s` as a relay policy rule (BIP-146), so a
            // high-`s` signature yields a transaction nodes refuse to relay.
            // Normalizing here means a caller that signed with a library which
            // does not normalize still produces a broadcastable transaction,
            // and one that does is unaffected — the operation is idempotent.
            signature.normalize_s();

            let mut witness = Witness::new();
            let mut der = signature.serialize_der().to_vec();
            der.push(EcdsaSighashType::All as u8);
            witness.push(der);
            witness.push(public_key);
            input.witness = witness;
        }

        Ok(bitcoin::consensus::encode::serialize_hex(&tx))
    }

    /// Refuse early if `public_key` does not control `from`.
    ///
    /// Caught here rather than after broadcasting an unspendable transaction —
    /// which is unrecoverable, because the fee is paid either way.
    fn check_controls_from(&self, public_key: &[u8; 33]) -> Result<()> {
        let public = CompressedPublicKey::from_slice(public_key).map_err(|_| Error::Signing {
            reason: "not a valid compressed secp256k1 public key".to_string(),
        })?;
        let derived = Address::p2wpkh(&public, Network::Bitcoin).to_string();
        if derived != self.from.trim() {
            return Err(Error::Signing {
                reason: "public key does not control the `from` address".to_string(),
            });
        }
        Ok(())
    }
}

/// The scriptPubKey that pays to `address`.
fn script_pubkey(address: &str) -> Result<ScriptBuf> {
    Ok(Address::from_str(address)
        .map_err(|e| Error::InvalidField {
            field: "address",
            reason: e.to_string(),
        })?
        .assume_checked()
        .script_pubkey())
}

#[cfg(test)]
mod test {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{DUST_THRESHOLD, Transfer, Utxo, select_coins};
    use crate::tx::Error;

    const VECTOR: &str = "abandon abandon abandon abandon abandon abandon \
                          abandon abandon abandon abandon abandon about";
    const PATH: &str = "m/84'/0'/0'/0/0";
    /// The BIP-84 vector address, which the key below controls.
    const FROM: &str = "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu";
    const TO: &str = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";
    const TXID: &str = "7f3b662ea8b6ff2e0e1a1f9bd0f1c39a6b8ba51e1b0f0e0d0c0b0a0908070605";

    fn key() -> Vec<u8> {
        crate::key::derive(crate::Chain::Btc, VECTOR, PATH)
            .unwrap()
            .secret_bytes()
            .to_vec()
    }

    fn utxo(value: u64, vout: u32) -> Utxo {
        Utxo {
            txid: TXID.to_string(),
            vout,
            value,
        }
    }

    fn transfer(amount: u64, fee: u64) -> Transfer {
        Transfer {
            from: FROM.to_string(),
            to: TO.to_string(),
            amount,
            fee,
        }
    }

    #[test]
    fn selection_takes_the_largest_coins_first() {
        let utxos = [utxo(1_000, 0), utxo(50_000, 1), utxo(10_000, 2)];
        let selection = select_coins(&utxos, 40_000).unwrap();
        assert_eq!(selection.inputs.len(), 1, "one big coin suffices");
        assert_eq!(selection.inputs[0].value, 50_000);
        assert_eq!(selection.change, 10_000);
    }

    #[test]
    fn selection_accumulates_until_the_target_is_met() {
        let utxos = [utxo(10_000, 0), utxo(10_000, 1), utxo(10_000, 2)];
        let selection = select_coins(&utxos, 25_000).unwrap();
        assert_eq!(selection.inputs.len(), 3);
        assert_eq!(selection.change, 5_000);
    }

    #[test]
    fn dust_change_is_folded_into_the_fee_not_emitted() {
        // A sub-dust output is unspendable and makes the transaction
        // unrelayable, so it must not be created.
        let utxos = [utxo(10_000 + DUST_THRESHOLD, 0)];
        let selection = select_coins(&utxos, 10_000).unwrap();
        assert_eq!(selection.change, 0, "dust surplus goes to the fee");

        let (tx, _) = transfer(9_000, 1_000 + DUST_THRESHOLD)
            .build(&utxos)
            .unwrap();
        assert_eq!(tx.output.len(), 1, "no dust change output");
    }

    #[test]
    fn change_above_the_dust_threshold_is_emitted() {
        let utxos = [utxo(100_000, 0)];
        let (tx, selection) = transfer(50_000, 1_000).build(&utxos).unwrap();
        assert_eq!(selection.change, 49_000);
        assert_eq!(tx.output.len(), 2, "recipient plus change");
        assert_eq!(tx.output[1].value.to_sat(), 49_000);
    }

    #[test]
    fn insufficient_funds_reports_both_sides() {
        // The one failure a caller can act on, so it names the numbers.
        match select_coins(&[utxo(1_000, 0)], 5_000).unwrap_err() {
            Error::InsufficientFunds {
                available,
                required,
            } => {
                assert_eq!(available, 1_000);
                assert_eq!(required, 5_000);
            }
            other => panic!("expected InsufficientFunds, got {other:?}"),
        }
        assert!(matches!(
            select_coins(&[], 1).unwrap_err(),
            Error::InsufficientFunds { available: 0, .. }
        ));
    }

    #[test]
    fn the_fee_is_exactly_inputs_minus_outputs() {
        // Bitcoin's fee is implicit, so this is the invariant that stops a
        // forgotten change output paying the balance to miners.
        let utxos = [utxo(100_000, 0)];
        let (tx, _) = transfer(30_000, 2_000).build(&utxos).unwrap();
        let out: u64 = tx.output.iter().map(|o| o.value.to_sat()).sum();
        assert_eq!(
            100_000 - out,
            2_000,
            "implicit fee must equal the stated fee"
        );
    }

    #[test]
    fn every_input_is_signed_with_a_witness() {
        let utxos = [utxo(60_000, 0), utxo(60_000, 1)];
        let hex = transfer(100_000, 1_000).sign(&utxos, &key()).unwrap();
        assert!(!hex.is_empty());
        // Segwit marker and flag follow the 4-byte version in the serialised
        // form: 02000000 then 0001.
        assert!(hex.starts_with("020000000001"), "{hex}");
    }

    #[test]
    fn the_transaction_opts_into_replace_by_fee() {
        // A transfer stuck at a low fee should be bumpable rather than left
        // to sit in the mempool.
        let (tx, _) = transfer(10_000, 500).build(&[utxo(50_000, 0)]).unwrap();
        assert!(tx.input[0].sequence.is_rbf());
    }

    #[test]
    fn a_key_that_does_not_control_the_sender_is_rejected() {
        let other = crate::key::derive(crate::Chain::Btc, VECTOR, "m/84'/0'/0'/0/1")
            .unwrap()
            .secret_bytes()
            .to_vec();
        match transfer(10_000, 500)
            .sign(&[utxo(50_000, 0)], &other)
            .unwrap_err()
        {
            Error::Signing { reason } => assert!(reason.contains("does not control")),
            other => panic!("expected Signing, got {other:?}"),
        }
    }

    #[test]
    fn a_non_p2wpkh_sender_is_rejected() {
        // The signing path only implements P2WPKH; a legacy sender would fail
        // much later, after a transaction had been assembled.
        let legacy = Transfer {
            from: "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2".to_string(),
            ..transfer(1_000, 100)
        };
        assert!(matches!(
            legacy.build(&[utxo(50_000, 0)]),
            Err(Error::Address(_))
        ));
    }

    #[test]
    fn any_address_type_is_accepted_as_a_recipient() {
        // Paying to P2PKH, P2SH or P2TR is the same operation.
        for to in [
            "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2",
            "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy",
            "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr",
        ] {
            let t = Transfer {
                to: to.to_string(),
                ..transfer(10_000, 500)
            };
            assert!(
                t.build(&[utxo(50_000, 0)]).is_ok(),
                "{to} should be payable"
            );
        }
    }

    #[test]
    fn a_malformed_txid_is_rejected() {
        let bad = Utxo {
            txid: "not-a-txid".to_string(),
            vout: 0,
            value: 50_000,
        };
        match transfer(1_000, 100).build(&[bad]).unwrap_err() {
            Error::InvalidField { field, .. } => assert_eq!(field, "utxo.txid"),
            other => panic!("expected InvalidField, got {other:?}"),
        }
    }

    #[test]
    fn signing_is_deterministic() {
        let utxos = [utxo(50_000, 0)];
        let t = transfer(10_000, 500);
        assert_eq!(
            t.sign(&utxos, &key()).unwrap(),
            t.sign(&utxos, &key()).unwrap()
        );
    }

    #[test]
    fn changing_an_input_value_changes_the_signature() {
        // BIP-143 commits to each input's value, which is what stopped the
        // fee-inflation attack legacy sighash allowed.
        let t = transfer(10_000, 500);
        let a = t.sign(&[utxo(50_000, 0)], &key()).unwrap();
        let b = t.sign(&[utxo(60_000, 0)], &key()).unwrap();
        assert_ne!(a, b, "the input value must reach the sighash");
    }

    /// The compressed public key for the test mnemonic's P2WPKH account.
    fn public_key() -> [u8; 33] {
        use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
        let secret = SecretKey::from_slice(&key()).unwrap();
        PublicKey::from_secret_key(&Secp256k1::new(), &secret).serialize()
    }

    #[test]
    fn split_signing_matches_one_shot_signing_across_several_inputs() {
        // Several inputs on purpose: Bitcoin is the only chain here needing
        // more than one signature, and the split contract is that they come
        // back in input order. A transposition would still produce a
        // well-formed transaction — just an unspendable one — so the two
        // paths are compared byte-for-byte.
        use bitcoin::secp256k1::{Message, Secp256k1, SecretKey};

        let utxos = [utxo(60_000, 0), utxo(70_000, 1), utxo(80_000, 2)];
        let transfer = transfer(150_000, 2_000);
        let public = public_key();

        let one_shot = transfer.sign(&utxos, &key()).unwrap();

        let (selection, digests) = transfer.sighashes(&utxos, &public).unwrap();
        assert!(
            digests.len() > 1,
            "the fixture must actually select several inputs"
        );
        assert_eq!(digests.len(), selection.inputs.len());

        let secret = SecretKey::from_slice(&key()).unwrap();
        let secp = Secp256k1::signing_only();
        let signatures: Vec<[u8; 64]> = digests
            .into_iter()
            .map(|digest| {
                secp.sign_ecdsa(&Message::from_digest(digest), &secret)
                    .serialize_compact()
            })
            .collect();

        let split = transfer
            .attach_signatures(&utxos, &public, &signatures)
            .unwrap();

        assert_eq!(split, one_shot);
    }

    #[test]
    fn a_public_key_that_does_not_control_the_sender_is_refused() {
        // Both halves must refuse, not just the first: a host could call
        // `attach_signatures` without ever calling `sighashes`.
        let utxos = [utxo(100_000, 0)];
        let transfer = transfer(50_000, 1_000);
        let wrong = [0x02u8; 33];

        assert!(matches!(
            transfer.sighashes(&utxos, &wrong),
            Err(Error::Signing { .. })
        ));
        assert!(matches!(
            transfer.attach_signatures(&utxos, &wrong, &[[0u8; 64]]),
            Err(Error::Signing { .. })
        ));
    }

    #[test]
    fn a_signature_count_that_does_not_match_the_inputs_is_refused() {
        // Silently zipping would leave later inputs with an empty witness and
        // broadcast an unspendable transaction, paying the fee for nothing.
        let utxos = [utxo(60_000, 0), utxo(70_000, 1), utxo(80_000, 2)];
        let transfer = transfer(150_000, 2_000);

        let error = transfer
            .attach_signatures(&utxos, &public_key(), &[[0x11; 64]])
            .unwrap_err();
        assert!(matches!(error, Error::Signing { .. }), "{error:?}");
    }
}
