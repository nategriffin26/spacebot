//! Structured worker outcome receipts persisted alongside worker runs.

use crate::conversation::history::ProcessRunLogger;
use crate::{WorkerId, WorkerOutcomeReceipt};
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Tool for persisting a compact execution receipt before a worker finishes.
#[derive(Debug, Clone)]
pub struct RecordReceiptTool {
    worker_id: WorkerId,
    run_logger: ProcessRunLogger,
}

impl RecordReceiptTool {
    pub fn new(worker_id: WorkerId, run_logger: ProcessRunLogger) -> Self {
        Self {
            worker_id,
            run_logger,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Failed to record receipt: {0}")]
pub struct RecordReceiptError(String);

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RecordReceiptArgs {
    /// One-paragraph summary of the work outcome.
    pub summary: String,
    /// Explicit sources actually used for the work.
    #[serde(default)]
    pub sources_used: Vec<String>,
    /// Files, artifacts, or outputs created during execution.
    #[serde(default)]
    pub artifacts_created: Vec<String>,
    /// Commands or scripts that were run.
    #[serde(default)]
    pub commands_run: Vec<String>,
    /// Claims backed by direct execution or inspection.
    #[serde(default)]
    pub verified_claims: Vec<String>,
    /// Reasonable inferences that are not directly verified.
    #[serde(default)]
    pub inferred_claims: Vec<String>,
    /// Any remaining blockers, gaps, or failures.
    #[serde(default)]
    pub blockers: Vec<String>,
    /// Set true if the worker detected that the wrong source or artifact had been used.
    #[serde(default)]
    pub wrong_source_detected: bool,
}

#[derive(Debug, Serialize)]
pub struct RecordReceiptOutput {
    pub success: bool,
    pub worker_id: WorkerId,
    pub receipt: WorkerOutcomeReceipt,
}

impl Tool for RecordReceiptTool {
    const NAME: &'static str = "record_receipt";

    type Error = RecordReceiptError;
    type Args = RecordReceiptArgs;
    type Output = RecordReceiptOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Persist a structured execution receipt before you finish. Use this before set_status(kind=\"outcome\") so the channel can audit sources used, artifacts created, commands run, and which claims are verified versus inferred.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "summary": { "type": "string" },
                    "sources_used": { "type": "array", "items": { "type": "string" } },
                    "artifacts_created": { "type": "array", "items": { "type": "string" } },
                    "commands_run": { "type": "array", "items": { "type": "string" } },
                    "verified_claims": { "type": "array", "items": { "type": "string" } },
                    "inferred_claims": { "type": "array", "items": { "type": "string" } },
                    "blockers": { "type": "array", "items": { "type": "string" } },
                    "wrong_source_detected": { "type": "boolean", "default": false }
                },
                "required": ["summary"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        if args.summary.trim().is_empty() {
            return Err(RecordReceiptError("summary cannot be empty".into()));
        }

        let receipt = WorkerOutcomeReceipt {
            summary: args.summary.trim().to_string(),
            sources_used: args.sources_used,
            artifacts_created: args.artifacts_created,
            commands_run: args.commands_run,
            verified_claims: args.verified_claims,
            inferred_claims: args.inferred_claims,
            blockers: args.blockers,
            wrong_source_detected: args.wrong_source_detected,
        };

        self.run_logger.log_worker_receipt(self.worker_id, &receipt);

        Ok(RecordReceiptOutput {
            success: true,
            worker_id: self.worker_id,
            receipt,
        })
    }
}
