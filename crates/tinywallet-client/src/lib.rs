//! Typed host calls to the TinyWallet TinyBus module.
//!
//! WalletClient is a transport facade over a host configured
//! tinybus::module::ModuleClient. It owns no loader, endpoint policy, approval
//! flow, provider configuration, or wallet implementation. Confidential calls
//! use call_attested, which checks the host registry's archive and extracted
//! library pins against the same unique peer before serializing a request.

use std::fmt;

use serde::{Serialize, de::DeserializeOwned};
use tinybus::module::{ModuleCallError, ModuleCallFailure, ModuleClient};
use tinywallet_bus::names::{CONFIDENTIAL_METHODS, methods};
use tinywallet_bus::wire::{
    DeriveMnemonicSeedRequest, DeriveMnemonicSeedResponse, DerivedAccount,
    EncodeErc20TransferRequest, EncodeErc20TransferResponse, ExportRequest, ExportedKey,
    GenerateMnemonicRequest, GenerateMnemonicResponse, Scheme, SecretMaterial, SignMessageRequest,
    SignRequest, Signature, SignedTransaction, TransactionSpec,
};

const MODULE_ID: &str = "tinywallet";

/// Failure from a typed TinyWallet module call.
///
/// Detailed remote prose is retained only for ordinary calls in Remote. Its
/// Debug and Display renderings redact that prose through TinyBus. Attested
/// calls expose only their sanitized failure kind.
#[derive(Debug)]
pub enum ClientError {
    /// An ordinary call was attempted for a member declared confidential.
    ConfidentialRequired,
    /// An attested call was attempted for a public member.
    NotConfidential,
    /// An ordinary module call failed.
    Remote(ModuleCallFailure),
    /// Strict attestation or confidential delivery failed.
    Attested(ModuleCallError),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConfidentialRequired => f.write_str("wallet member requires attested delivery"),
            Self::NotConfidential => f.write_str("wallet member is not confidential"),
            Self::Remote(error) => fmt::Display::fmt(error, f),
            Self::Attested(error) => fmt::Display::fmt(error, f),
        }
    }
}

impl std::error::Error for ClientError {}

/// Typed call facade over the host's configured TinyBus module client.
#[derive(Clone)]
pub struct WalletClient {
    client: ModuleClient,
}

impl fmt::Debug for WalletClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WalletClient").finish_non_exhaustive()
    }
}

impl WalletClient {
    /// Create a wallet facade from host-owned module access and release pins.
    #[must_use]
    pub fn new(client: ModuleClient) -> Self {
        Self { client }
    }

    /// Make a typed ordinary call with exactly one request argument.
    ///
    /// Members declared confidential by tinywallet-bus are refused before the
    /// request is serialized.
    ///
    /// # Errors
    ///
    /// Returns ConfidentialRequired for a confidential member, or Remote for
    /// a failed module call.
    pub async fn call<Request, Reply>(
        &self,
        member: &str,
        request: Request,
    ) -> Result<Reply, ClientError>
    where
        Request: Serialize + Send,
        Reply: DeserializeOwned,
    {
        if is_confidential(member) {
            return Err(ClientError::ConfidentialRequired);
        }
        self.client
            .call_detailed(MODULE_ID, member, (request,))
            .await
            .map_err(ClientError::Remote)
    }

    /// Make a typed call using strict peer and release-library attestation.
    ///
    /// TinyBus validates archive and extracted-library pins in the host's
    /// ReleaseRecord, then proves both against the same unique peer before it
    /// serializes the request. There is no ordinary-delivery fallback.
    ///
    /// # Errors
    ///
    /// Returns NotConfidential for a public member, or Attested if the strict
    /// call fails.
    pub async fn call_attested<Request, Reply>(
        &self,
        member: &str,
        request: Request,
    ) -> Result<Reply, ClientError>
    where
        Request: Serialize + Send,
        Reply: DeserializeOwned,
    {
        if !is_confidential(member) {
            return Err(ClientError::NotConfidential);
        }
        self.client
            .call_attested(MODULE_ID, member, (request,))
            .await
            .map_err(ClientError::Attested)
    }

    /// Derive an account through an attested wallet module.
    ///
    /// # Errors
    ///
    /// Returns an error if the module is unavailable, unattested, or rejects
    /// the request.
    pub async fn derive_account(
        &self,
        secret: SecretMaterial,
    ) -> Result<DerivedAccount, ClientError> {
        self.call_attested(methods::DERIVE_ACCOUNT, secret).await
    }

    /// Sign a transaction inside the attested wallet module.
    ///
    /// # Errors
    ///
    /// Returns an error if the module is unavailable, unattested, or rejects
    /// the transaction.
    pub async fn sign_transaction(
        &self,
        secret: SecretMaterial,
        transaction: TransactionSpec,
    ) -> Result<SignedTransaction, ClientError> {
        self.call_attested(
            methods::SIGN_TRANSACTION,
            SignRequest {
                secret,
                transaction,
            },
        )
        .await
    }

    /// Sign opaque bytes inside the attested wallet module.
    ///
    /// # Errors
    ///
    /// Returns an error if the module is unavailable, unattested, or rejects
    /// the request.
    pub async fn sign_message(
        &self,
        secret: SecretMaterial,
        message_hex: String,
        scheme: Scheme,
    ) -> Result<Signature, ClientError> {
        self.call_attested(
            methods::SIGN_MESSAGE,
            SignMessageRequest {
                secret,
                message_hex,
                scheme,
            },
        )
        .await
    }

    /// Export a derived key through the attested wallet module.
    ///
    /// # Errors
    ///
    /// Returns an error if the module is unavailable, unattested, or rejects
    /// the request.
    pub async fn export_key(&self, secret: SecretMaterial) -> Result<ExportedKey, ClientError> {
        self.call_attested(methods::EXPORT_KEY, ExportRequest { secret })
            .await
    }

    /// Generate a recovery phrase in the attested wallet module.
    ///
    /// # Errors
    ///
    /// Returns an error if the module is unavailable, unattested, or rejects
    /// the requested word count.
    pub async fn generate_mnemonic(
        &self,
        word_count: u8,
    ) -> Result<GenerateMnemonicResponse, ClientError> {
        self.call_attested(
            methods::GENERATE_MNEMONIC,
            GenerateMnemonicRequest { word_count },
        )
        .await
    }

    /// Derive a BIP-39 seed through the attested wallet module.
    ///
    /// # Errors
    ///
    /// Returns an error if the module is unavailable, unattested, or rejects
    /// the phrase.
    pub async fn derive_mnemonic_seed(
        &self,
        mnemonic: String,
    ) -> Result<DeriveMnemonicSeedResponse, ClientError> {
        self.call_attested(
            methods::DERIVE_MNEMONIC_SEED,
            DeriveMnemonicSeedRequest { mnemonic },
        )
        .await
    }

    /// Encode ERC-20 transfer calldata with the module-owned ABI implementation.
    ///
    /// # Errors
    ///
    /// Returns an error if the module is unavailable or rejects the recipient
    /// or amount.
    pub async fn encode_erc20_transfer(
        &self,
        to_address: String,
        amount_raw: String,
    ) -> Result<EncodeErc20TransferResponse, ClientError> {
        self.call(
            methods::ENCODE_ERC20_TRANSFER,
            EncodeErc20TransferRequest {
                to_address,
                amount_raw,
            },
        )
        .await
    }
}

fn is_confidential(member: &str) -> bool {
    CONFIDENTIAL_METHODS.contains(&member)
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
