//! Tests for the host-authored storage migration readiness DTO.

use super::*;
use serde_json::json;

#[test]
fn readiness_requires_the_current_schema_and_import_epoch_fields() -> Result<(), serde_json::Error>
{
    let ready: StorageMigrationReady = serde_json::from_value(json!({
        "schema_version": 2,
        "import_epoch": "x402-payments-jsonl-v1",
    }))?;

    assert_eq!(ready.schema_version, 2);
    assert_eq!(ready.import_epoch, "x402-payments-jsonl-v1");
    assert!(
        serde_json::from_value::<StorageMigrationReady>(json!({
            "schema_version": 2,
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<StorageMigrationReady>(json!({
            "schema_version": 2,
            "import_epoch": "x402-payments-jsonl-v1",
            "extra": true,
        }))
        .is_err()
    );
    Ok(())
}
