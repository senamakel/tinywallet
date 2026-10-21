//! Exercise the storage callback against a live, host-issued storage lease.

use super::*;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
use tinystoragedrivers_tinybus::migration::MigrationModuleClient;

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

    let registration = Arc::try_unwrap(registration)
        .map_err(|_| std::io::Error::other("wallet storage registration remained in use"))?;
    registration.shutdown().await?;
    host.shutdown(Duration::from_secs(2)).await;
    broker_task.abort();
    Ok(())
}

struct MigrationSetupContext {
    controller: Connection,
    module_peer: tinybus::BusName,
}

impl HostContext for MigrationSetupContext {}

struct MigrationSetupEnvironment {
    backend: Arc<MemoryStorage>,
    require_migration: Arc<AtomicBool>,
    migration_discovered: Arc<AtomicBool>,
}

#[async_trait]
impl Environment<MigrationSetupContext> for MigrationSetupEnvironment {
    async fn resolve(
        &self,
        context: &MigrationSetupContext,
        _module: &tinybus::module::ModuleId,
    ) -> Result<ResolvedStorage, HostFailure> {
        if self.require_migration.load(Ordering::Acquire) {
            MigrationModuleClient::discover(&context.controller, &context.module_peer)
                .await
                .map_err(|_| HostFailure::Denied("wallet-migration-unavailable"))?;
            self.migration_discovered.store(true, Ordering::Release);
        }
        let storage = self
            .backend
            .for_scope(&Scope::local())
            .map_err(|_| HostFailure::Denied("test-storage-unavailable"))?;
        let grant = StorageGrant::from_needs(StorageNeeds::default(), &GrantPolicy::default())
            .map_err(|_| HostFailure::Denied("test-storage-grant-invalid"))?;
        Ok(ResolvedStorage {
            backend_identity: "wallet-initial-migration-test".to_owned(),
            binding_epoch: 1,
            canonical_scope: "wallet-initial-migration-test".to_owned(),
            grant,
            server: Arc::new(StorageServer::new(storage)),
        })
    }

    async fn admit_write(
        &self,
        _context: &MigrationSetupContext,
        _module: &tinybus::module::ModuleId,
    ) -> Result<(), HostFailure> {
        Ok(())
    }
}

struct MigrationSetupAudit;

impl AuditSink for MigrationSetupAudit {
    fn record(&self, _event: AuditEvent) {}
}

type MigrationSetupRegistration = tinystoragedrivers_tinybus::StorageRegistration<
    MigrationSetupEnvironment,
    MigrationSetupAudit,
    MigrationSetupContext,
>;

struct InitialSetupFixture {
    registration: Arc<MigrationSetupRegistration>,
    controller: Connection,
    require_migration: Arc<AtomicBool>,
}

static INITIAL_SETUP_FIXTURE: std::sync::OnceLock<
    std::sync::Mutex<Option<InitialSetupFixture>>,
> = std::sync::OnceLock::new();

fn initial_setup_fixture() -> &'static std::sync::Mutex<Option<InitialSetupFixture>> {
    INITIAL_SETUP_FIXTURE.get_or_init(|| std::sync::Mutex::new(None))
}

mod initial_setup_export {
    use super::*;

    async fn setup(
        context: tinybus_module::ModuleSetupContext,
        _config: Option<crate::service::WalletModuleConfig>,
    ) -> tinybus::Result<()> {
        let fixture = initial_setup_fixture()
            .lock()
            .map_err(|_| tinybus::Error::failed("initial setup fixture lock poisoned"))?
            .take()
            .ok_or_else(|| tinybus::Error::failed("initial setup fixture missing"))?;
        let connection = context.connection().clone();
        let peer = connection
            .unique_name()
            .ok_or_else(|| tinybus::Error::failed("wallet module has no unique peer"))?;
        let (lease, descriptor) = fixture
            .registration
            .register_lease(
                tinystoragedrivers_busproxy::HostStorageContext::new(MigrationSetupContext {
                    controller: fixture.controller,
                    module_peer: peer.clone(),
                }),
                peer,
            )
            .await
            .map_err(|_| tinybus::Error::failed("wallet storage lease was refused"))?;
        // Lease creation resolves once before migrations are required. The
        // configured setup below resolves again and must discover the actual
        // module-owned GetSchema service before binding storage.
        fixture.require_migration.store(true, Ordering::Release);
        INITIAL_SETUP_LEASE
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .map_err(|_| tinybus::Error::failed("initial setup lease lock poisoned"))?
            .replace(lease);

        crate::service::setup_with_context(
            context,
            Some(crate::service::WalletModuleConfig {
                storage_descriptor: Some(descriptor),
            }),
        )
        .await
    }

