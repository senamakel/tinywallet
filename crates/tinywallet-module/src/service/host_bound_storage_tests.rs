//! Exercise the storage callback against a live, host-issued storage lease.

use super::*;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tinybus::{
    Connection,
    broker::Broker,
    module::ModuleHost,
    transport::memory::{MemoryBus, MemoryTransport},
};
use tinystoragedrivers_bus::StorageNeeds;
use tinystoragedrivers_busproxy::{
    AuditEvent, AuditSink, Environment, GovernedEndpointLimits, GovernedStorage, GrantPolicy,
    HostContext, HostFailure, ResolvedStorage, StorageGrant,
};
use tinystoragedrivers_client::StorageServer;
use tinystoragedrivers_core::{MemoryStorage, Scope, StorageBackend};

struct BoundStorageContext;

impl HostContext for BoundStorageContext {}

struct BoundStorageEnvironment {
    backend: Arc<MemoryStorage>,
}

#[async_trait]
impl Environment<BoundStorageContext> for BoundStorageEnvironment {
    async fn resolve(
        &self,
        _context: &BoundStorageContext,
        _module: &tinybus::module::ModuleId,
    ) -> Result<ResolvedStorage, HostFailure> {
        let storage = self
            .backend
            .for_scope(&Scope::local())
            .map_err(|_| HostFailure::Denied("test-storage-unavailable"))?;
        let grant = StorageGrant::from_needs(StorageNeeds::default(), &GrantPolicy::default())
            .map_err(|_| HostFailure::Denied("test-storage-grant-invalid"))?;
        Ok(ResolvedStorage {
            backend_identity: "wallet-bound-storage-test".to_owned(),
            binding_epoch: 1,
            canonical_scope: "wallet-test".to_owned(),
            grant,
            server: Arc::new(StorageServer::new(storage)),
        })
    }

    async fn admit_write(
        &self,
        _context: &BoundStorageContext,
        _module: &tinybus::module::ModuleId,
    ) -> Result<(), HostFailure> {
        Ok(())
    }
}

struct BoundStorageAudit;

impl AuditSink for BoundStorageAudit {
    fn record(&self, _event: AuditEvent) {}
}

type BoundStorageRegistration = tinystoragedrivers_tinybus::StorageRegistration<
    BoundStorageEnvironment,
    BoundStorageAudit,
    BoundStorageContext,
>;

#[tokio::test(flavor = "multi_thread")]
async fn selected_linked_wallet_uses_and_caches_its_current_only_ledger_route()
-> Result<(), Box<dyn std::error::Error>> {
    let bus = MemoryBus::new();
    let broker = Broker::new_restricted_module_control();
    let (broker_transport, controller_transport) = MemoryTransport::pair();
    broker.attach_module_controller(Arc::new(broker_transport))?;
    let controller = Connection::connect(Box::new(controller_transport)).await?;
    let broker_task = broker.spawn(bus.clone());
    let host = ModuleHost::new(broker);
    let backend = Arc::new(MemoryStorage::new());
    let environment = Arc::new(BoundStorageEnvironment {
        backend: Arc::clone(&backend),
    });
    let registration: Arc<BoundStorageRegistration> = Arc::new(
        tinystoragedrivers_tinybus::attach_storage(
            &host,
            &controller,
            GovernedStorage::new(
                environment,
                Arc::new(BoundStorageAudit),
                GovernedEndpointLimits::default(),
            ),
            tokio::runtime::Handle::current(),
        )
        .await
        .map_err(|_| std::io::Error::other("governed storage attachment failed"))?,
    );
    let (lease_tx, lease_rx) = tokio::sync::oneshot::channel();
    let registration_for_setup = Arc::clone(&registration);
    let info = host
        .attach_linked_with_setup(
            crate::linked_module()?,
            serde_json::json!({}),
            move |connection| {
                Box::pin(async move {
                    let peer = connection.unique_name().ok_or_else(|| {
                        tinybus::Error::failed("wallet module has no unique peer")
                    })?;
                    let (lease, descriptor) = registration_for_setup
                        .register_lease(
                            tinystoragedrivers_busproxy::HostStorageContext::new(
                                BoundStorageContext,
                            ),
                            peer.clone(),
                        )
                        .await
                        .map_err(|_| tinybus::Error::failed("wallet storage lease was refused"))?;
                    let callbacks =
                        HostCallbacks::with_storage_descriptor(&connection, Some(descriptor))?;
                    callbacks.validate_configured_storage().await.map_err(|_| {
                        tinybus::Error::failed("wallet storage route was unavailable")
                    })?;
                    callbacks
                        .storage()
                        .await
                        .map_err(|_| {
                            tinybus::Error::failed("wallet storage route was unavailable")
                        })?
                        .ok_or_else(|| tinybus::Error::failed("wallet storage route was absent"))?;
                    callbacks.ledger_storage().await.map_err(|_| {
                        tinybus::Error::failed("wallet ledger storage was unavailable")
                    })?;
                    lease_tx
                        .send((lease, peer))
                        .map_err(|_| tinybus::Error::failed("wallet lease observer was dropped"))?;
                    crate::service::setup_with_callbacks(connection, callbacks).await
                })
            },
        )
        .await?;
    host.ensure_ready(info.manifest.bus_name.as_str()).await?;
    let (_lease, module_peer) = lease_rx.await?;
    if controller.name_owner(crate::BUS_NAME).await?.as_ref() != Some(&module_peer) {
        return Err(std::io::Error::other("wallet module peer changed after setup").into());
    }

    let proxy = controller.proxy(crate::BUS_NAME, crate::OBJECT_PATH, crate::BUS_NAME)?;
    let response: tinywallet_bus::wire::ValidateAddressResponse = proxy
        .call(
            "ValidateAddress",
            (tinywallet_bus::wire::ValidateAddressRequest {
                chain: tinywallet::Chain::Evm,
                address: "0x3535353535353535353535353535353535353535".to_owned(),
                sender: false,
            },),
        )
        .await?;
    if response.result.is_err() {
        return Err(std::io::Error::other("wallet safe operation failed after lease setup").into());
    }

    registration.shutdown().await?;
    host.shutdown(Duration::from_secs(2)).await?;
    broker_task.abort();
    Ok(())
}
