//! An excluded downstream consumer of the published collection contract.

use tinystoragedrivers_types::CollectionSpec;

#[test]
fn declaration_uses_the_canonical_type_and_wallet_ledger_collections() {
    let collections: Vec<CollectionSpec> = tinywallet_bus::storage::collections();

    assert_eq!(collections.len(), 2);
    assert_eq!(collections[0].name, "wallet.budgets");
    assert_eq!(collections[1].name, "wallet.migrations");
    let collection = &collections[0];
    assert!(collection.indexes.is_empty());
    assert_eq!(collection.ttl_field, None);
    assert_eq!(collection.search, None);
}
