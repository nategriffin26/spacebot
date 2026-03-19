//! Shared execution-receipt summaries for cron, APIs, and future orchestrators.

use crate::{WorkerOutcomeReceipt, WorkerTerminalReason};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptReference {
    pub worker_id: String,
    pub summary: String,
    #[serde(default)]
    pub terminal_reason: Option<WorkerTerminalReason>,
    #[serde(default)]
    pub retries_attempted: u32,
    #[serde(default)]
    pub merge_ready: bool,
    #[serde(default)]
    pub wrong_source_detected: bool,
    #[serde(default)]
    pub sources_used: Vec<String>,
    #[serde(default)]
    pub artifacts_created: Vec<String>,
}

pub fn summarize_worker_receipt(
    worker_id: impl Into<String>,
    receipt: &WorkerOutcomeReceipt,
) -> ReceiptReference {
    ReceiptReference {
        worker_id: worker_id.into(),
        summary: receipt.summary.clone(),
        terminal_reason: receipt.terminal_reason,
        retries_attempted: receipt.retries_attempted,
        merge_ready: receipt.merge_ready,
        wrong_source_detected: receipt.wrong_source_detected,
        sources_used: receipt.sources_used.clone(),
        artifacts_created: if receipt.artifact_outcomes.is_empty() {
            receipt.artifacts_created.clone()
        } else {
            receipt
                .artifact_outcomes
                .iter()
                .map(|artifact| artifact.path.clone())
                .collect()
        },
    }
}
