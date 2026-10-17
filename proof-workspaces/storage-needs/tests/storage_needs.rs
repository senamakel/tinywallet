//! An excluded downstream consumer of the published storage-needs contract.

use tinystoragedrivers_bus::StorageNeeds;

#[test]
fn declaration_uses_the_canonical_type_and_only_wallet_ledger_resources() {
    let needs: StorageNeeds<'static> = tinywallet_bus::storage_needs();

    assert_eq!(needs.collections.len(), 1);
    let collection = &needs.collections[0];
    assert_eq!(collection.name, "x402_budget");
    assert!(collection.indexes.is_empty());
    assert_eq!(collection.ttl_field, None);
    assert_eq!(collection.search, None);
    assert_eq!(needs.stream_prefixes, ["x402.payments"]);
    assert!(needs.blob_prefixes.is_empty());
    assert!(needs.secret_prefixes.is_empty());
}
