//! Transaction building and signing.
//!
//! Pure: a transaction is built and signed from its fields and a key, with no
//! network involved. Fetching a nonce or a gas price and broadcasting the
//! result are [`crate::client`]'s job, over the host's
//! [`Transport`](crate::rpc::Transport).
//!
//! Splitting it this way is what makes signing testable. A signed transaction
//! is a deterministic function of its inputs, so it can be pinned against a
//! published vector byte-for-byte — which matters more here than anywhere else
//! in the crate, because a signing bug does not fail loudly. It produces a
//! well-formed transaction that moves the wrong funds, or one that is valid on
//! a chain the user did not intend.

pub mod evm;
mod rlp;
#[cfg(feature = "solana")]
pub mod solana;

#[cfg(test)]
mod test;

/// Errors raised while building or signing a transaction.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// An address in the transaction was rejected.
    #[error(transparent)]
    Address(crate::Error),

    /// A field was structurally invalid.
    #[error("invalid transaction field '{field}': {reason}")]
    InvalidField {
        /// Which field.
        field: &'static str,
        /// Why it was rejected.
        reason: String,
    },

    /// Signing failed.
    ///
    /// Carries no key material — see [`crate::key`] for why an error string is
    /// the easiest way for a secret to escape.
    #[error("signing failed: {reason}")]
    Signing {
        /// What went wrong, never including key material.
        reason: String,
    },
}

/// Result alias for transaction building and signing.
pub type Result<T> = std::result::Result<T, Error>;
