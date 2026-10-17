//! Tests for confidential routing decisions in the typed wallet client.

use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tinybus::module::{
    AccessFuture, FailureReporter, FailureSink, ModuleAccess, ModuleCallError, ProxyFailure,
    ReleaseRecord,
};

struct Quiet;

impl FailureSink for Quiet {
    fn capture(&self, _: &tinybus::module::FailureReport<'_>) {}
}

struct NoModules {
    reporter: FailureReporter,
}

static RECORD: ReleaseRecord = ReleaseRecord {
    id: "tinywallet",
    bus_name: tinywallet_bus::BUS_NAME,
    object_path: tinywallet_bus::OBJECT_PATH,
    version: "0.25.0",
    release_url: "https://example.invalid/tinywallet",
    assets: &[],
};

impl ModuleAccess for NoModules {
    fn find(&self, id: &str) -> Option<&'static ReleaseRecord> {
        (id == RECORD.id).then_some(&RECORD)
    }

    fn enabled(&self) -> bool {
        true
    }

    fn reporter(&self) -> &FailureReporter {
        &self.reporter
    }

    fn ensure_loaded<'a>(&'a self, _: &'a str) -> AccessFuture<'a, Result<(), String>> {
        Box::pin(async { Err("not loaded".to_string()) })
    }

    fn proxy<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
    ) -> AccessFuture<'a, Result<tinybus::Proxy, ProxyFailure>> {
        Box::pin(async { Err(ProxyFailure::Transport) })
    }
}

fn client() -> WalletClient {
    WalletClient::new(ModuleClient::new(NoModules {
        reporter: FailureReporter::new(Arc::new(Quiet)),
    }))
}

struct MustNotSerialize(Arc<AtomicBool>);

impl serde::Serialize for MustNotSerialize {
    fn serialize<S>(&self, _: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.store(true, Ordering::SeqCst);
        Err(<S::Error as serde::ser::Error>::custom(
            "request serialization was not expected",
        ))
    }
}

#[test]
fn every_bus_confidential_member_is_classified_as_confidential() {
    for member in CONFIDENTIAL_METHODS {
        assert!(is_confidential(member), "{member}");
    }
}

#[test]
fn wallet_signing_members_are_declared_confidential() {
    for member in [
        methods::DERIVE_ACCOUNT,
        methods::DERIVE_MNEMONIC_SEED,
        methods::EXPORT_KEY,
        methods::GENERATE_MNEMONIC,
        methods::SIGN_MESSAGE,
        methods::SIGN_TRANSACTION,
    ] {
        assert!(is_confidential(member), "{member}");
    }
}

#[test]
fn calls_use_the_tinywallet_contract_identity_and_declared_members() {
    assert_eq!(tinywallet_bus::BUS_NAME, "ai.tinyhumans.tinywallet.Wallet");
    assert_eq!(
        tinywallet_bus::OBJECT_PATH,
        "/ai/tinyhumans/tinywallet/Wallet"
    );
    for member in [
        methods::DERIVE_ACCOUNT,
        methods::DERIVE_MNEMONIC_SEED,
        methods::ENCODE_ERC20_TRANSFER,
        methods::EXPORT_KEY,
        methods::GENERATE_MNEMONIC,
        methods::SIGN_MESSAGE,
        methods::SIGN_TRANSACTION,
    ] {
        assert!(
            tinywallet_bus::METHODS.contains(&member),
            "{member} is not in the contract"
        );
    }
}

#[test]
fn public_wallet_members_remain_ordinary_calls() {
    for member in [
        methods::ENCODE_ERC20_TRANSFER,
        methods::VALIDATE_ADDRESS,
        methods::WEB3_ROUTES,
        methods::X402_CLOSE,
    ] {
        assert!(!is_confidential(member), "{member}");
    }
}

#[test]
fn confidential_wire_requests_keep_their_distinct_wrapper_shapes() {
    let secret = SecretMaterial {
        mnemonic: "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".to_string(),
        derivation_path: "m/44'/60'/0'/0/0".to_string(),
        chain: tinywallet_bus::Chain::Evm,
    };
    let bare = serde_json::to_value(&secret);
    assert!(bare.is_ok(), "the secret wire type must serialize");
    let bare = bare.unwrap_or(serde_json::Value::Null);
    assert!(serde_json::from_value::<ExportRequest>(bare).is_err());

    let signing = serde_json::to_value(SignRequest {
        secret: secret.clone(),
        transaction: TransactionSpec::Evm {
            to: format!("0x{}", "11".repeat(20)),
            value_wei: "1".to_string(),
            data_hex: "0x".to_string(),
            nonce: 0,
            gas_limit: 21_000,
            gas_price_wei: "1".to_string(),
            chain_id: 1,
        },
    });
    assert!(signing.is_ok(), "the sign request wire type must serialize");
    let signing = signing.unwrap_or(serde_json::Value::Null);
    assert!(serde_json::from_value::<SignMessageRequest>(signing).is_err());
}

#[test]
fn remote_peer_details_are_absent_from_error_renderings() {
    let error = ClientError::Remote(ModuleCallFailure {
        kind: ModuleCallError::ModuleFault,
        wire_name: "private-peer-detail".to_string(),
        message: "sensitive request content".to_string(),
    });
    assert!(!format!("{error:?}").contains("private-peer-detail"));
    assert!(!format!("{error:?}").contains("sensitive request content"));
    assert!(!error.to_string().contains("private-peer-detail"));
    assert!(!error.to_string().contains("sensitive request content"));
}

