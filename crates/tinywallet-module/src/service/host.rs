//! Typed callbacks from the wallet implementation to its composing host.

use std::sync::Arc;

use log::debug;
use tinybus::{Connection, Error as BusError, Proxy};
use tinystoragedrivers_core::{ScopedStorage, StorageBackend};
use tinywallet_bus::wire::{
    AuthorizationResult, AuthorizeNativeWalletExecutionRequest, AuthorizeWeb3ExecutionRequest,
    AuthorizeX402PaymentRequest, ChainTransportRequest, ChainTransportResult, HostResponse,
    WalletAccountsResponse, WalletBackendRequest, WalletBackendResponse,
};
use tinywallet_x402::ledger::LedgerStorage;

use super::{HOST_INTERFACE, HOST_OBJECT_PATH};

/// A typed handle to the host services the module is permitted to request.
#[derive(Clone)]
pub(crate) struct HostCallbacks {
    proxy: Proxy,
    connection: Connection,
    /// The host's storage, connected on first use and shared by every clone.
    storage: Arc<tokio::sync::Mutex<Option<ScopedStorage>>>,
    /// Test seam: ledger storage used in place of the storage bus.
    #[cfg(test)]
    test_ledger_storage: Arc<std::sync::Mutex<Option<LedgerStorage>>>,
}

impl std::fmt::Debug for HostCallbacks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostCallbacks").finish_non_exhaustive()
    }
}

impl HostCallbacks {
    /// Bind the callback handle to the host interface on the module connection.
    pub(crate) fn new(connection: &Connection) -> Result<Self, BusError> {
        Ok(Self {
            proxy: connection.proxy(HOST_INTERFACE, HOST_OBJECT_PATH, HOST_INTERFACE)?,
            connection: connection.clone(),
            storage: Arc::new(tokio::sync::Mutex::new(None)),
            #[cfg(test)]
            test_ledger_storage: Arc::new(std::sync::Mutex::new(None)),
        })
    }

    /// The module's scoped handle on the host's storage, over the storage bus.
    ///
    /// The host binds the scope from the authenticated caller; nothing here
    /// names one. A connect failure is not cached, so a host that starts
    /// serving storage later is picked up by the next call.
    async fn storage(&self) -> tinystoragedrivers_core::Result<ScopedStorage> {
        let mut slot = self.storage.lock().await;
        if let Some(scoped) = slot.as_ref() {
            return Ok(scoped.clone());
        }
        debug!("[wallet::host] connecting to the host storage bus");
        let bus = tinystoragedrivers_busproxy::connect_storage(self.connection.clone()).await?;
        let scoped = bus.for_scope(&tinystoragedrivers_busproxy::module_scope_label())?;
        *slot = Some(scoped.clone());
        Ok(scoped)
    }

    /// The ports the x402 ledger persists through: the host's payments stream,
    /// budget-hold document, and readiness document in the same scope.
    ///
    /// # Errors
    ///
    /// The host does not serve storage. The caller fails closed rather than
    /// keep a spending ledger that would not survive a restart.
    pub(crate) async fn ledger_storage(&self) -> Result<LedgerStorage, String> {
        #[cfg(test)]
        if let Some(storage) = self
            .test_ledger_storage
            .lock()
            .map_err(|_| "test ledger storage lock poisoned".to_string())?
            .clone()
        {
            return Ok(storage);
        }
        match self.storage().await {
            Ok(scoped) => Ok(LedgerStorage::from_scoped(&scoped)),
            #[cfg(feature = "e2e-memory-storage")]
            Err(error) => {
                static MEMORY: std::sync::OnceLock<LedgerStorage> = std::sync::OnceLock::new();
                log::warn!("[wallet::host] host storage unavailable ({error}); using memory");
                Ok(MEMORY.get_or_init(LedgerStorage::memory).clone())
            }
            #[cfg(not(feature = "e2e-memory-storage"))]
            Err(error) => Err(format!("x402 ledger storage is unavailable: {error}")),
        }
    }

    /// Serve `storage` to the ledger in place of the storage bus.
    #[cfg(test)]
    #[allow(clippy::unwrap_used)]
    pub(crate) fn set_test_ledger_storage(&self, storage: LedgerStorage) {
        *self.test_ledger_storage.lock().unwrap() = Some(storage);
    }

    /// Perform one authenticated wallet-backend transport operation.
    pub(crate) async fn wallet_backend(
        &self,
        request: WalletBackendRequest,
    ) -> Result<WalletBackendResponse, String> {
        self.call("WalletBackend", (request,)).await
    }

    /// Read public account state from the host's custody store.
    pub(crate) async fn wallet_accounts(&self) -> Result<WalletAccountsResponse, String> {
        self.call("WalletAccounts", ()).await
    }

    /// Execute one chain transport operation using host endpoint and proxy
    /// policy.
    pub(crate) async fn chain_transport(
        &self,
        request: ChainTransportRequest,
    ) -> Result<ChainTransportResult, String> {
        self.call("ChainTransport", (request,)).await
    }

    /// Request approval for the exact facts in a stored Web3 transaction.
    pub(crate) async fn authorize_web3_execution(
        &self,
        request: AuthorizeWeb3ExecutionRequest,
    ) -> Result<AuthorizationResult, String> {
        self.call("AuthorizeWeb3Execution", (request,)).await
    }

    /// Request approval for the exact facts of one prepared native transfer.
    pub(crate) async fn authorize_native_wallet_execution(
        &self,
        request: AuthorizeNativeWalletExecutionRequest,
    ) -> Result<AuthorizationResult, String> {
        self.call("AuthorizeNativeWalletExecution", (request,))
            .await
    }

    /// Request host approval for exact x402 payment terms.
    pub(crate) async fn authorize_x402_payment(
        &self,
        request: AuthorizeX402PaymentRequest,
    ) -> Result<AuthorizationResult, String> {
        self.call("AuthorizeX402Payment", (request,)).await
    }

    async fn call<R: serde::de::DeserializeOwned>(
        &self,
        member: &str,
        args: impl serde::Serialize,
    ) -> Result<R, String> {
        match self.proxy.call::<HostResponse>(member, args).await {
            Ok(response) => extract_response(member, response),
            Err(error) => Err(error.to_string()),
        }
    }
}

fn extract_response<R: serde::de::DeserializeOwned>(
    expected: &str,
    response: HostResponse,
) -> Result<R, String> {
    let value = match (expected, response) {
        ("WalletBackend", HostResponse::WalletBackend(value)) => serde_json::to_value(value),
        ("WalletAccounts", HostResponse::WalletAccounts(value)) => serde_json::to_value(value),
        ("ChainTransport", HostResponse::ChainTransport(value)) => serde_json::to_value(value),
        (
            "AuthorizeWeb3Execution" | "AuthorizeNativeWalletExecution" | "AuthorizeX402Payment",
            HostResponse::Authorization(value),
        ) => serde_json::to_value(value),
        (_, response) => {
            return Err(format!(
                "host callback {expected} returned the wrong response kind: {response:?}"
            ));
        }
    }
    .map_err(|error| format!("encoding host callback response {expected}: {error}"))?;
    serde_json::from_value(value)
        .map_err(|error| format!("decoding host callback response {expected}: {error}"))
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
