//! Host-authored readiness record for opening the current x402 ledger.

use serde::{Deserialize, Serialize};

/// Migration readiness written by the host after wallet storage migration and
/// legacy payment import have both completed for this module's scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageMigrationReady {
    /// Current wallet collection and stream schema version.
    pub schema_version: u32,
    /// Legacy payment-file importer epoch completed by the host.
    pub import_epoch: String,
}