#[test]
fn mnemonic_request_types_wipe_their_phrase_fields() {
    let mut secret = SecretMaterial {
        mnemonic: "sensitive phrase".to_string(),
        derivation_path: "m/44'/60'/0'/0/0".to_string(),
        chain: tinywallet_bus::Chain::Evm,
    };
    secret.zeroize_mnemonic();
    assert!(secret.mnemonic.is_empty());

    let mut request = DeriveMnemonicSeedRequest {
        mnemonic: "sensitive phrase".to_string(),
    };
    request.zeroize_mnemonic();
    assert!(request.mnemonic.is_empty());
}

fn secret_material() -> SecretMaterial {
    SecretMaterial {
        mnemonic: "sensitive phrase".to_string(),
        derivation_path: "m/44'/60'/0'/0/0".to_string(),
        chain: Chain::Evm,
    }
}

fn sign_transaction_request() -> SignRequest {
    SignRequest {
        secret: secret_material(),
        transaction: TransactionSpec::Evm {
            to: format!("0x{}", "11".repeat(20)),
            value_wei: "1".to_string(),
            data_hex: "0x".to_string(),
            nonce: 0,
            gas_limit: 21_000,
            gas_price_wei: "1".to_string(),
            chain_id: 1,
        },
    }
}

fn sign_message_request() -> SignMessageRequest {
    SignMessageRequest {
        secret: secret_material(),
        message_hex: "42".to_string(),
        scheme: Scheme::Secp256k1Prehash,
    }
}

fn assert_sign_transaction_request_wiped_on_result(result: Result<(), ()>) {
    let mut request = sign_transaction_request();
    let outcome = {
        let _request = MnemonicRequestGuard(&mut request);
        result
    };
    assert_eq!(outcome, result);
    assert!(request.secret.mnemonic.is_empty());
}

fn assert_sign_message_request_wiped_on_result(result: Result<(), ()>) {
    let mut request = sign_message_request();
    let outcome = {
        let _request = MnemonicRequestGuard(&mut request);
        result
    };
    assert_eq!(outcome, result);
    assert!(request.secret.mnemonic.is_empty());
}

fn assert_export_request_wiped_on_result(result: Result<(), ()>) {
    let mut request = ExportRequest {
        secret: secret_material(),
    };
    let outcome = {
        let _request = MnemonicRequestGuard(&mut request);
        result
    };
    assert_eq!(outcome, result);
    assert!(request.secret.mnemonic.is_empty());
}

#[test]
fn signing_and_export_request_guards_wipe_after_success_and_error() {
    for result in [Ok(()), Err(())] {
        assert_sign_transaction_request_wiped_on_result(result);
        assert_sign_message_request_wiped_on_result(result);
        assert_export_request_wiped_on_result(result);
    }
}

async fn abort_request_while_guarded<T>(request: Arc<tokio::sync::Mutex<T>>)
where
    T: ZeroizeMnemonic + Send + 'static,
{
    let task_request = request.clone();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut request = task_request.lock().await;
        let _guard = MnemonicRequestGuard(&mut *request);
        let _ = started_tx.send(());
        std::future::pending::<()>().await;
    });
    assert!(started_rx.await.is_ok());
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn signing_and_export_request_guards_wipe_when_the_call_is_cancelled() {
    let transaction = Arc::new(tokio::sync::Mutex::new(sign_transaction_request()));
    abort_request_while_guarded(transaction.clone()).await;
    assert!(transaction.lock().await.secret.mnemonic.is_empty());

    let message = Arc::new(tokio::sync::Mutex::new(sign_message_request()));
    abort_request_while_guarded(message.clone()).await;
    assert!(message.lock().await.secret.mnemonic.is_empty());

    let export = Arc::new(tokio::sync::Mutex::new(ExportRequest {
        secret: secret_material(),
    }));
    abort_request_while_guarded(export.clone()).await;
    assert!(export.lock().await.secret.mnemonic.is_empty());
}

#[tokio::test]
async fn ordinary_call_refuses_confidential_member_before_serializing_request() {
    for member in CONFIDENTIAL_METHODS {
        let serialized = Arc::new(AtomicBool::new(false));
        let result = client()
            .call::<_, serde_json::Value>(member, MustNotSerialize(serialized.clone()))
            .await;
        assert!(
            matches!(result, Err(ClientError::ConfidentialRequired)),
            "{member}"
        );
        assert!(!serialized.load(Ordering::SeqCst), "{member}");
    }
}

#[tokio::test]
async fn attested_call_refuses_public_member_before_module_access() {
    let serialized = Arc::new(AtomicBool::new(false));
    let result = client()
        .call_attested::<_, serde_json::Value>(
            methods::ENCODE_ERC20_TRANSFER,
            MustNotSerialize(serialized.clone()),
        )
        .await;
    assert!(matches!(result, Err(ClientError::NotConfidential)));
    assert!(!serialized.load(Ordering::SeqCst));
}

#[tokio::test]
async fn attested_call_with_no_release_library_pins_never_serializes_request() {
    let serialized = Arc::new(AtomicBool::new(false));
    let result = client()
        .call_attested::<_, serde_json::Value>(
            methods::SIGN_MESSAGE,
            MustNotSerialize(serialized.clone()),
        )
        .await;
    assert!(matches!(
        result,
        Err(ClientError::Remote(failure)) if failure.kind == ModuleCallError::Unavailable
    ));
    assert!(!serialized.load(Ordering::SeqCst));
}
