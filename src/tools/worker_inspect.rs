//! Worker transcript inspection tool for branches.
//!
//! Allows a branch to retrieve the full transcript of a completed worker run,
//! or list recent worker runs to find the right one.

use crate::conversation::history::ProcessRunLogger;
use crate::conversation::worker_transcript;

use super::truncate_utf8_ellipsis;

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Tool for inspecting worker run transcripts.
#[derive(Debug, Clone)]
pub struct WorkerInspectTool {
    run_logger: ProcessRunLogger,
    agent_id: String,
}

impl WorkerInspectTool {
    pub fn new(run_logger: ProcessRunLogger, agent_id: String) -> Self {
        Self {
            run_logger,
            agent_id,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Worker inspect failed: {0}")]
pub struct WorkerInspectError(String);

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WorkerInspectArgs {
    /// The worker ID to inspect. Omit to list recent worker runs.
    #[serde(default)]
    pub worker_id: Option<String>,
    /// Maximum number of worker runs to list (default 10, max 50). Only used when listing.
    #[serde(default = "default_list_limit")]
    pub limit: i64,
}

fn default_list_limit() -> i64 {
    10
}

#[derive(Debug, Serialize)]
pub struct WorkerInspectOutput {
    pub action: String,
    pub summary: String,
}

impl Tool for WorkerInspectTool {
    const NAME: &'static str = "worker_inspect";

    type Error = WorkerInspectError;
    type Args = WorkerInspectArgs;
    type Output = WorkerInspectOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: crate::prompts::text::get("tools/worker_inspect").to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "worker_id": {
                        "type": "string",
                        "description": "UUID of the worker run to inspect. Omit to list recent workers."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 50,
                        "default": 10,
                        "description": "Number of recent workers to list (1-50). Only used when worker_id is omitted."
                    }
                }
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let Some(worker_id) = args.worker_id else {
            return self.list_workers(args.limit).await;
        };

        let detail = self
            .run_logger
            .get_worker_detail(&self.agent_id, &worker_id)
            .await
            .map_err(|e| WorkerInspectError(format!("Failed to query worker: {e}")))?
            .ok_or_else(|| WorkerInspectError(format!("No worker found with ID {worker_id}")))?;

        let mut summary = format!(
            "## Worker {}\n\n**Task:** {}\n**Status:** {}\n**Started:** {}\n",
            detail.id, detail.task, detail.status, detail.started_at,
        );

        if let Some(completed_at) = &detail.completed_at {
            summary.push_str(&format!("**Completed:** {completed_at}\n"));
        }

        if let Some(result) = &detail.result {
            summary.push_str(&format!("\n### Result\n\n{result}\n"));
        }

        if let Some(receipt_json) = &detail.outcome_receipt_json {
            match serde_json::from_str::<crate::WorkerOutcomeReceipt>(receipt_json) {
                Ok(receipt) => {
                    summary.push_str("\n### Receipt\n\n");
                    summary.push_str(&format!("**Summary:** {}\n\n", receipt.summary));
                    if let Some(task_summary) = &receipt.original_task_summary {
                        summary.push_str(&format!("**Original task:** {task_summary}\n\n"));
                    }
                    if let Some(terminal_reason) = receipt.terminal_reason {
                        summary.push_str(&format!(
                            "**Terminal reason:** {}\n\n",
                            terminal_reason.as_str()
                        ));
                    }
                    if !receipt.sources_used.is_empty() {
                        summary.push_str(&format!(
                            "**Sources used:** {}\n\n",
                            receipt.sources_used.join(", ")
                        ));
                    }
                    if !receipt.artifacts_created.is_empty() {
                        summary.push_str(&format!(
                            "**Artifacts created:** {}\n\n",
                            receipt.artifacts_created.join(", ")
                        ));
                    }
                    if !receipt.artifact_outcomes.is_empty() {
                        summary.push_str("**Artifact outcomes:**\n");
                        for artifact in &receipt.artifact_outcomes {
                            summary.push_str(&format!("- {}\n", artifact.path));
                        }
                        summary.push('\n');
                    }
                    if !receipt.commands_run.is_empty() {
                        summary.push_str("**Commands run:**\n");
                        for command in &receipt.commands_run {
                            summary.push_str(&format!("- `{command}`\n"));
                        }
                        summary.push('\n');
                    }
                    if !receipt.verified_claims.is_empty() {
                        summary.push_str("**Verified claims:**\n");
                        for claim in &receipt.verified_claims {
                            summary.push_str(&format!("- {claim}\n"));
                        }
                        summary.push('\n');
                    }
                    if !receipt.inferred_claims.is_empty() {
                        summary.push_str("**Inferred claims:**\n");
                        for claim in &receipt.inferred_claims {
                            summary.push_str(&format!("- {claim}\n"));
                        }
                        summary.push('\n');
                    }
                    if !receipt.blockers.is_empty() {
                        summary.push_str("**Blockers:**\n");
                        for blocker in &receipt.blockers {
                            summary.push_str(&format!("- {blocker}\n"));
                        }
                        summary.push('\n');
                    }
                    if !receipt.requirements_completed.is_empty() {
                        summary.push_str("**Requirements completed:**\n");
                        for item in &receipt.requirements_completed {
                            summary.push_str(&format!("- {item}\n"));
                        }
                        summary.push('\n');
                    }
                    if !receipt.requirements_blocked.is_empty() {
                        summary.push_str("**Requirements blocked:**\n");
                        for item in &receipt.requirements_blocked {
                            summary.push_str(&format!("- {item}\n"));
                        }
                        summary.push('\n');
                    }
                    if receipt.wrong_source_detected {
                        summary.push_str("**Wrong source detected:** yes\n\n");
                    }
                }
                Err(error) => {
                    summary.push_str(&format!(
                        "\n*Outcome receipt could not be parsed: {error}*\n"
                    ));
                }
            }
        }

        if let Some(blob) = &detail.transcript_blob {
            match worker_transcript::deserialize_transcript(blob) {
                Ok(steps) => {
                    summary.push_str(&format!("\n### Transcript ({} steps)\n\n", steps.len()));
                    for step in &steps {
                        match step {
                            worker_transcript::TranscriptStep::Action { content } => {
                                for item in content {
                                    match item {
                                        worker_transcript::ActionContent::Text { text } => {
                                            summary.push_str(&format!("**Agent:** {text}\n\n"));
                                        }
                                        worker_transcript::ActionContent::ToolCall {
                                            name,
                                            args,
                                            ..
                                        } => {
                                            summary.push_str(&format!(
                                                "**Tool call:** `{name}`\n```\n{args}\n```\n\n"
                                            ));
                                        }
                                    }
                                }
                            }
                            worker_transcript::TranscriptStep::UserText { text } => {
                                summary.push_str(&format!("**User:** {text}\n\n"));
                            }
                            worker_transcript::TranscriptStep::ToolResult {
                                name, text, ..
                            } => {
                                let label = if name.is_empty() { "tool" } else { name };
                                let display = if text.len() > 500 {
                                    format!(
                                        "{}\n[truncated, {} bytes total]",
                                        truncate_utf8_ellipsis(text, 500),
                                        text.len()
                                    )
                                } else {
                                    text.clone()
                                };
                                summary.push_str(&format!(
                                    "**Result ({label}):**\n```\n{display}\n```\n\n"
                                ));
                            }
                        }
                    }
                }
                Err(error) => {
                    summary.push_str(&format!(
                        "\n*Transcript could not be decompressed: {error}*\n"
                    ));
                }
            }
        } else {
            summary.push_str("\n*No transcript available for this worker.*\n");
        }

        if detail.status == "failed"
            && detail
                .result
                .as_deref()
                .is_some_and(|result| result.contains("Spacebot restarted before completion"))
        {
            summary.push_str(&format!(
                "\n### Recovery\n\nSuggested prompt:\n\n> Inspect worker `{}` and continue the interrupted task from the saved transcript instead of starting over.\n",
                detail.id
            ));
        }

        Ok(WorkerInspectOutput {
            action: "inspect".to_string(),
            summary,
        })
    }
}

impl WorkerInspectTool {
    async fn list_workers(&self, limit: i64) -> Result<WorkerInspectOutput, WorkerInspectError> {
        let limit = limit.clamp(1, 50);
        let (rows, total) = self
            .run_logger
            .list_worker_runs(&self.agent_id, limit, 0, None)
            .await
            .map_err(|e| WorkerInspectError(format!("Failed to list workers: {e}")))?;

        if rows.is_empty() {
            return Ok(WorkerInspectOutput {
                action: "list".to_string(),
                summary: "No worker runs found.".to_string(),
            });
        }

        let mut summary = format!("## Recent Workers ({} of {total})\n\n", rows.len());

        for row in &rows {
            let status_marker = match row.status.as_str() {
                "running" => "[running]",
                "done" => "[done]",
                "failed" => "[failed]",
                "cancelled" => "[cancelled]",
                _ => "[-]",
            };
            summary.push_str(&format!(
                "{status_marker} `{}` — {} ({})\n",
                row.id, row.task, row.status,
            ));
            if let Some(channel) = &row.channel_name {
                summary.push_str(&format!("  Channel: {channel}\n"));
            }
            summary.push_str(&format!(
                "  Started: {} | {} tool calls{}\n\n",
                row.started_at,
                row.tool_calls,
                if row.has_transcript {
                    " | transcript available"
                } else {
                    ""
                },
            ));
        }

        summary.push_str("Use `worker_inspect` with a `worker_id` to view the full transcript.");

        Ok(WorkerInspectOutput {
            action: "list".to_string(),
            summary,
        })
    }
}
