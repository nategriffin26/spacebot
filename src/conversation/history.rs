//! Conversation message persistence (SQLite).

use crate::tooling::{ReceiptReference, SwarmMergeSummary, merge_receipt_references};
use crate::{
    BranchId, ChannelId, RequestGroupMode, RequestGroupState, SourceManifest, TaskClass, WorkerId,
    WorkerOutcomeReceipt,
};

use serde::{Deserialize, Serialize};
use sqlx::{Row as _, SqlitePool};
use std::collections::HashMap;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct InterruptedWorkerRow {
    pub id: String,
    pub channel_id: Option<String>,
    pub task: String,
    pub worker_type: String,
    pub started_at: String,
    pub directory: Option<String>,
    pub transcript: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct RequestGroupRecord {
    pub id: String,
    pub agent_id: String,
    pub channel_id: String,
    pub origin_message_id: Option<String>,
    pub origin_user_text: String,
    pub mode: String,
    pub state: String,
    pub task_class: Option<String>,
    pub source_manifest_json: Option<String>,
    pub required_receipts_json: Option<String>,
    pub completion_policy_json: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestGroupBranchRecord {
    pub id: String,
    pub description: String,
    pub conclusion: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestGroupWorkerRecord {
    pub id: String,
    pub task: String,
    pub status: String,
    pub completed_at: Option<String>,
    pub worker_type: String,
    pub receipt: Option<ReceiptReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestGroupDetail {
    pub request_group: RequestGroupRecord,
    pub branches: Vec<RequestGroupBranchRecord>,
    pub workers: Vec<RequestGroupWorkerRecord>,
    pub merged: SwarmMergeSummary,
}

#[derive(Debug, Clone)]
pub struct ArtifactDeliveryUpdate {
    pub path: String,
    pub delivery_status: String,
    pub delivery_target: Option<String>,
}

fn summarize_request_group_receipt(
    worker_id: &str,
    receipt: &WorkerOutcomeReceipt,
) -> ReceiptReference {
    crate::tooling::summarize_worker_receipt(worker_id.to_string(), receipt)
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct OperatorPolicyRecord {
    pub id: String,
    pub agent_id: String,
    pub policy_key: String,
    pub content: String,
    pub priority: i64,
    pub created_at: String,
    pub updated_at: String,
    pub last_reaffirmed_at: Option<String>,
    pub source_channel_id: Option<String>,
}

/// Persists conversation messages (user and assistant) to SQLite.
///
/// All write methods are fire-and-forget — they spawn a tokio task and return
/// immediately so the caller never blocks on a DB write.
#[derive(Debug, Clone)]
pub struct ConversationLogger {
    pool: SqlitePool,
}

/// A persisted conversation message.
#[derive(Debug, Clone)]
pub struct ConversationMessage {
    pub id: String,
    pub channel_id: String,
    pub role: String,
    pub sender_name: Option<String>,
    pub sender_id: Option<String>,
    pub content: String,
    pub metadata: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl ConversationLogger {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Log a user message. Fire-and-forget.
    pub fn log_user_message(
        &self,
        channel_id: &ChannelId,
        sender_name: &str,
        sender_id: &str,
        content: &str,
        metadata: &HashMap<String, serde_json::Value>,
    ) {
        let pool = self.pool.clone();
        let id = uuid::Uuid::new_v4().to_string();
        let channel_id = channel_id.to_string();
        let sender_name = sender_name.to_string();
        let sender_id = sender_id.to_string();
        let content = content.to_string();
        let metadata_json = serde_json::to_string(metadata).ok();

        tokio::spawn(async move {
            if let Err(error) = sqlx::query(
                "INSERT INTO conversation_messages (id, channel_id, role, sender_name, sender_id, content, metadata) \
                 VALUES (?, ?, 'user', ?, ?, ?, ?)"
            )
            .bind(&id)
            .bind(&channel_id)
            .bind(&sender_name)
            .bind(&sender_id)
            .bind(&content)
            .bind(&metadata_json)
            .execute(&pool)
            .await
            {
                tracing::warn!(%error, "failed to persist user message");
            }
        });
    }

    /// Log a bot (assistant) message. Fire-and-forget.
    pub fn log_bot_message(&self, channel_id: &ChannelId, content: &str) {
        self.log_bot_message_with_name(channel_id, content, None);
    }

    /// Log a system message (e.g. task delegation audit record). Fire-and-forget.
    ///
    /// System messages are persisted with role `"system"` and are not fed to any
    /// LLM context window. They exist purely for UI display in link channel
    /// timelines and audit logs.
    pub fn log_system_message(&self, channel_id: &str, content: &str) {
        let pool = self.pool.clone();
        let id = uuid::Uuid::new_v4().to_string();
        let channel_id = channel_id.to_string();
        let content = content.to_string();

        tokio::spawn(async move {
            if let Err(error) = sqlx::query(
                "INSERT INTO conversation_messages (id, channel_id, role, sender_name, content) \
                 VALUES (?, ?, 'system', 'system', ?)",
            )
            .bind(&id)
            .bind(&channel_id)
            .bind(&content)
            .execute(&pool)
            .await
            {
                tracing::warn!(%error, %channel_id, "failed to persist system message");
            }
        });
    }

    /// Log a bot (assistant) message with an agent display name. Fire-and-forget.
    pub fn log_bot_message_with_name(
        &self,
        channel_id: &ChannelId,
        content: &str,
        sender_name: Option<&str>,
    ) {
        let pool = self.pool.clone();
        let id = uuid::Uuid::new_v4().to_string();
        let channel_id = channel_id.to_string();
        let content = content.to_string();
        let sender_name = sender_name.map(String::from);

        tokio::spawn(async move {
            if let Err(error) = sqlx::query(
                "INSERT INTO conversation_messages (id, channel_id, role, sender_name, content) \
                 VALUES (?, ?, 'assistant', ?, ?)",
            )
            .bind(&id)
            .bind(&channel_id)
            .bind(&sender_name)
            .bind(&content)
            .execute(&pool)
            .await
            {
                tracing::warn!(%error, "failed to persist bot message");
            }
        });
    }

    /// Load recent messages for a channel (oldest first).
    pub async fn load_recent(
        &self,
        channel_id: &ChannelId,
        limit: i64,
    ) -> crate::error::Result<Vec<ConversationMessage>> {
        let rows = sqlx::query(
            "SELECT id, channel_id, role, sender_name, sender_id, content, metadata, created_at \
             FROM conversation_messages \
             WHERE channel_id = ? \
             ORDER BY created_at DESC \
             LIMIT ?",
        )
        .bind(channel_id.as_ref())
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

        let mut messages: Vec<ConversationMessage> = rows
            .into_iter()
            .map(|row| ConversationMessage {
                id: row.try_get("id").unwrap_or_default(),
                channel_id: row.try_get("channel_id").unwrap_or_default(),
                role: row.try_get("role").unwrap_or_default(),
                sender_name: row.try_get("sender_name").ok(),
                sender_id: row.try_get("sender_id").ok(),
                content: row.try_get("content").unwrap_or_default(),
                metadata: row.try_get("metadata").ok(),
                created_at: row
                    .try_get("created_at")
                    .unwrap_or_else(|_| chrono::Utc::now()),
            })
            .collect();

        // Reverse to chronological order
        messages.reverse();

        Ok(messages)
    }

    /// Load messages from any channel (not just the current one).
    ///
    /// Supports optional temporal filtering via `before` and `after` (RFC 3339 strings)
    /// and ordering via `oldest_first`. When `oldest_first` is true, returns the earliest
    /// matching messages instead of the most recent.
    pub async fn load_channel_transcript(
        &self,
        channel_id: &str,
        limit: i64,
        before: Option<&str>,
        after: Option<&str>,
        oldest_first: bool,
    ) -> crate::error::Result<Vec<ConversationMessage>> {
        let mut sql = String::from(
            "SELECT id, channel_id, role, sender_name, sender_id, content, metadata, created_at \
             FROM conversation_messages \
             WHERE channel_id = ?",
        );

        if before.is_some() {
            sql.push_str(" AND created_at < ?");
        }
        if after.is_some() {
            sql.push_str(" AND created_at > ?");
        }

        if oldest_first {
            sql.push_str(" ORDER BY created_at ASC");
        } else {
            sql.push_str(" ORDER BY created_at DESC");
        }
        sql.push_str(" LIMIT ?");

        let mut query = sqlx::query(&sql).bind(channel_id);
        if let Some(before) = before {
            query = query.bind(before);
        }
        if let Some(after) = after {
            query = query.bind(after);
        }
        query = query.bind(limit);

        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;

        let mut messages: Vec<ConversationMessage> = rows
            .into_iter()
            .map(|row| ConversationMessage {
                id: row.try_get("id").unwrap_or_default(),
                channel_id: row.try_get("channel_id").unwrap_or_default(),
                role: row.try_get("role").unwrap_or_default(),
                sender_name: row.try_get("sender_name").ok(),
                sender_id: row.try_get("sender_id").ok(),
                content: row.try_get("content").unwrap_or_default(),
                metadata: row.try_get("metadata").ok(),
                created_at: row
                    .try_get("created_at")
                    .unwrap_or_else(|_| chrono::Utc::now()),
            })
            .collect();

        // When fetching newest-first, reverse to chronological for the caller
        if !oldest_first {
            messages.reverse();
        }
        Ok(messages)
    }
}

/// A unified timeline item combining messages, branch runs, and worker runs.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimelineItem {
    Message {
        id: String,
        role: String,
        sender_name: Option<String>,
        sender_id: Option<String>,
        content: String,
        created_at: String,
    },
    BranchRun {
        id: String,
        description: String,
        conclusion: Option<String>,
        started_at: String,
        completed_at: Option<String>,
    },
    WorkerRun {
        id: String,
        task: String,
        result: Option<String>,
        status: String,
        started_at: String,
        completed_at: Option<String>,
    },
}

/// Persists branch and worker run records for channel timeline history.
///
/// All write methods are fire-and-forget, same pattern as ConversationLogger.
#[derive(Debug, Clone)]
pub struct ProcessRunLogger {
    pool: SqlitePool,
}

impl ProcessRunLogger {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Persist or reaffirm a durable operator policy for an agent. Fire-and-forget.
    pub fn upsert_operator_policy(
        &self,
        agent_id: &str,
        policy_key: &str,
        content: &str,
        priority: i64,
        source_channel_id: Option<&ChannelId>,
    ) {
        let pool = self.pool.clone();
        let id = uuid::Uuid::new_v4().to_string();
        let agent_id = agent_id.to_string();
        let policy_key = policy_key.to_string();
        let content = content.to_string();
        let source_channel_id = source_channel_id.map(ToString::to_string);

        tokio::spawn(async move {
            if let Err(error) = sqlx::query(
                "INSERT INTO operator_policies \
                 (id, agent_id, policy_key, content, priority, source_channel_id, last_reaffirmed_at) \
                 VALUES (?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP) \
                 ON CONFLICT(agent_id, policy_key) DO UPDATE SET \
                    content = excluded.content, \
                    priority = MAX(operator_policies.priority, excluded.priority), \
                    updated_at = CURRENT_TIMESTAMP, \
                    last_reaffirmed_at = CURRENT_TIMESTAMP, \
                    source_channel_id = COALESCE(excluded.source_channel_id, operator_policies.source_channel_id)",
            )
            .bind(&id)
            .bind(&agent_id)
            .bind(&policy_key)
            .bind(&content)
            .bind(priority)
            .bind(&source_channel_id)
            .execute(&pool)
            .await
            {
                tracing::warn!(%error, %agent_id, %policy_key, "failed to persist operator policy");
            }
        });
    }

    /// Load durable operator policies for an agent, ordered by precedence.
    pub async fn list_operator_policies(
        &self,
        agent_id: &str,
        limit: i64,
    ) -> crate::error::Result<Vec<OperatorPolicyRecord>> {
        let rows = sqlx::query_as::<_, OperatorPolicyRecord>(
            "SELECT id, agent_id, policy_key, content, priority, created_at, updated_at, \
                    last_reaffirmed_at, source_channel_id \
             FROM operator_policies \
             WHERE agent_id = ? \
             ORDER BY priority DESC, updated_at DESC \
             LIMIT ?",
        )
        .bind(agent_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| anyhow::anyhow!(error))?;

        Ok(rows)
    }

    /// Persist a new request-group record. Fire-and-forget.
    #[allow(clippy::too_many_arguments)]
    pub fn log_request_group_started(
        &self,
        request_group_id: &str,
        agent_id: &str,
        channel_id: &ChannelId,
        origin_message_id: Option<&str>,
        origin_user_text: &str,
        mode: RequestGroupMode,
        task_class: Option<TaskClass>,
        source_manifest: &SourceManifest,
    ) {
        let pool = self.pool.clone();
        let request_group_id = request_group_id.to_string();
        let agent_id = agent_id.to_string();
        let channel_id = channel_id.to_string();
        let origin_message_id = origin_message_id.map(ToOwned::to_owned);
        let origin_user_text = origin_user_text.to_string();
        let mode = mode.as_str().to_string();
        let task_class = task_class.map(|value| value.as_str().to_string());
        let source_manifest_json = serde_json::to_string(source_manifest).ok();

        tokio::spawn(async move {
            if let Err(error) = sqlx::query(
                "INSERT OR IGNORE INTO request_groups \
                 (id, agent_id, channel_id, origin_message_id, origin_user_text, mode, state, task_class, source_manifest_json) \
                 VALUES (?, ?, ?, ?, ?, ?, 'planning', ?, ?)",
            )
            .bind(&request_group_id)
            .bind(&agent_id)
            .bind(&channel_id)
            .bind(&origin_message_id)
            .bind(&origin_user_text)
            .bind(&mode)
            .bind(&task_class)
            .bind(&source_manifest_json)
            .execute(&pool)
            .await
            {
                tracing::warn!(%error, request_group_id, "failed to persist request group");
            }
        });
    }

    /// Update the durable state of a request group. Fire-and-forget.
    pub fn log_request_group_state(
        &self,
        request_group_id: &str,
        state: RequestGroupState,
        required_receipts_json: Option<String>,
        completion_policy_json: Option<String>,
    ) {
        let pool = self.pool.clone();
        let request_group_id = request_group_id.to_string();
        let state = state.as_str().to_string();

        tokio::spawn(async move {
            if let Err(error) = sqlx::query(
                "UPDATE request_groups SET state = ?, required_receipts_json = COALESCE(?, required_receipts_json), \
                 completion_policy_json = COALESCE(?, completion_policy_json), updated_at = CURRENT_TIMESTAMP WHERE id = ?",
            )
            .bind(&state)
            .bind(&required_receipts_json)
            .bind(&completion_policy_json)
            .bind(&request_group_id)
            .execute(&pool)
            .await
            {
                tracing::warn!(%error, request_group_id, "failed to update request group state");
            }
        });
    }

    /// Load recent request groups for an agent, newest first.
    pub async fn list_request_groups(
        &self,
        agent_id: &str,
        limit: i64,
        channel_id: Option<&str>,
        state: Option<&str>,
    ) -> crate::error::Result<Vec<RequestGroupRecord>> {
        let rows = sqlx::query_as::<_, RequestGroupRecord>(
            "SELECT id, agent_id, channel_id, origin_message_id, origin_user_text, mode, state, \
                    task_class, source_manifest_json, required_receipts_json, completion_policy_json, \
                    created_at, updated_at \
             FROM request_groups \
             WHERE agent_id = ? \
               AND (? IS NULL OR channel_id = ?) \
               AND (? IS NULL OR state = ?) \
             ORDER BY updated_at DESC \
             LIMIT ?",
        )
        .bind(agent_id)
        .bind(channel_id)
        .bind(channel_id)
        .bind(state)
        .bind(state)
        .bind(limit.max(1))
        .fetch_all(&self.pool)
        .await
        .map_err(|error| anyhow::anyhow!(error))?;

        Ok(rows)
    }

    /// Load one request-group with its branches, workers, and merged receipt summary.
    pub async fn get_request_group_detail(
        &self,
        request_group_id: &str,
    ) -> crate::error::Result<Option<RequestGroupDetail>> {
        let request_group = sqlx::query_as::<_, RequestGroupRecord>(
            "SELECT id, agent_id, channel_id, origin_message_id, origin_user_text, mode, state, \
                    task_class, source_manifest_json, required_receipts_json, completion_policy_json, \
                    created_at, updated_at \
             FROM request_groups WHERE id = ?",
        )
        .bind(request_group_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| anyhow::anyhow!(error))?;

        let Some(request_group) = request_group else {
            return Ok(None);
        };

        let branch_rows = sqlx::query(
            "SELECT id, description, conclusion, completed_at \
             FROM branch_runs WHERE request_group_id = ? ORDER BY COALESCE(completed_at, started_at) ASC",
        )
        .bind(request_group_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| anyhow::anyhow!(error))?;
        let branches = branch_rows
            .into_iter()
            .map(|row| RequestGroupBranchRecord {
                id: row.try_get("id").unwrap_or_default(),
                description: row.try_get("description").unwrap_or_default(),
                conclusion: row.try_get("conclusion").ok(),
                completed_at: row.try_get("completed_at").ok(),
            })
            .collect::<Vec<_>>();

        let worker_rows = sqlx::query(
            "SELECT id, task, status, completed_at, worker_type, outcome_receipt_json \
             FROM worker_runs WHERE request_group_id = ? ORDER BY COALESCE(completed_at, started_at) ASC",
        )
        .bind(request_group_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| anyhow::anyhow!(error))?;
        let workers = worker_rows
            .into_iter()
            .map(|row| {
                let worker_id = row.try_get::<String, _>("id").unwrap_or_default();
                let receipt = row
                    .try_get::<String, _>("outcome_receipt_json")
                    .ok()
                    .and_then(|value| serde_json::from_str::<WorkerOutcomeReceipt>(&value).ok())
                    .map(|receipt| summarize_request_group_receipt(&worker_id, &receipt));

                RequestGroupWorkerRecord {
                    id: worker_id,
                    task: row.try_get("task").unwrap_or_default(),
                    status: row.try_get("status").unwrap_or_default(),
                    completed_at: row.try_get("completed_at").ok(),
                    worker_type: row.try_get("worker_type").unwrap_or_default(),
                    receipt,
                }
            })
            .collect::<Vec<_>>();

        let merged = merge_receipt_references(
            &workers
                .iter()
                .filter_map(|worker| worker.receipt.clone())
                .collect::<Vec<_>>(),
        );

        Ok(Some(RequestGroupDetail {
            request_group,
            branches,
            workers,
            merged,
        }))
    }

    /// Record a branch starting. Fire-and-forget.
    pub fn log_branch_started(
        &self,
        channel_id: &ChannelId,
        branch_id: BranchId,
        description: &str,
    ) {
        let pool = self.pool.clone();
        let id = branch_id.to_string();
        let channel_id = channel_id.to_string();
        let description = description.to_string();

        tokio::spawn(async move {
            if let Err(error) = sqlx::query(
                "INSERT OR IGNORE INTO branch_runs (id, channel_id, description) VALUES (?, ?, ?)",
            )
            .bind(&id)
            .bind(&channel_id)
            .bind(&description)
            .execute(&pool)
            .await
            {
                tracing::warn!(%error, branch_id = %id, "failed to persist branch start");
            }
        });
    }

    /// Record a branch completing with its conclusion. Fire-and-forget.
    pub fn log_branch_completed(&self, branch_id: BranchId, conclusion: &str) {
        let pool = self.pool.clone();
        let id = branch_id.to_string();
        let conclusion = conclusion.to_string();

        tokio::spawn(async move {
            if let Err(error) = sqlx::query(
                "UPDATE branch_runs SET conclusion = ?, completed_at = CURRENT_TIMESTAMP WHERE id = ?"
            )
            .bind(&conclusion)
            .bind(&id)
            .execute(&pool)
            .await
            {
                tracing::warn!(%error, branch_id = %id, "failed to persist branch completion");
            }
        });
    }

    /// Link an existing branch run to a request group. Fire-and-forget.
    pub fn log_branch_request_group(&self, branch_id: BranchId, request_group_id: &str) {
        let pool = self.pool.clone();
        let branch_id = branch_id.to_string();
        let request_group_id = request_group_id.to_string();

        tokio::spawn(async move {
            if let Err(error) =
                sqlx::query("UPDATE branch_runs SET request_group_id = ? WHERE id = ?")
                    .bind(&request_group_id)
                    .bind(&branch_id)
                    .execute(&pool)
                    .await
            {
                tracing::warn!(%error, branch_id, request_group_id, "failed to link branch request group");
            }
        });
    }

    /// Record a worker starting. Fire-and-forget.
    #[allow(clippy::too_many_arguments)]
    pub fn log_worker_started(
        &self,
        channel_id: Option<&ChannelId>,
        worker_id: WorkerId,
        task: &str,
        worker_type: &str,
        agent_id: &crate::AgentId,
        interactive: bool,
        directory: Option<&std::path::Path>,
    ) {
        let pool = self.pool.clone();
        let id = worker_id.to_string();
        let channel_id = channel_id.map(|c| c.to_string());
        let task = task.to_string();
        let worker_type = worker_type.to_string();
        let agent_id = agent_id.to_string();
        let directory = directory.map(|d| d.to_string_lossy().to_string());

        tokio::spawn(async move {
            if let Err(error) = sqlx::query(
                "INSERT OR IGNORE INTO worker_runs (id, channel_id, task, worker_type, agent_id, interactive, directory) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(&channel_id)
            .bind(&task)
            .bind(&worker_type)
            .bind(&agent_id)
            .bind(interactive)
            .bind(&directory)
            .execute(&pool)
            .await
            {
                tracing::warn!(%error, worker_id = %id, "failed to persist worker start");
            }
        });
    }

    /// Link a worker run to a project and/or worktree. Fire-and-forget.
    ///
    /// Called after spawn when `project_id` or `worktree_id` was set in the
    /// spawn args. Uses a separate UPDATE to avoid changing the WorkerStarted
    /// event shape.
    pub fn log_worker_project_link(
        &self,
        worker_id: WorkerId,
        project_id: Option<&str>,
        worktree_id: Option<&str>,
    ) {
        if project_id.is_none() && worktree_id.is_none() {
            return;
        }
        let pool = self.pool.clone();
        let id = worker_id.to_string();
        let project_id = project_id.map(|s| s.to_string());
        let worktree_id = worktree_id.map(|s| s.to_string());

        tokio::spawn(async move {
            // The worker_run INSERT may not have committed yet (it's also
            // fire-and-forget). Retry a few times with a short delay so we
            // don't silently update 0 rows.
            for attempt in 0..3u8 {
                if attempt > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                match sqlx::query(
                    "UPDATE worker_runs SET project_id = COALESCE(?, project_id), \
                     worktree_id = COALESCE(?, worktree_id) WHERE id = ?",
                )
                .bind(&project_id)
                .bind(&worktree_id)
                .bind(&id)
                .execute(&pool)
                .await
                {
                    Ok(result) if result.rows_affected() > 0 => return,
                    Ok(_) => {
                        // Row doesn't exist yet — retry.
                    }
                    Err(error) => {
                        tracing::warn!(%error, worker_id = %id, "failed to link worker to project");
                        return;
                    }
                }
            }
            tracing::debug!(worker_id = %id, "worker_runs row not found after retries for project link");
        });
    }

    /// Link a worker run to a request group and persist its task class/source manifest.
    pub fn log_worker_request_group(
        &self,
        worker_id: WorkerId,
        request_group_id: &str,
        task_class: Option<TaskClass>,
        source_manifest: Option<&SourceManifest>,
    ) {
        let pool = self.pool.clone();
        let worker_id = worker_id.to_string();
        let request_group_id = request_group_id.to_string();
        let task_class = task_class.map(|value| value.as_str().to_string());
        let source_manifest_json =
            source_manifest.and_then(|value| serde_json::to_string(value).ok());

        tokio::spawn(async move {
            if let Err(error) = sqlx::query(
                "UPDATE worker_runs SET request_group_id = ?, task_class = COALESCE(?, task_class), \
                 source_manifest_json = COALESCE(?, source_manifest_json) WHERE id = ?",
            )
            .bind(&request_group_id)
            .bind(&task_class)
            .bind(&source_manifest_json)
            .bind(&worker_id)
            .execute(&pool)
            .await
            {
                tracing::warn!(%error, worker_id, request_group_id, "failed to link worker request group");
            }
        });
    }

    /// Persist a structured worker completion receipt. Fire-and-forget.
    pub fn log_worker_receipt(&self, worker_id: WorkerId, receipt: &WorkerOutcomeReceipt) {
        let pool = self.pool.clone();
        let worker_id = worker_id.to_string();
        let receipt_json = match serde_json::to_string(receipt) {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(%error, worker_id, "failed to serialize worker receipt");
                return;
            }
        };

        tokio::spawn(async move {
            if let Err(error) =
                sqlx::query("UPDATE worker_runs SET outcome_receipt_json = ? WHERE id = ?")
                    .bind(&receipt_json)
                    .bind(&worker_id)
                    .execute(&pool)
                    .await
            {
                tracing::warn!(%error, worker_id, "failed to persist worker receipt");
            }
        });
    }

    /// Update typed artifact delivery facts after the channel/runtime has
    /// attempted delivery. This keeps worker receipts auditable without asking
    /// the model to infer adapter behavior in prose.
    pub async fn update_worker_receipt_artifact_delivery(
        &self,
        worker_id: &str,
        updates: &[ArtifactDeliveryUpdate],
    ) -> crate::error::Result<()> {
        if updates.is_empty() {
            return Ok(());
        }

        let row = sqlx::query("SELECT outcome_receipt_json FROM worker_runs WHERE id = ?")
            .bind(worker_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| anyhow::anyhow!(error))?;

        let Some(row) = row else {
            return Ok(());
        };

        let Some(receipt_json) = row
            .try_get::<Option<String>, _>("outcome_receipt_json")
            .ok()
            .flatten()
        else {
            return Ok(());
        };

        let mut receipt: WorkerOutcomeReceipt = serde_json::from_str(&receipt_json)
            .map_err(|error| anyhow::anyhow!("failed to decode worker receipt: {error}"))?;

        if receipt.artifact_outcomes.is_empty() {
            return Ok(());
        }

        for update in updates {
            if let Some(artifact) = receipt
                .artifact_outcomes
                .iter_mut()
                .find(|artifact| artifact.path == update.path)
            {
                artifact.delivery_status = Some(update.delivery_status.clone());
                if update.delivery_target.is_some() {
                    artifact.delivery_target = update.delivery_target.clone();
                }
            }
        }

        let updated_json = serde_json::to_string(&receipt)
            .map_err(|error| anyhow::anyhow!("failed to encode updated worker receipt: {error}"))?;

        sqlx::query("UPDATE worker_runs SET outcome_receipt_json = ? WHERE id = ?")
            .bind(updated_json)
            .bind(worker_id)
            .execute(&self.pool)
            .await
            .map_err(|error| anyhow::anyhow!(error))?;

        Ok(())
    }

    /// Update a worker's status. Fire-and-forget.
    /// Most status text updates are transient — they're available via the
    /// in-memory StatusBlock for live workers and don't need to be persisted.
    /// The `status` column is reserved for the state enum (running/idle/done/failed).
    ///
    /// The one exception: when an idle worker resumes (status contains
    /// "processing follow-up" or similar active-work indicators), we persist
    /// `running` to the DB so the frontend doesn't show stale "idle" state.
    pub fn log_worker_status(&self, worker_id: WorkerId, status: &str) {
        // Detect when an idle worker resumes active work and persist the
        // transition. All other status text is transient.
        if status.starts_with("processing") || status == "running" {
            self.log_worker_resumed(worker_id);
        }
    }

    /// Mark an interactive worker as idle (waiting for follow-up input).
    /// Persisted so the frontend shows "idle" instead of "running".
    pub fn log_worker_idle(&self, worker_id: WorkerId) {
        let pool = self.pool.clone();
        let id = worker_id.to_string();

        tokio::spawn(async move {
            if let Err(error) = sqlx::query("UPDATE worker_runs SET status = 'idle' WHERE id = ?")
                .bind(&id)
                .execute(&pool)
                .await
            {
                tracing::warn!(%error, worker_id = %id, "failed to persist worker idle state");
            }
        });
    }

    /// Mark an idle worker as running again (follow-up received).
    pub fn log_worker_resumed(&self, worker_id: WorkerId) {
        let pool = self.pool.clone();
        let id = worker_id.to_string();

        tokio::spawn(async move {
            if let Err(error) =
                sqlx::query("UPDATE worker_runs SET status = 'running' WHERE id = ?")
                    .bind(&id)
                    .execute(&pool)
                    .await
            {
                tracing::warn!(%error, worker_id = %id, "failed to persist worker resumed state");
            }
        });
    }

    /// Record a worker completing with its result. Fire-and-forget.
    pub fn log_worker_completed(&self, worker_id: WorkerId, result: &str, success: bool) {
        let status = if success { "done" } else { "failed" };
        self.log_worker_completed_with_status(worker_id, result, status);
    }

    /// Record a worker as cancelled. Fire-and-forget.
    pub fn log_worker_cancelled(&self, worker_id: WorkerId, result: &str) {
        self.log_worker_completed_with_status(worker_id, result, "cancelled");
    }

    fn log_worker_completed_with_status(&self, worker_id: WorkerId, result: &str, status: &str) {
        let pool = self.pool.clone();
        let id = worker_id.to_string();
        let result = result.to_string();
        let status = status.to_string();

        tokio::spawn(async move {
            if let Err(error) = sqlx::query(
                "UPDATE worker_runs SET result = ?, status = ?, completed_at = CURRENT_TIMESTAMP WHERE id = ? AND completed_at IS NULL"
            )
            .bind(&result)
            .bind(status)
            .bind(&id)
            .execute(&pool)
            .await
            {
                tracing::warn!(%error, worker_id = %id, "failed to persist worker completion");
            }
        });
    }

    /// Record OpenCode session metadata on a worker run. Fire-and-forget.
    ///
    /// Stores the session ID and server port so the frontend can construct
    /// an iframe URL to the embedded OpenCode web UI.
    ///
    /// The worker row is inserted by `log_worker_started` (also fire-and-forget),
    /// which may not have committed yet when this runs. To handle the race we
    /// retry with a short back-off when the UPDATE affects zero rows.
    pub fn log_opencode_metadata(&self, worker_id: WorkerId, session_id: &str, port: u16) {
        let pool = self.pool.clone();
        let id = worker_id.to_string();
        let session_id = session_id.to_string();

        tokio::spawn(async move {
            const MAX_RETRIES: u32 = 5;
            const BASE_DELAY_MS: u64 = 50;

            for attempt in 0..=MAX_RETRIES {
                match sqlx::query(
                    "UPDATE worker_runs SET opencode_session_id = ?, opencode_port = ? WHERE id = ?",
                )
                .bind(&session_id)
                .bind(port as i32)
                .bind(&id)
                .execute(&pool)
                .await
                {
                    Ok(result) if result.rows_affected() > 0 => {
                        return; // Successfully updated.
                    }
                    Ok(_) => {
                        // Row doesn't exist yet — INSERT hasn't committed.
                        if attempt < MAX_RETRIES {
                            let delay = BASE_DELAY_MS * 2u64.pow(attempt);
                            tracing::debug!(
                                worker_id = %id,
                                attempt,
                                delay_ms = delay,
                                "worker_runs row not yet inserted, retrying opencode metadata update"
                            );
                            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        } else {
                            tracing::warn!(
                                worker_id = %id,
                                "worker_runs row never appeared after {MAX_RETRIES} retries, \
                                 opencode metadata (port={port}) lost"
                            );
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            worker_id = %id,
                            "failed to persist OpenCode metadata"
                        );
                        return;
                    }
                }
            }
        });
    }

    /// Mark orphaned **running** workers as failed for an agent.
    ///
    /// Called at startup to reconcile rows that were left in `running` status
    /// when the process exited before a `WorkerComplete` event was persisted.
    ///
    /// Idle interactive workers are intentionally left alone — they will be
    /// resumed by `get_idle_interactive_workers()` + the reconnection logic.
    pub async fn reconcile_running_workers_for_agent(
        &self,
        agent_id: &str,
        failure_message: &str,
    ) -> crate::error::Result<Vec<InterruptedWorkerRow>> {
        let interrupted_workers = sqlx::query_as::<_, InterruptedWorkerRow>(
            "SELECT id, channel_id, task, worker_type, started_at, directory, transcript \
             FROM worker_runs \
             WHERE status = 'running' AND (agent_id = ? OR agent_id IS NULL)",
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| anyhow::anyhow!(error))?;

        if interrupted_workers.is_empty() {
            return Ok(Vec::new());
        }

        let result = sqlx::query(
            "UPDATE worker_runs \
             SET status = 'failed', \
                 completed_at = COALESCE(completed_at, CURRENT_TIMESTAMP), \
                 result = CASE \
                     WHEN result IS NULL OR result = '' THEN ? \
                     ELSE result \
                 END \
             WHERE status = 'running' AND (agent_id = ? OR agent_id IS NULL)",
        )
        .bind(failure_message)
        .bind(agent_id)
        .execute(&self.pool)
        .await
        .map_err(|error| anyhow::anyhow!(error))?;

        let rows_affected = result.rows_affected() as usize;
        if rows_affected < interrupted_workers.len() {
            tracing::warn!(
                agent_id,
                rows_affected,
                expected = interrupted_workers.len(),
                "startup worker reconciliation updated fewer rows than expected"
            );
        }

        Ok(interrupted_workers)
    }

    /// Load all idle interactive workers for an agent.
    ///
    /// Called at startup to find workers that were waiting for follow-up input
    /// when the process exited. These can potentially be reconnected to their
    /// sessions and resumed rather than marked as failed.
    pub async fn get_idle_interactive_workers(
        &self,
        agent_id: &str,
    ) -> crate::error::Result<Vec<IdleWorkerRow>> {
        let rows = sqlx::query_as::<_, IdleWorkerRow>(
            "SELECT id, task, channel_id, worker_type, started_at, transcript, \
                    COALESCE(tool_calls, 0) AS tool_calls, \
                    opencode_session_id, opencode_port, directory \
             FROM worker_runs \
             WHERE status = 'idle' AND interactive = TRUE \
                   AND (agent_id = ? OR agent_id IS NULL)",
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| anyhow::anyhow!(error))?;

        Ok(rows)
    }

    /// Mark an idle worker as failed (used when reconnection fails at startup).
    pub async fn fail_idle_worker(
        &self,
        worker_id: &str,
        reason: &str,
    ) -> crate::error::Result<()> {
        sqlx::query(
            "UPDATE worker_runs \
             SET status = 'failed', \
                 completed_at = COALESCE(completed_at, CURRENT_TIMESTAMP), \
                 result = CASE \
                     WHEN result IS NULL OR result = '' THEN ? \
                     ELSE result \
                 END \
             WHERE id = ? AND status = 'idle'",
        )
        .bind(reason)
        .bind(worker_id)
        .execute(&self.pool)
        .await
        .map_err(|error| anyhow::anyhow!(error))?;
        Ok(())
    }

    /// Retire an idle worker whose session can no longer be resumed.
    ///
    /// Marks the row as `done` (not `failed`) because the worker completed its
    /// work successfully — only the follow-up session expired. The existing
    /// result and transcript are preserved.
    pub async fn retire_idle_worker(&self, worker_id: &str) -> crate::error::Result<()> {
        sqlx::query(
            "UPDATE worker_runs \
             SET status = 'done', \
                 completed_at = COALESCE(completed_at, CURRENT_TIMESTAMP) \
             WHERE id = ? AND status = 'idle'",
        )
        .bind(worker_id)
        .execute(&self.pool)
        .await
        .map_err(|error| anyhow::anyhow!(error))?;
        Ok(())
    }

    /// Mark a detached running worker as cancelled.
    ///
    /// Used by API cancellation when the in-memory channel state no longer has
    /// a live handle for this worker (for example after restart).
    pub async fn cancel_running_worker(
        &self,
        channel_id: &str,
        worker_id: WorkerId,
    ) -> crate::error::Result<bool> {
        let result = sqlx::query(
            "UPDATE worker_runs \
             SET result = CASE \
                     WHEN result IS NULL OR result = '' THEN 'Worker cancelled' \
                     ELSE result \
                 END, \
                 status = 'cancelled', \
                 completed_at = COALESCE(completed_at, CURRENT_TIMESTAMP) \
             WHERE id = ? AND channel_id = ? AND status = 'running'",
        )
        .bind(worker_id.to_string())
        .bind(channel_id)
        .execute(&self.pool)
        .await
        .map_err(|error| anyhow::anyhow!(error))?;

        Ok(result.rows_affected() > 0)
    }

    /// Mark a detached running worker (`channel_id IS NULL`) as cancelled.
    ///
    /// Used by API cancellation fallback when no in-memory channel state exists.
    pub async fn cancel_running_detached_worker(
        &self,
        worker_id: WorkerId,
    ) -> crate::error::Result<bool> {
        let result = sqlx::query(
            "UPDATE worker_runs \
             SET result = CASE \
                     WHEN result IS NULL OR result = '' THEN 'Worker cancelled' \
                     ELSE result \
                 END, \
                 status = 'cancelled', \
                 completed_at = COALESCE(completed_at, CURRENT_TIMESTAMP) \
             WHERE id = ? AND channel_id IS NULL AND status = 'running'",
        )
        .bind(worker_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|error| anyhow::anyhow!(error))?;

        Ok(result.rows_affected() > 0)
    }

    /// Load a unified timeline for a channel: messages, branch runs, and worker runs
    /// interleaved chronologically (oldest first).
    ///
    /// When `before` is provided, only items with a timestamp strictly before that
    /// value are returned, enabling cursor-based pagination.
    pub async fn load_channel_timeline(
        &self,
        channel_id: &str,
        limit: i64,
        before: Option<&str>,
    ) -> crate::error::Result<Vec<TimelineItem>> {
        let before_clause = if before.is_some() {
            "AND datetime(timestamp) < datetime(?3)"
        } else {
            ""
        };

        let query_str = format!(
            "SELECT * FROM ( \
                SELECT 'message' AS item_type, id, role, sender_name, sender_id, content, \
                       NULL AS description, NULL AS conclusion, NULL AS task, NULL AS result, NULL AS status, \
                       created_at AS timestamp, NULL AS completed_at \
                FROM conversation_messages WHERE channel_id = ?1 \
                UNION ALL \
                SELECT 'branch_run' AS item_type, id, NULL, NULL, NULL, NULL, \
                       description, conclusion, NULL, NULL, NULL, \
                       started_at AS timestamp, completed_at \
                FROM branch_runs WHERE channel_id = ?1 \
                UNION ALL \
                SELECT 'worker_run' AS item_type, id, NULL, NULL, NULL, NULL, \
                       NULL, NULL, task, result, status, \
                       started_at AS timestamp, completed_at \
                FROM worker_runs WHERE channel_id = ?1 \
            ) WHERE 1=1 {before_clause} ORDER BY timestamp DESC LIMIT ?2"
        );

        let mut query = sqlx::query(&query_str).bind(channel_id).bind(limit);

        if let Some(before_ts) = before {
            query = query.bind(before_ts);
        }

        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;

        let mut items: Vec<TimelineItem> = rows
            .into_iter()
            .filter_map(|row| {
                let item_type: String = row.try_get("item_type").ok()?;
                match item_type.as_str() {
                    "message" => Some(TimelineItem::Message {
                        id: row.try_get("id").unwrap_or_default(),
                        role: row.try_get("role").unwrap_or_default(),
                        sender_name: row.try_get("sender_name").ok(),
                        sender_id: row.try_get("sender_id").ok(),
                        content: row.try_get("content").unwrap_or_default(),
                        created_at: row
                            .try_get::<chrono::DateTime<chrono::Utc>, _>("timestamp")
                            .map(|t| t.to_rfc3339())
                            .unwrap_or_default(),
                    }),
                    "branch_run" => Some(TimelineItem::BranchRun {
                        id: row.try_get("id").unwrap_or_default(),
                        description: row.try_get("description").unwrap_or_default(),
                        conclusion: row.try_get("conclusion").ok(),
                        started_at: row
                            .try_get::<chrono::DateTime<chrono::Utc>, _>("timestamp")
                            .map(|t| t.to_rfc3339())
                            .unwrap_or_default(),
                        completed_at: row
                            .try_get::<chrono::DateTime<chrono::Utc>, _>("completed_at")
                            .ok()
                            .map(|t| t.to_rfc3339()),
                    }),
                    "worker_run" => Some(TimelineItem::WorkerRun {
                        id: row.try_get("id").unwrap_or_default(),
                        task: row.try_get("task").unwrap_or_default(),
                        result: row.try_get("result").ok(),
                        status: row.try_get("status").unwrap_or_default(),
                        started_at: row
                            .try_get::<chrono::DateTime<chrono::Utc>, _>("timestamp")
                            .map(|t| t.to_rfc3339())
                            .unwrap_or_default(),
                        completed_at: row
                            .try_get::<chrono::DateTime<chrono::Utc>, _>("completed_at")
                            .ok()
                            .map(|t| t.to_rfc3339()),
                    }),
                    _ => None,
                }
            })
            .collect();

        // Reverse to chronological order
        items.reverse();
        Ok(items)
    }

    /// List worker runs for an agent, ordered by most recent first.
    /// Does NOT include the transcript blob — that's fetched separately via `get_worker_detail`.
    pub async fn list_worker_runs(
        &self,
        agent_id: &str,
        limit: i64,
        offset: i64,
        status_filter: Option<&str>,
    ) -> crate::error::Result<(Vec<WorkerRunRow>, i64)> {
        let (count_where_clause, list_where_clause, has_status_filter) = if status_filter.is_some()
        {
            (
                "WHERE w.agent_id = ?1 AND w.status = ?2",
                "WHERE w.agent_id = ?1 AND w.status = ?4",
                true,
            )
        } else {
            ("WHERE w.agent_id = ?1", "WHERE w.agent_id = ?1", false)
        };

        let count_query =
            format!("SELECT COUNT(*) as total FROM worker_runs w {count_where_clause}");
        let list_query = format!(
            "SELECT w.id, w.task, w.status, w.worker_type, w.channel_id, w.started_at, \
                    w.completed_at, w.transcript IS NOT NULL as has_transcript, \
                    w.tool_calls, w.opencode_port, w.interactive, \
                    c.display_name as channel_name \
             FROM worker_runs w \
             LEFT JOIN channels c ON w.channel_id = c.id \
             {list_where_clause} \
             ORDER BY w.started_at DESC \
             LIMIT ?2 OFFSET ?3"
        );

        let mut count_q = sqlx::query(&count_query).bind(agent_id);
        let mut list_q = sqlx::query(&list_query)
            .bind(agent_id)
            .bind(limit)
            .bind(offset);

        if has_status_filter {
            let filter = status_filter.unwrap_or("");
            count_q = count_q.bind(filter);
            list_q = list_q.bind(filter);
        }

        let total: i64 = count_q
            .fetch_one(&self.pool)
            .await
            .map(|row| row.try_get("total").unwrap_or(0))
            .map_err(|e| anyhow::anyhow!(e))?;

        let rows = list_q
            .fetch_all(&self.pool)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;

        let items = rows
            .into_iter()
            .map(|row| WorkerRunRow {
                id: row.try_get("id").unwrap_or_default(),
                task: row.try_get("task").unwrap_or_default(),
                status: row.try_get("status").unwrap_or_default(),
                worker_type: row
                    .try_get("worker_type")
                    .unwrap_or_else(|_| "builtin".into()),
                channel_id: row.try_get("channel_id").ok(),
                channel_name: row.try_get("channel_name").ok(),
                started_at: row
                    .try_get::<chrono::DateTime<chrono::Utc>, _>("started_at")
                    .map(|t| t.to_rfc3339())
                    .unwrap_or_default(),
                completed_at: row
                    .try_get::<chrono::DateTime<chrono::Utc>, _>("completed_at")
                    .ok()
                    .map(|t| t.to_rfc3339()),
                has_transcript: row.try_get::<bool, _>("has_transcript").unwrap_or(false),
                tool_calls: row.try_get::<i64, _>("tool_calls").unwrap_or(0),
                opencode_port: row.try_get::<i32, _>("opencode_port").ok(),
                interactive: row.try_get::<bool, _>("interactive").unwrap_or(false),
            })
            .collect();

        Ok((items, total))
    }

    /// Get full detail for a single worker run, including the compressed transcript blob.
    pub async fn get_worker_detail(
        &self,
        agent_id: &str,
        worker_id: &str,
    ) -> crate::error::Result<Option<WorkerDetailRow>> {
        let row = sqlx::query(
            "SELECT w.id, w.task, w.result, w.status, w.worker_type, w.channel_id, \
                    w.started_at, w.completed_at, w.transcript, w.tool_calls, \
                    w.opencode_session_id, w.opencode_port, w.interactive, w.directory, \
                    w.outcome_receipt_json, c.display_name as channel_name \
             FROM worker_runs w \
             LEFT JOIN channels c ON w.channel_id = c.id \
             WHERE w.agent_id = ? AND w.id = ?",
        )
        .bind(agent_id)
        .bind(worker_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

        Ok(row.map(|row| WorkerDetailRow {
            id: row.try_get("id").unwrap_or_default(),
            task: row.try_get("task").unwrap_or_default(),
            result: row.try_get("result").ok(),
            status: row.try_get("status").unwrap_or_default(),
            worker_type: row
                .try_get("worker_type")
                .unwrap_or_else(|_| "builtin".into()),
            channel_id: row.try_get("channel_id").ok(),
            channel_name: row.try_get("channel_name").ok(),
            started_at: row
                .try_get::<chrono::DateTime<chrono::Utc>, _>("started_at")
                .map(|t| t.to_rfc3339())
                .unwrap_or_default(),
            completed_at: row
                .try_get::<chrono::DateTime<chrono::Utc>, _>("completed_at")
                .ok()
                .map(|t| t.to_rfc3339()),
            transcript_blob: row.try_get("transcript").ok(),
            tool_calls: row.try_get::<i64, _>("tool_calls").unwrap_or(0),
            opencode_session_id: row.try_get("opencode_session_id").ok(),
            opencode_port: row.try_get::<i32, _>("opencode_port").ok(),
            interactive: row.try_get::<bool, _>("interactive").unwrap_or(false),
            directory: row
                .try_get::<Option<String>, _>("directory")
                .unwrap_or(None),
            outcome_receipt_json: row.try_get("outcome_receipt_json").ok(),
        }))
    }
}

/// A worker run row without the transcript blob (for list queries).
#[derive(Debug, Clone, Serialize)]
pub struct WorkerRunRow {
    pub id: String,
    pub task: String,
    pub status: String,
    pub worker_type: String,
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub has_transcript: bool,
    pub tool_calls: i64,
    pub opencode_port: Option<i32>,
    pub interactive: bool,
}

/// A worker that was idle at shutdown, loaded for reconnection at startup.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct IdleWorkerRow {
    pub id: String,
    pub task: String,
    pub channel_id: Option<String>,
    pub worker_type: String,
    pub started_at: String,
    pub transcript: Option<Vec<u8>>,
    pub tool_calls: i64,
    pub opencode_session_id: Option<String>,
    pub opencode_port: Option<i32>,
    pub directory: Option<String>,
}

/// A worker run row with full detail including the transcript blob.
#[derive(Debug, Clone)]
pub struct WorkerDetailRow {
    pub id: String,
    pub task: String,
    pub result: Option<String>,
    pub status: String,
    pub worker_type: String,
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub transcript_blob: Option<Vec<u8>>,
    pub tool_calls: i64,
    pub opencode_session_id: Option<String>,
    pub opencode_port: Option<i32>,
    pub interactive: bool,
    pub directory: Option<String>,
    pub outcome_receipt_json: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{ArtifactDeliveryUpdate, ProcessRunLogger};
    use crate::{ArtifactOutcome, WorkerOutcomeReceipt};

    async fn setup_worker_runs_table() -> sqlx::SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("failed to create sqlite memory pool");

        sqlx::query(
            "CREATE TABLE worker_runs (
                id TEXT PRIMARY KEY,
                channel_id TEXT,
                status TEXT NOT NULL,
                result TEXT,
                completed_at TIMESTAMP,
                outcome_receipt_json TEXT
            )",
        )
        .execute(&pool)
        .await
        .expect("failed to create worker_runs table");

        pool
    }

    #[tokio::test]
    async fn cancel_running_detached_worker_updates_null_channel_rows() {
        let pool = setup_worker_runs_table().await;
        let logger = ProcessRunLogger::new(pool.clone());
        let worker_id = uuid::Uuid::new_v4();

        sqlx::query("INSERT INTO worker_runs (id, channel_id, status, result) VALUES (?, NULL, 'running', '')")
            .bind(worker_id.to_string())
            .execute(&pool)
            .await
            .expect("failed to insert detached worker row");

        let cancelled = logger
            .cancel_running_detached_worker(worker_id)
            .await
            .expect("cancel should succeed");
        assert!(cancelled);

        let row = sqlx::query("SELECT status, result FROM worker_runs WHERE id = ?")
            .bind(worker_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("failed to fetch worker row");

        let status: String = sqlx::Row::try_get(&row, "status").expect("missing status");
        let result: String = sqlx::Row::try_get(&row, "result").expect("missing result");
        assert_eq!(status, "cancelled");
        assert_eq!(result, "Worker cancelled");
    }

    #[tokio::test]
    async fn cancel_running_worker_sets_cancelled_status() {
        let pool = setup_worker_runs_table().await;
        let logger = ProcessRunLogger::new(pool.clone());
        let worker_id = uuid::Uuid::new_v4();

        sqlx::query("INSERT INTO worker_runs (id, channel_id, status, result) VALUES (?, 'ch-1', 'running', '')")
            .bind(worker_id.to_string())
            .execute(&pool)
            .await
            .expect("insert");

        let cancelled = logger
            .cancel_running_worker("ch-1", worker_id)
            .await
            .expect("cancel should succeed");
        assert!(cancelled);

        let row = sqlx::query("SELECT status, result FROM worker_runs WHERE id = ?")
            .bind(worker_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("fetch");
        let status: String = sqlx::Row::try_get(&row, "status").expect("status");
        let result: String = sqlx::Row::try_get(&row, "result").expect("result");
        assert_eq!(status, "cancelled");
        assert_eq!(result, "Worker cancelled");
    }

    /// Poll until a worker's status changes from "running", with a timeout.
    async fn poll_worker_status(
        pool: &sqlx::SqlitePool,
        worker_id: uuid::Uuid,
        timeout_ms: u64,
    ) -> String {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            let row = sqlx::query("SELECT status FROM worker_runs WHERE id = ?")
                .bind(worker_id.to_string())
                .fetch_one(pool)
                .await
                .expect("fetch");
            let status: String = sqlx::Row::try_get(&row, "status").expect("status");
            if status != "running" {
                return status;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!("worker status did not transition within {timeout_ms}ms");
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    #[tokio::test]
    async fn log_worker_completed_success_sets_done_status() {
        let pool = setup_worker_runs_table().await;
        let logger = ProcessRunLogger::new(pool.clone());
        let worker_id = uuid::Uuid::new_v4();

        sqlx::query("INSERT INTO worker_runs (id, channel_id, status, result) VALUES (?, 'ch-1', 'running', '')")
            .bind(worker_id.to_string())
            .execute(&pool)
            .await
            .expect("insert");

        logger.log_worker_completed(worker_id, "task finished", true);
        let status = poll_worker_status(&pool, worker_id, 2000).await;

        assert_eq!(status, "done");
        let row = sqlx::query("SELECT result FROM worker_runs WHERE id = ?")
            .bind(worker_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("fetch");
        let result: String = sqlx::Row::try_get(&row, "result").expect("result");
        assert_eq!(result, "task finished");
    }

    #[tokio::test]
    async fn log_worker_completed_failure_sets_failed_status() {
        let pool = setup_worker_runs_table().await;
        let logger = ProcessRunLogger::new(pool.clone());
        let worker_id = uuid::Uuid::new_v4();

        sqlx::query("INSERT INTO worker_runs (id, channel_id, status, result) VALUES (?, 'ch-1', 'running', '')")
            .bind(worker_id.to_string())
            .execute(&pool)
            .await
            .expect("insert");

        logger.log_worker_completed(worker_id, "something broke", false);
        let status = poll_worker_status(&pool, worker_id, 2000).await;

        assert_eq!(status, "failed");
    }

    #[tokio::test]
    async fn log_worker_cancelled_sets_cancelled_status() {
        let pool = setup_worker_runs_table().await;
        let logger = ProcessRunLogger::new(pool.clone());
        let worker_id = uuid::Uuid::new_v4();

        sqlx::query("INSERT INTO worker_runs (id, channel_id, status, result) VALUES (?, 'ch-1', 'running', '')")
            .bind(worker_id.to_string())
            .execute(&pool)
            .await
            .expect("insert");

        logger.log_worker_cancelled(worker_id, "Worker cancelled: user requested");
        let status = poll_worker_status(&pool, worker_id, 2000).await;

        assert_eq!(status, "cancelled");
        let row = sqlx::query("SELECT result FROM worker_runs WHERE id = ?")
            .bind(worker_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("fetch");
        let result: String = sqlx::Row::try_get(&row, "result").expect("result");
        assert_eq!(result, "Worker cancelled: user requested");
    }

    #[tokio::test]
    async fn cancel_running_detached_worker_does_not_touch_channel_bound_rows() {
        let pool = setup_worker_runs_table().await;
        let logger = ProcessRunLogger::new(pool.clone());
        let worker_id = uuid::Uuid::new_v4();

        sqlx::query(
            "INSERT INTO worker_runs (id, channel_id, status, result) VALUES (?, 'channel-1', 'running', '')",
        )
        .bind(worker_id.to_string())
        .execute(&pool)
        .await
        .expect("failed to insert channel worker row");

        let cancelled = logger
            .cancel_running_detached_worker(worker_id)
            .await
            .expect("cancel should not error");
        assert!(!cancelled);

        let row = sqlx::query("SELECT status FROM worker_runs WHERE id = ?")
            .bind(worker_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("failed to fetch worker row");
        let status: String = sqlx::Row::try_get(&row, "status").expect("missing status");
        assert_eq!(status, "running");
    }

    #[tokio::test]
    async fn update_worker_receipt_artifact_delivery_patches_typed_outcomes() {
        let pool = setup_worker_runs_table().await;
        let logger = ProcessRunLogger::new(pool.clone());
        let worker_id = uuid::Uuid::new_v4();
        let receipt = WorkerOutcomeReceipt {
            summary: "done".to_string(),
            artifact_outcomes: vec![ArtifactOutcome {
                path: "/tmp/report.txt".to_string(),
                suggested_name: Some("report.txt".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };

        sqlx::query(
            "INSERT INTO worker_runs (id, channel_id, status, result, outcome_receipt_json) VALUES (?, 'ch-1', 'done', 'ok', ?)",
        )
        .bind(worker_id.to_string())
        .bind(serde_json::to_string(&receipt).expect("receipt json"))
        .execute(&pool)
        .await
        .expect("insert");

        logger
            .update_worker_receipt_artifact_delivery(
                &worker_id.to_string(),
                &[ArtifactDeliveryUpdate {
                    path: "/tmp/report.txt".to_string(),
                    delivery_status: "delivered".to_string(),
                    delivery_target: Some("telegram:123".to_string()),
                }],
            )
            .await
            .expect("update should succeed");

        let updated_json: String =
            sqlx::query_scalar("SELECT outcome_receipt_json FROM worker_runs WHERE id = ?")
                .bind(worker_id.to_string())
                .fetch_one(&pool)
                .await
                .expect("fetch receipt json");
        let updated: WorkerOutcomeReceipt =
            serde_json::from_str(&updated_json).expect("decode updated receipt");

        assert_eq!(
            updated.artifact_outcomes[0].delivery_status.as_deref(),
            Some("delivered")
        );
        assert_eq!(
            updated.artifact_outcomes[0].delivery_target.as_deref(),
            Some("telegram:123")
        );
    }
}
