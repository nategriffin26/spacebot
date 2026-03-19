//! Request-group API endpoints for swarm and multi-lane inspection.

use super::state::ApiState;

use crate::conversation::history::ProcessRunLogger;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Deserialize)]
pub(super) struct RequestGroupListQuery {
    agent_id: String,
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    channel_id: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

fn default_limit() -> i64 {
    50
}

#[derive(Deserialize)]
pub(super) struct RequestGroupDetailQuery {
    agent_id: String,
    request_group_id: String,
}

#[derive(Serialize)]
pub(super) struct RequestGroupListResponse {
    request_groups: Vec<RequestGroupListItem>,
}

#[derive(Serialize)]
pub(super) struct RequestGroupListItem {
    id: String,
    channel_id: String,
    mode: String,
    state: String,
    task_class: Option<String>,
    origin_user_text: String,
    worker_count: usize,
    branch_count: usize,
    merge_ready_workers: usize,
    blocked_workers: usize,
    updated_at: String,
}

pub(super) async fn list_request_groups(
    State(state): State<Arc<ApiState>>,
    Query(query): Query<RequestGroupListQuery>,
) -> Result<Json<RequestGroupListResponse>, StatusCode> {
    let pools = state.agent_pools.load();
    let pool = pools.get(&query.agent_id).ok_or(StatusCode::NOT_FOUND)?;
    let logger = ProcessRunLogger::new(pool.clone());

    let groups = logger
        .list_request_groups(
            &query.agent_id,
            query.limit.clamp(1, 200),
            query.channel_id.as_deref(),
            query.state.as_deref(),
        )
        .await
        .map_err(|error| {
            tracing::warn!(%error, agent_id = %query.agent_id, "failed to list request groups");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let mut items = Vec::with_capacity(groups.len());
    for group in groups {
        let detail = logger
            .get_request_group_detail(&group.id)
            .await
            .map_err(|error| {
                tracing::warn!(%error, request_group_id = %group.id, "failed to load request group detail");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        let (worker_count, branch_count, merge_ready_workers, blocked_workers) = match detail {
            Some(detail) => (
                detail.workers.len(),
                detail.branches.len(),
                detail
                    .workers
                    .iter()
                    .filter(|worker| {
                        worker
                            .receipt
                            .as_ref()
                            .is_some_and(|receipt| receipt.merge_ready)
                    })
                    .count(),
                detail
                    .workers
                    .iter()
                    .filter(|worker| {
                        worker
                            .receipt
                            .as_ref()
                            .is_some_and(|receipt| !receipt.merge_ready)
                    })
                    .count(),
            ),
            None => (0, 0, 0, 0),
        };

        items.push(RequestGroupListItem {
            id: group.id,
            channel_id: group.channel_id,
            mode: group.mode,
            state: group.state,
            task_class: group.task_class,
            origin_user_text: group.origin_user_text,
            worker_count,
            branch_count,
            merge_ready_workers,
            blocked_workers,
            updated_at: group.updated_at,
        });
    }

    Ok(Json(RequestGroupListResponse {
        request_groups: items,
    }))
}

pub(super) async fn request_group_detail(
    State(state): State<Arc<ApiState>>,
    Query(query): Query<RequestGroupDetailQuery>,
) -> Result<Json<crate::conversation::history::RequestGroupDetail>, StatusCode> {
    let pools = state.agent_pools.load();
    let pool = pools.get(&query.agent_id).ok_or(StatusCode::NOT_FOUND)?;
    let logger = ProcessRunLogger::new(pool.clone());

    let detail = logger
        .get_request_group_detail(&query.request_group_id)
        .await
        .map_err(|error| {
            tracing::warn!(%error, request_group_id = %query.request_group_id, "failed to load request group detail");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(detail))
}