    tinybus_module::module_export_static! {
        setup_with_context = setup,
        config = Option<crate::service::WalletModuleConfig>,
        worker_threads = 2,
        provides = [
            "ai.tinyhumans.tinywallet.Wallet",
            "org.tinystoragedrivers.Migration",
        ],
        methods = [],
        signals = [],
        requires = [],
        optional = [],
        lazy = false,
    }
}

static INITIAL_SETUP_LEASE: std::sync::OnceLock<
    std::sync::Mutex<Option<tinystoragedrivers_busproxy::StorageLease>>,
> = std::sync::OnceLock::new();

#[tokio::test(flavor = "multi_thread")]
async fn initial_bound_storage_validation_can_discover_wallet_migrations()
-> Result<(), Box<dyn std::error::Error>> {
    let bus = MemoryBus::new();
    let broker = Broker::new_restricted_module_control();
    let (broker_transport, controller_transport) = MemoryTransport::pair();
    broker.attach_module_controller(Arc::new(broker_transport))?;
    let controller = Connection::connect(Box::new(controller_transport)).await?;
    let broker_task = broker.spawn(bus.clone());
    let host = ModuleHost::new(broker);
    let backend = Arc::new(MemoryStorage::new());
    let require_migration = Arc::new(AtomicBool::new(false));
    let migration_discovered = Arc::new(AtomicBool::new(false));
    let registration: Arc<MigrationSetupRegistration> = Arc::new(
        tinystoragedrivers_tinybus::attach_storage(
            &host,
            &controller,
            GovernedStorage::new(
                Arc::new(MigrationSetupEnvironment {
                    backend,
                    require_migration: Arc::clone(&require_migration),
                    migration_discovered: Arc::clone(&migration_discovered),
                }),
                Arc::new(MigrationSetupAudit),
                GovernedEndpointLimits::default(),
            ),
            tokio::runtime::Handle::current(),
        )
        .await
        .map_err(|_| std::io::Error::other("governed storage attachment failed"))?,
    );
    *initial_setup_fixture()
        .lock()
        .map_err(|_| std::io::Error::other("initial setup fixture lock poisoned"))? =
        Some(InitialSetupFixture {
            registration: Arc::clone(&registration),
            controller: controller.clone(),
            require_migration,
        });

    let info = host.attach_linked_with_config(
        initial_setup_export::linked_module()?,
        serde_json::json!({}),
    )?;
    host.ensure_ready(info.manifest.bus_name.as_str()).await?;
    assert!(
        migration_discovered.load(Ordering::Acquire),
        "initial bound storage validation must discover GetSchema after the test lease enables it"
    );
    let module_peer = controller
        .name_owner(crate::BUS_NAME)
        .await?
        .ok_or_else(|| std::io::Error::other("wallet module did not request its bus name"))?;
    let migration = MigrationModuleClient::discover(&controller, &module_peer)
        .await
        .map_err(|_| std::io::Error::other("initial setup did not serve wallet migrations"))?;
    assert_eq!(migration.declaration().owner.as_str(), "wallet");

    let lease = INITIAL_SETUP_LEASE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .map_err(|_| std::io::Error::other("initial setup lease lock poisoned"))?
        .take()
        .ok_or_else(|| std::io::Error::other("initial setup did not retain its lease"))?;
    assert!(registration.revoke(lease).await);
    let registration = Arc::try_unwrap(registration)
        .map_err(|_| std::io::Error::other("storage registration remained in use"))?;
    registration.shutdown().await?;
    host.shutdown(Duration::from_secs(2)).await;
    broker_task.abort();
    Ok(())
}
