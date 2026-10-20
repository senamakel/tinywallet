//! Tests for the module's public storage declaration.

#![allow(clippy::unwrap_used)]

use super::*;

use tinystoragedrivers_core::{Registry, ScopeClass};

#[test]
fn every_stored_item_is_wallet_owned_and_module_scoped() {
    let schema = storage_schema();
    assert_eq!(schema.owner.as_str(), "wallet");
    assert_ne!(
        schema.items,
        [] as [tinystoragedrivers_core::Item; 0],
        "the wallet schema must declare at least one persisted item"
    );
    for item in &schema.items {
        assert!(item.name().starts_with("wallet."), "{}", item.name());
        assert_eq!(item.scope(), ScopeClass::Module, "{}", item.name());
    }
    Registry::compose([schema]).unwrap();
}
