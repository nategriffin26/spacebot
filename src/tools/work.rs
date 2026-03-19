//! Higher-level channel work action tool.

use crate::agent::channel::ChannelState;
use crate::agent::channel_dispatch::{spawn_branch_and_spawn_from_state, spawn_swarm_from_state};
use crate::tooling::{SwarmCompletionPolicy, SwarmPlan, SwarmShardSpec};
use crate::tools::TurnActionTracker;
use crate::{BranchId, WorkerId};

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct WorkTool {
    state: ChannelState,
    turn_action_tracker: TurnActionTracker,
}

impl WorkTool {
    pub fn new(state: ChannelState, turn_action_tracker: TurnActionTracker) -> Self {
        Self {
            state,
            turn_action_tracker,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("work failed: {0}")]
pub struct WorkError(String);

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WorkArgs {
    /// One of: start, continue, inspect, cancel.
    pub action: String,
    /// Work description for `start`.
    #[serde(default)]
    pub task: Option<String>,
    /// Follow-up message for `continue`.
    #[serde(default)]
    pub message: Option<String>,
    /// Optional worker identifier. Strongly preferred when more than one active worker exists.
    #[serde(default)]
    pub worker_id: Option<String>,
    /// Optional branch identifier for cancellation.
    #[serde(default)]
    pub branch_id: Option<String>,
    /// Whether a started worker should stay interactive.
    #[serde(default)]
    pub interactive: bool,
    /// Suggest worker skills for `start`.
    #[serde(default)]
    pub suggested_skills: Vec<String>,
    /// Inject topic context for `start`.
    #[serde(default)]
    pub topic_ids: Vec<String>,
    /// Force preparation before execution even if the runtime would otherwise
    /// choose a direct worker spawn.
    #[serde(default)]
    pub prefer_preparation: bool,
    /// Optional shard list for orchestrated multi-worker execution.
    #[serde(default)]
    pub shards: Vec<WorkShardArgs>,
    /// Optional cancellation reason.
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WorkShardArgs {
    pub key: String,
    pub task: String,
    #[serde(default)]
    pub suggested_skills: Vec<String>,
    #[serde(default)]
    pub topic_ids: Vec<String>,
    #[serde(default)]
    pub interactive: bool,
    #[serde(default = "default_required_shard")]
    pub required: bool,
}

const fn default_required_shard() -> bool {
    true
}

#[derive(Debug, Serialize)]
pub struct WorkOutput {
    pub success: bool,
    pub action: String,
    pub dispatch: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_id: Option<WorkerId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_id: Option<BranchId>,
}

impl Tool for WorkTool {
    const NAME: &'static str = "work";

    type Error = WorkError;
    type Args = WorkArgs;
    type Output = WorkOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: crate::prompts::text::get("tools/work").to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["start", "continue", "inspect", "cancel"],
                        "description": "Intent-level work action."
                    },
                    "task": {
                        "type": "string",
                        "description": "Required for start. Describe the work to begin."
                    },
                    "message": {
                        "type": "string",
                        "description": "Required for continue. Follow-up or steering message for the active worker."
                    },
                    "worker_id": {
                        "type": "string",
                        "description": "Optional worker ID for continue, inspect, or cancel."
                    },
                    "branch_id": {
                        "type": "string",
                        "description": "Optional branch ID for cancel."
                    },
                    "interactive": {
                        "type": "boolean",
                        "default": false,
                        "description": "If true, a started worker stays alive for follow-up input."
                    },
                    "suggested_skills": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "topic_ids": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "prefer_preparation": {
                        "type": "boolean",
                        "default": false,
                        "description": "Force a prep branch before execution."
                    },
                    "shards": {
                        "type": "array",
                        "description": "Optional multi-lane worker plan for one coordinated swarm run.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "key": { "type": "string" },
                                "task": { "type": "string" },
                                "suggested_skills": { "type": "array", "items": { "type": "string" } },
                                "topic_ids": { "type": "array", "items": { "type": "string" } },
                                "interactive": { "type": "boolean", "default": false },
                                "required": { "type": "boolean", "default": true }
                            },
                            "required": ["key", "task"]
                        }
                    },
                    "reason": {
                        "type": "string",
                        "description": "Optional cancel reason."
                    }
                },
                "required": ["action"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        match args.action.as_str() {
            "start" => self.start_work(args).await,
            "continue" => self.continue_work(args).await,
            "inspect" => self.inspect_work(args).await,
            "cancel" => self.cancel_work(args).await,
            other => Err(WorkError(format!("unknown action: {other}"))),
        }
    }
}

impl WorkTool {
    async fn start_work(&self, args: WorkArgs) -> Result<WorkOutput, WorkError> {
        if !args.shards.is_empty() {
            return self.start_swarm(args).await;
        }

        let task = args
            .task
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| WorkError("start requires a non-empty task".to_string()))?;

