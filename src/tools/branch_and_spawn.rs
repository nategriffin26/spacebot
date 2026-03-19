//! Branch-and-spawn tool for channel-side prepared execution handoff.

use crate::BranchId;
use crate::agent::channel::ChannelState;
use crate::agent::channel_dispatch::spawn_branch_and_spawn_from_state;
use crate::tools::TurnActionTracker;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Tool for spawning a prep branch that will start a worker when ready.
#[derive(Debug, Clone)]
pub struct BranchAndSpawnTool {
    state: ChannelState,
    turn_action_tracker: TurnActionTracker,
}

impl BranchAndSpawnTool {
    /// Create a new branch-and-spawn tool with access to channel state.
    pub fn new(state: ChannelState, turn_action_tracker: TurnActionTracker) -> Self {
        Self {
            state,
            turn_action_tracker,
        }
    }
}

/// Error type for branch-and-spawn tool.
#[derive(Debug, thiserror::Error)]
#[error("Branch-and-spawn failed: {0}")]
pub struct BranchAndSpawnError(String);

/// Arguments for branch-and-spawn tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BranchAndSpawnArgs {
    /// The work to prepare and then execute.
    pub task: String,
    /// Whether the eventual worker should remain interactive.
    #[serde(default)]
    pub interactive: bool,
    /// Optional skill names to suggest to the eventual worker.
    #[serde(default)]
    pub suggested_skills: Vec<String>,
    /// Topic IDs to inject into the eventual worker task.
    #[serde(default)]
    pub topic_ids: Vec<String>,
}

/// Output from branch-and-spawn tool.
#[derive(Debug, Serialize)]
pub struct BranchAndSpawnOutput {
    /// The ID of the preparation branch.
    pub branch_id: BranchId,
    /// Whether the branch was started successfully.
    pub spawned: bool,
    /// Status message.
    pub message: String,
}

impl Tool for BranchAndSpawnTool {
    const NAME: &'static str = "branch_and_spawn";

    type Error = BranchAndSpawnError;
    type Args = BranchAndSpawnArgs;
    type Output = BranchAndSpawnOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: crate::prompts::text::get("tools/branch_and_spawn").to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "task": {
                        "type": "string",
                        "description": "The work to prepare and then execute. Use this when memory recall, docs lookup, transcript inspection, or source-aware task shaping would materially improve the worker handoff."
                    },
                    "interactive": {
                        "type": "boolean",
                        "default": false,
                        "description": "If true, the eventual worker remains alive for follow-up messages."
                    },
                    "suggested_skills": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Skill names to flag as recommended for the eventual worker."
                    },
                    "topic_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Topic IDs to inject as context into the eventual worker."
                    }
                },
                "required": ["task"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let readiness = self.state.deps.runtime_config.work_readiness();
        let branch_id = spawn_branch_and_spawn_from_state(
            &self.state,
            &args.task,
            args.interactive,
            &args
                .suggested_skills
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            &args.topic_ids,
        )
        .await
        .map_err(|error| BranchAndSpawnError(error.to_string()))?;
        self.turn_action_tracker.mark_action("branch_and_spawn");

        let readiness_note = if readiness.ready {
            String::new()
        } else {
            let reason = readiness
                .reason
                .map(|value| value.as_str())
                .unwrap_or("unknown");
            format!(
                " Readiness note: warmup is not fully ready ({reason}, state: {:?}); a warmup pass may already be running or was queued in the background.",
                readiness.warmup_state
            )
        };

        Ok(BranchAndSpawnOutput {
            branch_id,
            spawned: true,
            message: format!(
                "Preparation branch {branch_id} spawned for: {}. It will shape the worker handoff and start the worker automatically when ready.{}",
                args.task, readiness_note
            ),
        })
    }
}
