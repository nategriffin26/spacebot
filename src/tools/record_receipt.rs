//! Structured worker outcome receipts persisted alongside worker runs.

use crate::conversation::history::ProcessRunLogger;
use crate::{
    ArtifactOutcome, SourceManifest, WorkerId, WorkerOutcomeReceipt, WorkerTerminalReason,
};
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
    /// Short restatement of the original task so later inspection can verify scope.
    #[serde(default)]
    pub original_task_summary: Option<String>,
    /// Explicit sources actually used for the work.
    #[serde(default)]
    pub sources_used: Vec<String>,
    /// Files, artifacts, or outputs created during execution.
    #[serde(default)]
    pub artifacts_created: Vec<String>,
    /// Structured artifact results when delivery/runtime facts are known.
    #[serde(default)]
    pub artifact_outcomes: Vec<ArtifactOutcome>,
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
    /// Requirements that were completed.
    #[serde(default)]
    pub requirements_completed: Vec<String>,
    /// Requirements that could not be completed.
    #[serde(default)]
    pub requirements_blocked: Vec<String>,
    /// Terminal classification for this run.
    #[serde(default)]
    pub terminal_reason: Option<WorkerTerminalReason>,
    /// Number of retries the worker attempted before this receipt.
    #[serde(default)]
    pub retries_attempted: u32,
    /// Concrete remediations attempted during the run.
    #[serde(default)]
    pub remediations_attempted: Vec<String>,
    /// Whether this output is merge-ready for a downstream synthesis/orchestrator.
    #[serde(default)]
    pub merge_ready: bool,
    /// Source manifest the worker actually honored, when relevant.
    #[serde(default)]
    pub source_manifest_used: Option<SourceManifest>,
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
                    "original_task_summary": { "type": "string" },
                    "sources_used": { "type": "array", "items": { "type": "string" } },
                    "artifacts_created": { "type": "array", "items": { "type": "string" } },
                    "artifact_outcomes": { "type": "array", "items": { "type": "object" } },
                    "commands_run": { "type": "array", "items": { "type": "string" } },
                    "verified_claims": { "type": "array", "items": { "type": "string" } },
                    "inferred_claims": { "type": "array", "items": { "type": "string" } },
                    "blockers": { "type": "array", "items": { "type": "string" } },
                    "requirements_completed": { "type": "array", "items": { "type": "string" } },
                    "requirements_blocked": { "type": "array", "items": { "type": "string" } },
                    "terminal_reason": {
                        "type": "string",
                        "enum": ["success", "partial_success", "blocked", "failed_transient", "failed_deterministic", "cancelled", "suspicious_completion"]
                    },
                    "retries_attempted": { "type": "integer", "minimum": 0, "default": 0 },
                    "remediations_attempted": { "type": "array", "items": { "type": "string" } },
                    "merge_ready": { "type": "boolean", "default": false },
                    "source_manifest_used": { "type": "object" },
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
            original_task_summary: args
                .original_task_summary
                .map(|value| value.trim().to_string()),
            sources_used: args.sources_used,
            artifacts_created: args.artifacts_created,
            artifact_outcomes: args.artifact_outcomes,
            commands_run: args.commands_run,
            verified_claims: args.verified_claims,
            inferred_claims: args.inferred_claims,
            requirements_completed: args.requirements_completed,
            requirements_blocked: args.requirements_blocked,
            blockers: args.blockers,
            terminal_reason: args.terminal_reason,
            retries_attempted: args.retries_attempted,
            remediations_attempted: args.remediations_attempted,
            merge_ready: args.merge_ready,
            source_manifest_used: args.source_manifest_used,
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
