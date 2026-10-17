# Storage-needs consumer proof

This directory is excluded from the TinyWallet workspace and has its own lock
file. It compiles the public `tinywallet-bus::storage_needs()` API from a
downstream crate while naming `tinystoragedrivers-bus::StorageNeeds` directly,
so a duplicated or copied contract type fails to compile. The CI job runs this
consumer with Rust 1.85 and `--locked`.
