//! The module-owned x402 schema and transform export.

use std::sync::Arc;

use tinybus_module::ModuleSetupContext;
use tinystoragedrivers_core::schema::{
    Schema, Step, StepBody, TransformError, TransformErrorKind, declaration_from_schema,
    delivery::{
        DocumentsTransformRequest, MigrationPayload, MigrationPayloadHandler,
        MigrationPayloadReply, StreamEntriesRequest, StreamMapMode, StreamPlan, StreamPlanRequest,
    },
};
use tinystoragedrivers_tinybus::migration::serve_module_migration;

const BUDGETS_STEP: &str = "budgets_v2";
const BUDGETS_OPERATION: &str = "tinywallet.budgets-v2";
const PAYMENTS_STEP: &str = "payments_v2";
const PAYMENTS_OPERATION: &str = "tinywallet.payments-v2";
const OPERATION_VERSION: u32 = 1;

/// Register the module-owned schema and its existing transform closures.
pub(super) async fn serve(context: &ModuleSetupContext) -> tinybus::Result<()> {
    let schema = tinywallet_x402::ledger::storage_schema();
    let declaration = declaration_from_schema(&schema, operation_for_step)
        .map_err(|_| tinybus::Error::failed("TinyWallet migration schema unavailable"))?;
    serve_module_migration(
        context,
        declaration,
        Arc::new(WalletMigrationHandler { schema }),
    )
    .await
}

fn operation_for_step(step: &Step) -> Option<(String, u32)> {
    let operation = match step.id {
        BUDGETS_STEP => BUDGETS_OPERATION,
        PAYMENTS_STEP => PAYMENTS_OPERATION,
        _ => return None,
    };
    Some((operation.to_owned(), OPERATION_VERSION))
}

struct WalletMigrationHandler {
    schema: Schema,
}

impl MigrationPayloadHandler for WalletMigrationHandler {
    fn transform(&self, payload: MigrationPayload) -> MigrationPayloadReply {
        match payload {
            MigrationPayload::Documents(request) => {
                MigrationPayloadReply::Documents(self.documents(request))
            }
            MigrationPayload::StreamPlan(request) => {
                MigrationPayloadReply::StreamPlan(self.stream_plan(request))
            }
            MigrationPayload::StreamEntries(request) => {
                MigrationPayloadReply::StreamEntries(self.stream_entries(request))
            }
            MigrationPayload::BlobTarget(_) => {
                MigrationPayloadReply::BlobTarget(Err(unsupported_operation()))
            }
        }
    }
}

impl WalletMigrationHandler {
    fn documents(
        &self,
        request: DocumentsTransformRequest,
    ) -> tinystoragedrivers_core::schema::TransformResult<Vec<tinystoragedrivers_core::TargetRecord>>
    {
        let step = self.step(
            &request.operation,
            request.version,
            BUDGETS_OPERATION,
            BUDGETS_STEP,
        )?;
        let StepBody::Documents(transform) = &step.run else {
            return Err(unsupported_operation());
        };
        transform(request.records).map_err(|_| TransformError {
            kind: TransformErrorKind::InvalidInput,
            message: "TinyWallet budget migration rejected its input".to_owned(),
        })
    }

    fn stream_plan(
        &self,
        request: StreamPlanRequest,
    ) -> tinystoragedrivers_core::schema::TransformResult<Option<StreamPlan>> {
        let step = self.step(
            &request.operation,
            request.version,
            PAYMENTS_OPERATION,
            PAYMENTS_STEP,
        )?;
        let StepBody::Streams(transform) = &step.run else {
            return Err(unsupported_operation());
        };
        match transform(request.source, Vec::new()).map_err(|_| TransformError {
            kind: TransformErrorKind::InvalidInput,
            message: "TinyWallet payment migration rejected its source".to_owned(),
        })? {
            Some((target, _)) => Ok(Some(StreamPlan {
                target,
                mode: StreamMapMode::IndependentEntries,
            })),
            None => Ok(None),
        }
    }

    fn stream_entries(
        &self,
        request: StreamEntriesRequest,
    ) -> tinystoragedrivers_core::schema::TransformResult<Vec<serde_json::Value>> {
        let step = self.step(
            &request.operation,
            request.version,
            PAYMENTS_OPERATION,
            PAYMENTS_STEP,
        )?;
        let StepBody::Streams(transform) = &step.run else {
            return Err(unsupported_operation());
        };
        let source = step.source.ok_or_else(unsupported_operation)?.to_owned();
        match transform(source, request.entries).map_err(|_| TransformError {
            kind: TransformErrorKind::InvalidInput,
            message: "TinyWallet payment migration rejected its entries".to_owned(),
        })? {
            Some((_, entries)) => Ok(entries),
            None => Err(unsupported_operation()),
        }
    }

    fn step(
        &self,
        operation: &str,
        version: u32,
        expected_operation: &str,
        step_id: &str,
    ) -> Result<&Step, TransformError> {
        if operation != expected_operation {
            return Err(unsupported_operation());
        }
        if version != OPERATION_VERSION {
            return Err(TransformError {
                kind: TransformErrorKind::UnsupportedVersion,
                message: "TinyWallet migration operation version unsupported".to_owned(),
            });
        }
        self.schema
            .steps
            .iter()
            .find(|step| step.id == step_id)
            .ok_or_else(unsupported_operation)
    }
}

fn unsupported_operation() -> TransformError {
    TransformError {
        kind: TransformErrorKind::UnsupportedOperation,
        message: "TinyWallet migration operation unsupported".to_owned(),
    }
}