        let current_context = self.state.current_request_context().await;
        let task_class = current_context
            .as_ref()
            .and_then(|context| context.task_class);
        let has_explicit_sources = current_context
            .as_ref()
            .is_some_and(|context| context.source_manifest.has_explicit_sources());
        let lower = task.to_ascii_lowercase();
        let prefer_preparation = args.prefer_preparation
            || has_explicit_sources
            || matches!(
                task_class,
                Some(
                    crate::TaskClass::SourceVerification
                        | crate::TaskClass::ArtifactGroundedCopy
                        | crate::TaskClass::CapabilityDiagnosis
                        | crate::TaskClass::MultiLaneSynthesis
                )
            )
            || lower.contains("review")
            || lower.contains("inspect")
            || lower.contains("verify")
            || lower.contains("diff")
            || lower.contains("compare");

        if prefer_preparation {
            let branch_id = spawn_branch_and_spawn_from_state(
                &self.state,
                task,
                args.interactive,
                &args
                    .suggested_skills
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                &args.topic_ids,
            )
            .await
            .map_err(|error| WorkError(error.to_string()))?;
            self.turn_action_tracker
                .mark_action("work:start:branch_and_spawn");

            return Ok(WorkOutput {
                success: true,
                action: "start".to_string(),
                dispatch: "branch_and_spawn".to_string(),
                message: format!(
                    "Started prepared execution for: {task}. Prep branch {branch_id} will enrich the handoff and start the worker automatically."
                ),
                worker_id: None,
                branch_id: Some(branch_id),
            });
        }

        let worker_id = crate::agent::channel_dispatch::spawn_worker_from_state(
            &self.state,
            task,
            args.interactive,
            &args
                .suggested_skills
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            &args.topic_ids,
        )
        .await
        .map_err(|error| WorkError(error.to_string()))?;
        self.turn_action_tracker
            .mark_action("work:start:spawn_worker");

