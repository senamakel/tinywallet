//! Tests for the module's public storage declaration.

#![allow(clippy::unwrap_used)]

use super::*;

use tinystoragedrivers_types::CollectionSpec;

#[test]
fn every_stored_item_is_wallet_owned_and_module_scoped() {
    let collections = storage_collections();
    for collection in &collections {
        assert!(
            collection.name.starts_with("wallet."),
            "{}",
            collection.name
        );
    }
    assert!(
        collections
            .iter()
            .any(|item| *item == CollectionSpec::new("wallet.budgets"))
    );
}
