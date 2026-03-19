//! Task creation tool for branch processes.

use crate::tasks::{CreateTaskInput, TaskPriority, TaskStatus, TaskStore, TaskSubtask};
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct TaskCreateTool {
    task_store: Arc<TaskStore>,
    agent_id: String,
    created_by: String,
    autonomy_require_approval: bool,
}

impl TaskCreateTool {
    pub fn new(
        task_store: Arc<TaskStore>,
        agent_id: impl Into<String>,
        created_by: impl Into<String>,
    ) -> Self {
        Self {
            task_store,
            agent_id: agent_id.into(),
            created_by: created_by.into(),
            autonomy_require_approval: false,
        }
    }

    pub fn with_autonomy_require_approval(mut self, require_approval: bool) -> Self {
        self.autonomy_require_approval = require_approval;
        self
    }
}

#[derive(Debug, thiserror::Error)]
#[error("task_create failed: {0}")]
pub struct TaskCreateError(String);

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskCreateArgs {
    pub title: String,
    pub description: Option<String>,
    #[serde(default = "default_priority")]
    pub priority: String,
    #[serde(default)]
    pub subtasks: Vec<String>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub risk_class: Option<String>,
    #[serde(default)]
    pub risk_reason: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub source_memory_ids: Vec<String>,
    #[serde(default)]
    pub origin_process: Option<String>,
    #[serde(default)]
    pub origin_worker_id: Option<String>,
    #[serde(default)]
    pub requires_approval: Option<bool>,
    #[serde(default)]
    pub auto_ready_allowed: Option<bool>,
}

fn default_priority() -> String {
    "medium".to_string()
}

fn resolve_autonomy_task_status(
    requested_status: Option<TaskStatus>,
    requires_approval: bool,
    auto_ready_allowed: bool,
) -> TaskStatus {
    if requires_approval {
        return TaskStatus::PendingApproval;
    }

    if auto_ready_allowed {
        return requested_status.unwrap_or(TaskStatus::Ready);
    }

    requested_status.unwrap_or(TaskStatus::Backlog)
}

#[derive(Debug, Serialize)]
pub struct TaskCreateOutput {
    pub success: bool,
    pub task_number: i64,
    pub status: String,
    pub message: String,
}

impl Tool for TaskCreateTool {
    const NAME: &'static str = "task_create";

    type Error = TaskCreateError;
    type Args = TaskCreateArgs;
    type Output = TaskCreateOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: crate::prompts::text::get("tools/task_create").to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Short task title" },
                    "description": { "type": "string", "description": "Optional detailed description" },
                    "priority": {
                        "type": "string",
                        "enum": crate::tasks::TaskPriority::ALL.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
                        "description": "Task priority"
                    },
                    "subtasks": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional checklist items"
                    },
                    "metadata": {
                        "type": "object",
                        "description": "Optional metadata object"
                    },
                    "status": {
                        "type": "string",
                        "enum": crate::tasks::TaskStatus::ALL.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                        "description": "Optional initial status"
                    },
                    "risk_class": {
                        "type": "string",
                        "enum": ["observe_only", "read_only_external", "local_reversible_write", "privileged_or_external_mutation"],
                        "description": "Optional autonomy risk class for the task."
                    },
                    "risk_reason": {
                        "type": "string",
                        "description": "Optional explanation for the autonomy risk decision."
                    },
                    "source": {
                        "type": "string",
                        "description": "Optional stable source label, such as cortex:autonomy."
                    },
                    "source_memory_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional source memory IDs backing this task."
                    },
                    "origin_process": {
                        "type": "string",
                        "description": "Optional origin process label, such as autonomy_loop."
                    },
                    "origin_worker_id": {
                        "type": "string",
                        "description": "Optional originating worker ID."
                    },
                    "requires_approval": {
                        "type": "boolean",
                        "description": "Whether this task should wait for approval before execution."
                    },
                    "auto_ready_allowed": {
                        "type": "boolean",
                        "description": "Whether this task is safe to move straight to ready."
                    }
                },
                "required": ["title"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let priority = TaskPriority::parse(&args.priority)
            .ok_or_else(|| TaskCreateError(format!("invalid priority: {}", args.priority)))?;
        let requested_status = match args.status.as_deref() {
            None => None,
            Some(value) => TaskStatus::parse(value)
                .ok_or_else(|| TaskCreateError(format!("invalid status: {value}")))
                .map(Some)?,
        };
        let risk_class = match args.risk_class.as_deref() {
            None => None,
            Some(value) => Some(
                crate::tasks::TaskRiskClass::parse(value)
                    .ok_or_else(|| TaskCreateError(format!("invalid risk_class: {value}")))?,
            ),
        };

        let subtasks = args
            .subtasks
            .into_iter()
            .map(|title| TaskSubtask {
                title,
                completed: false,
            })
            .collect::<Vec<_>>();

        let requires_approval = args.requires_approval.unwrap_or(
            self.autonomy_require_approval
                || matches!(
                    risk_class,
                    Some(crate::tasks::TaskRiskClass::PrivilegedOrExternalMutation)
                ),
        );
        let auto_ready_allowed = args.auto_ready_allowed.unwrap_or(
            !requires_approval
                && matches!(
                    risk_class,
                    Some(
                        crate::tasks::TaskRiskClass::ReadOnlyExternal
                            | crate::tasks::TaskRiskClass::LocalReversibleWrite
                    )
                ),
        );
        let status =
            resolve_autonomy_task_status(requested_status, requires_approval, auto_ready_allowed);

        let task = self
            .task_store
            .create(CreateTaskInput {
                agent_id: self.agent_id.clone(),
                title: args.title,
                description: args.description,
                status,
                priority,
                subtasks,
                metadata: args.metadata.unwrap_or_else(|| serde_json::json!({})),
                source_memory_id: None,
                created_by: self.created_by.clone(),
                autonomy: Some(crate::tasks::TaskAutonomyContext {
                    source: args.source,
                    source_memory_ids: args.source_memory_ids,
                    origin_process: args.origin_process,
                    origin_worker_id: args.origin_worker_id,
                    risk_class,
                    risk_reason: args.risk_reason,
                    requires_approval,
                    auto_ready_allowed,
                }),
            })
            .await
            .map_err(|error| TaskCreateError(format!("{error}")))?;

        Ok(TaskCreateOutput {
            success: true,
            task_number: task.task_number,
            status: task.status.to_string(),
            message: format!("Created task #{}: {}", task.task_number, task.title),
        })
    }
}
