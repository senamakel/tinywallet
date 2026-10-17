//! Tests for confidential routing decisions in the typed wallet client.

use super::*;
use std::sync::Arc;
use tinybus::module::{
    AccessFuture, FailureReporter, FailureSink, ModuleAccess, ProxyFailure, ReleaseRecord,
};

struct Quiet;

impl FailureSink for Quiet {
    fn capture(&self, _: &tinybus::module::FailureReport<'_>) {}
}

struct NoModules {
    reporter: FailureReporter,
}

impl ModuleAccess for NoModules {
    fn find(&self, _: &str) -> Option<&'static ReleaseRecord> {
        None
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

struct MustNotSerialize;

impl serde::Serialize for MustNotSerialize {
    fn serialize<S>(&self, _: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        panic!("request was serialized before routing validation")
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

#[tokio::test]
async fn ordinary_call_refuses_confidential_member_before_serializing_request() {
    let result = client()
        .call::<_, serde_json::Value>(methods::SIGN_TRANSACTION, MustNotSerialize)
        .await;
    assert!(matches!(result, Err(ClientError::ConfidentialRequired)));
}

#[tokio::test]
async fn attested_call_refuses_public_member_before_module_access() {
    let result = client()
        .call_attested::<_, serde_json::Value>(
            methods::ENCODE_ERC20_TRANSFER,
            MustNotSerialize,
        )
        .await;
    assert!(matches!(result, Err(ClientError::NotConfidential)));
}