        Ok(WorkOutput {
            success: true,
            action: "start".to_string(),
            dispatch: "spawn_worker".to_string(),
            message: if args.interactive {
                format!("Started interactive worker {worker_id} for: {task}")
            } else {
                format!("Started worker {worker_id} for: {task}")
            },
            worker_id: Some(worker_id),
            branch_id: None,
        })
    }

    async fn start_swarm(&self, args: WorkArgs) -> Result<WorkOutput, WorkError> {
        let task = args
            .task
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("multi-lane work");

        let shards = args
            .shards
            .into_iter()
            .map(|shard| {
                if shard.key.trim().is_empty() || shard.task.trim().is_empty() {
                    return Err(WorkError(
                        "each shard requires a non-empty key and task".to_string(),
                    ));
                }

                Ok(SwarmShardSpec {
                    key: shard.key.trim().to_string(),
                    task: shard.task.trim().to_string(),
                    suggested_skills: shard.suggested_skills,
                    topic_ids: shard.topic_ids,
                    interactive: shard.interactive,
                    required: shard.required,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let plan = SwarmPlan {
            summary: task.to_string(),
            shards,
            completion_policy: SwarmCompletionPolicy::default(),
        };

        let dispatch = spawn_swarm_from_state(&self.state, &plan)
            .await
            .map_err(|error| WorkError(error.to_string()))?;
        self.turn_action_tracker.mark_action("work:start:swarm");

        Ok(WorkOutput {
            success: true,
            action: "start".to_string(),
            dispatch: "swarm".to_string(),
            message: format!(
                "Started swarm run for {} shard(s){}.",
                dispatch.worker_ids.len(),
                dispatch
                    .request_group_id
                    .as_ref()
                    .map(|value| format!(" in request group {value}"))
                    .unwrap_or_default()
            ),
            worker_id: dispatch.worker_ids.first().copied(),
            branch_id: None,
        })
    }

    async fn continue_work(&self, args: WorkArgs) -> Result<WorkOutput, WorkError> {
        let message = args
            .message
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| WorkError("continue requires a non-empty message".to_string()))?;
        let worker_id = self.resolve_worker_id(args.worker_id.as_deref()).await?;

        let worker_is_idle = {
            let status = self.state.status_block.read().await;
            status
                .active_workers
                .iter()
                .find(|worker| worker.id == worker_id)
                .map(|worker| worker.status == "idle")
                .ok_or_else(|| {
                    WorkError(format!(
                        "worker {worker_id} is not active; inspect before continuing"
                    ))
                })?
        };

        let dispatch = if worker_is_idle {
            let inputs = self.state.worker_inputs.read().await;
            let input_tx = inputs.get(&worker_id).cloned().ok_or_else(|| {
                WorkError(format!(
                    "worker {worker_id} is idle but not accepting follow-up input"
                ))
            })?;
            drop(inputs);
            input_tx.send(message.to_string()).await.map_err(|_| {
                WorkError(format!(
                    "worker {worker_id} stopped before follow-up could be delivered"
                ))
            })?;
            "route_input"
        } else {
            let injections = self.state.worker_injections.read().await;
            let inject_tx = injections.get(&worker_id).cloned().ok_or_else(|| {
                WorkError(format!(
                    "worker {worker_id} does not support mid-flight context injection"
                ))
            })?;
            drop(injections);
            inject_tx.send(message.to_string()).await.map_err(|_| {
                WorkError(format!(
                    "worker {worker_id} stopped before follow-up could be injected"
                ))
            })?;
            "route_injection"
        };

        self.turn_action_tracker
            .mark_action(format!("work:continue:{dispatch}"));
        Ok(WorkOutput {
            success: true,
            action: "continue".to_string(),
            dispatch: dispatch.to_string(),
            message: format!("Delivered follow-up to worker {worker_id}."),
            worker_id: Some(worker_id),
            branch_id: None,
        })
    }

    async fn inspect_work(&self, args: WorkArgs) -> Result<WorkOutput, WorkError> {
        if let Some(worker_id) = args.worker_id.as_deref() {
            let worker_id = worker_id
                .parse::<WorkerId>()
                .map_err(|error| WorkError(format!("invalid worker_id: {error}")))?;
            let detail = self
                .state
                .process_run_logger
                .get_worker_detail(&self.state.deps.agent_id, &worker_id.to_string())
                .await
                .map_err(|error| WorkError(error.to_string()))?
                .ok_or_else(|| WorkError(format!("no worker found with ID {worker_id}")))?;
            self.turn_action_tracker.mark_action("work:inspect:worker");
            let receipt_state = if detail.outcome_receipt_json.is_some() {
                "receipt present"
            } else {
                "receipt missing"
            };
            let summary = format!(
                "Worker {worker_id}: status={}, interactive={}, tool_calls={}, {receipt_state}",
                detail.status, detail.interactive, detail.tool_calls
            );
            return Ok(WorkOutput {
                success: true,
                action: "inspect".to_string(),
                dispatch: "worker_detail".to_string(),
                message: summary,
                worker_id: Some(worker_id),
                branch_id: None,
            });
        }

        let active_summary = {
            let status = self.state.status_block.read().await;
            if status.active_workers.is_empty() {
                "No active workers.".to_string()
            } else {
                status
                    .active_workers
                    .iter()
                    .map(|worker| {
                        format!(
                            "{} ({}, interactive={}, status={})",
                            worker.id, worker.task, worker.interactive, worker.status
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            }
        };
        self.turn_action_tracker.mark_action("work:inspect:list");

        Ok(WorkOutput {
            success: true,
            action: "inspect".to_string(),
            dispatch: "status_block".to_string(),
            message: active_summary,
            worker_id: None,
            branch_id: None,
        })
    }

    async fn cancel_work(&self, args: WorkArgs) -> Result<WorkOutput, WorkError> {
        let reason = args
            .reason
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("cancelled by work tool");

        if let Some(worker_id) = args.worker_id.as_deref() {
            let worker_id = worker_id
                .parse::<WorkerId>()
                .map_err(|error| WorkError(format!("invalid worker_id: {error}")))?;
            self.state
                .cancel_worker_with_reason(worker_id, reason)
                .await
                .map_err(WorkError)?;
            self.turn_action_tracker.mark_action("work:cancel:worker");
            return Ok(WorkOutput {
                success: true,
                action: "cancel".to_string(),
                dispatch: "cancel_worker".to_string(),
                message: format!("Cancelled worker {worker_id}."),
                worker_id: Some(worker_id),
                branch_id: None,
            });
        }

        if let Some(branch_id) = args.branch_id.as_deref() {
            let branch_id = branch_id
                .parse::<BranchId>()
                .map_err(|error| WorkError(format!("invalid branch_id: {error}")))?;
            self.state
                .cancel_branch_with_reason(branch_id, reason)
                .await
                .map_err(WorkError)?;
            self.turn_action_tracker.mark_action("work:cancel:branch");
            return Ok(WorkOutput {
                success: true,
                action: "cancel".to_string(),
                dispatch: "cancel_branch".to_string(),
                message: format!("Cancelled branch {branch_id}."),
                worker_id: None,
                branch_id: Some(branch_id),
            });
        }

        let worker_id = self.resolve_worker_id(None).await?;
        self.state
            .cancel_worker_with_reason(worker_id, reason)
            .await
            .map_err(WorkError)?;
        self.turn_action_tracker
            .mark_action("work:cancel:implicit_worker");
        Ok(WorkOutput {
            success: true,
            action: "cancel".to_string(),
            dispatch: "cancel_worker".to_string(),
            message: format!("Cancelled worker {worker_id}."),
            worker_id: Some(worker_id),
            branch_id: None,
        })
    }

    async fn resolve_worker_id(
        &self,
        explicit_worker_id: Option<&str>,
    ) -> Result<WorkerId, WorkError> {
        if let Some(worker_id) = explicit_worker_id {
            return worker_id
                .parse::<WorkerId>()
                .map_err(|error| WorkError(format!("invalid worker_id: {error}")));
        }

        let status = self.state.status_block.read().await;
        let interactive_workers = status
            .active_workers
            .iter()
            .filter(|worker| worker.interactive)
            .map(|worker| worker.id)
            .collect::<Vec<_>>();

        match interactive_workers.as_slice() {
            [worker_id] => Ok(*worker_id),
            [] => Err(WorkError(
                "no active interactive worker found; supply worker_id or start work first"
                    .to_string(),
            )),
            _ => Err(WorkError(
                "multiple interactive workers are active; supply worker_id explicitly".to_string(),
            )),
        }
    }
}
