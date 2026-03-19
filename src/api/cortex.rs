use super::state::ApiState;

use crate::agent::cortex::{CortexEvent, CortexLogger};
use crate::agent::cortex_chat::{
    CortexChatEvent, CortexChatMessage, CortexChatSendError, CortexChatStore, CortexChatThread,
};
use crate::memory::{SearchConfig, SearchMode};

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Sse;
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::Arc;

#[derive(Serialize)]
pub(super) struct CortexEventsResponse {
    events: Vec<CortexEvent>,
    total: i64,
}

#[derive(Serialize)]
pub(super) struct CortexSummaryResponse {
    recent_events: Vec<CortexEvent>,
    autonomy_cycles: Vec<CortexEvent>,
    recent_findings: Vec<AutonomyFindingSummary>,
}

#[derive(Serialize)]
pub(super) struct AutonomyFindingSummary {
    id: String,
    memory_type: String,
    source: Option<String>,
    importance: f32,
    created_at: String,
    content: String,
}

#[derive(Serialize)]
pub(super) struct CortexChatMessagesResponse {
    messages: Vec<CortexChatMessage>,
    thread_id: String,
}

#[derive(Deserialize)]
pub(super) struct CortexChatMessagesQuery {
    agent_id: String,
    /// If omitted, loads the latest thread.
    thread_id: Option<String>,
    #[serde(default = "default_cortex_chat_limit")]
    limit: i64,
}

fn default_cortex_chat_limit() -> i64 {
    50
}

#[derive(Deserialize)]
pub(super) struct CortexChatSendRequest {
    agent_id: String,
    thread_id: String,
    message: String,
    channel_id: Option<String>,
    task_number: Option<i64>,
}

#[derive(Deserialize)]
pub(super) struct CortexEventsQuery {
    agent_id: String,
    #[serde(default = "default_cortex_events_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
    #[serde(default)]
    event_type: Option<String>,
    #[serde(default)]
    include_heartbeat: bool,
}

fn default_cortex_events_limit() -> i64 {
    50
}

#[derive(Deserialize)]
pub(super) struct CortexSummaryQuery {
    agent_id: String,
    #[serde(default = "default_cortex_summary_limit")]
    limit: i64,
}

fn default_cortex_summary_limit() -> i64 {
    10
}

fn map_cortex_chat_send_error(error: &CortexChatSendError) -> StatusCode {
    match error {
        CortexChatSendError::Busy => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Load persisted cortex chat history for a thread.
/// If no thread_id is provided, loads the latest thread.
/// If no threads exist, returns an empty list with a fresh thread_id.
pub(super) async fn cortex_chat_messages(
    State(state): State<Arc<ApiState>>,
    Query(query): Query<CortexChatMessagesQuery>,
) -> Result<Json<CortexChatMessagesResponse>, StatusCode> {
    let pools = state.agent_pools.load();
    let pool = pools.get(&query.agent_id).ok_or(StatusCode::NOT_FOUND)?;
    let store = CortexChatStore::new(pool.clone());

    let thread_id = if let Some(tid) = query.thread_id {
        tid
    } else {
        store
            .latest_thread_id()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
    };

    let messages = store
        .load_history(&thread_id, query.limit.min(200))
        .await
        .map_err(|error| {
            tracing::warn!(%error, agent_id = %query.agent_id, "failed to load cortex chat history");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(CortexChatMessagesResponse {
        messages,
        thread_id,
    }))
}

/// Send a message to cortex chat. Returns an SSE stream with activity events.
///
/// The stream emits:
/// - `thinking` — cortex is processing
/// - `text_delta` — partial text from the model stream
/// - `tool_started` — a tool call began
/// - `tool_completed` — a tool call finished (with result preview)
/// - `done` — full response text
/// - `error` — if something went wrong
pub(super) async fn cortex_chat_send(
    State(state): State<Arc<ApiState>>,
    axum::Json(request): axum::Json<CortexChatSendRequest>,
) -> Result<Sse<impl Stream<Item = Result<axum::response::sse::Event, Infallible>>>, StatusCode> {
    let sessions = state.cortex_chat_sessions.load();
    let session = sessions
        .get(&request.agent_id)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)?;

    let thread_id = request.thread_id;
    let message = request.message;
    let channel_id = request.channel_id;
    let task_number = request.task_number;

    let channel_ref = channel_id.as_deref();
    let mut event_rx = session
        .send_message_with_events(&thread_id, &message, channel_ref, task_number)
        .await
        .map_err(|error| {
            let status = map_cortex_chat_send_error(&error);
            if status == StatusCode::INTERNAL_SERVER_ERROR {
                tracing::warn!(%error, "failed to start cortex chat send");
            }
            status
        })?;

    let stream = async_stream::stream! {
        yield Ok(axum::response::sse::Event::default()
            .event("thinking")
            .data("{}"));

        while let Some(event) = event_rx.recv().await {
            let event_name = match &event {
                CortexChatEvent::Thinking => "thinking",
                CortexChatEvent::TextDelta { .. } => "text_delta",
                CortexChatEvent::ToolStarted { .. } => "tool_started",
                CortexChatEvent::ToolCompleted { .. } => "tool_completed",
                CortexChatEvent::Done { .. } => "done",
                CortexChatEvent::Error { .. } => "error",
            };
            if let Ok(json) = serde_json::to_string(&event) {
                yield Ok(axum::response::sse::Event::default()
                    .event(event_name)
                    .data(json));
            }
        }
    };

    Ok(Sse::new(stream))
}

// -- Thread management --

#[derive(Serialize)]
pub(super) struct CortexChatThreadsResponse {
    threads: Vec<CortexChatThread>,
}

#[derive(Deserialize)]
pub(super) struct CortexChatThreadsQuery {
    agent_id: String,
}

#[derive(Deserialize)]
pub(super) struct CortexChatDeleteThreadRequest {
    agent_id: String,
    thread_id: String,
}

/// List all cortex chat threads for an agent, newest first.
pub(super) async fn cortex_chat_threads(
    State(state): State<Arc<ApiState>>,
    Query(query): Query<CortexChatThreadsQuery>,
) -> Result<Json<CortexChatThreadsResponse>, StatusCode> {
    let pools = state.agent_pools.load();
    let pool = pools.get(&query.agent_id).ok_or(StatusCode::NOT_FOUND)?;
    let store = CortexChatStore::new(pool.clone());

    let threads = store.list_threads().await.map_err(|error| {
        tracing::warn!(%error, agent_id = %query.agent_id, "failed to list cortex chat threads");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(CortexChatThreadsResponse { threads }))
}

/// Delete a cortex chat thread and all its messages.
pub(super) async fn cortex_chat_delete_thread(
    State(state): State<Arc<ApiState>>,
    axum::Json(request): axum::Json<CortexChatDeleteThreadRequest>,
) -> Result<StatusCode, StatusCode> {
    let pools = state.agent_pools.load();
    let pool = pools.get(&request.agent_id).ok_or(StatusCode::NOT_FOUND)?;
    let store = CortexChatStore::new(pool.clone());

    let deleted = store.delete_thread(&request.thread_id).await.map_err(|error| {
        tracing::warn!(%error, agent_id = %request.agent_id, thread_id = %request.thread_id, "failed to delete cortex chat thread");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if deleted == 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(StatusCode::NO_CONTENT)
}

/// List cortex events for an agent with optional type filter, newest first.
pub(super) async fn cortex_events(
    State(state): State<Arc<ApiState>>,
    Query(query): Query<CortexEventsQuery>,
) -> Result<Json<CortexEventsResponse>, StatusCode> {
    let pools = state.agent_pools.load();
    let pool = pools.get(&query.agent_id).ok_or(StatusCode::NOT_FOUND)?;
    let logger = CortexLogger::new(pool.clone());

    let limit = query.limit.min(200);
    let event_type_ref = query.event_type.as_deref();

    let mut events = logger
        .load_events(limit.saturating_mul(2), query.offset, event_type_ref)
        .await
        .map_err(|error| {
            tracing::warn!(%error, agent_id = %query.agent_id, "failed to load cortex events");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if !query.include_heartbeat && event_type_ref.is_none() {
        events.retain(|event| event.event_type != "heartbeat");
        events.truncate(limit as usize);
    } else if events.len() > limit as usize {
        events.truncate(limit as usize);
    }

    let total = logger.count_events(event_type_ref).await.map_err(|error| {
        tracing::warn!(%error, agent_id = %query.agent_id, "failed to count cortex events");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(CortexEventsResponse { events, total }))
}

pub(super) async fn cortex_summary(
    State(state): State<Arc<ApiState>>,
    Query(query): Query<CortexSummaryQuery>,
) -> Result<Json<CortexSummaryResponse>, StatusCode> {
    let pools = state.agent_pools.load();
    let pool = pools.get(&query.agent_id).ok_or(StatusCode::NOT_FOUND)?;
    let logger = CortexLogger::new(pool.clone());
    let limit = query.limit.clamp(1, 50);

    let recent_events = logger
        .load_events(limit.saturating_mul(3), 0, None)
        .await
        .map_err(|error| {
            tracing::warn!(%error, agent_id = %query.agent_id, "failed to load cortex summary events");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .into_iter()
        .filter(|event| event.event_type != "heartbeat")
        .take(limit as usize)
        .collect::<Vec<_>>();

    let autonomy_cycles = logger
        .load_events(limit.saturating_mul(4), 0, None)
        .await
        .map_err(|error| {
            tracing::warn!(%error, agent_id = %query.agent_id, "failed to load autonomy cycle events");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .into_iter()
        .filter(|event| event.event_type.starts_with("autonomy_cycle_"))
        .take(limit as usize)
        .collect::<Vec<_>>();

    let memory_searches = state.memory_searches.load();
    let memory_search = memory_searches
        .get(&query.agent_id)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)?;
    let findings = memory_search
        .search(
            "",
            &SearchConfig {
                mode: SearchMode::Recent,
                source: Some("cortex:autonomy".to_string()),
                max_results: limit as usize,
                ..Default::default()
            },
        )
        .await
        .map_err(|error| {
            tracing::warn!(%error, agent_id = %query.agent_id, "failed to load autonomy findings");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(CortexSummaryResponse {
        recent_events,
        autonomy_cycles,
        recent_findings: findings
            .into_iter()
            .map(|finding| AutonomyFindingSummary {
                id: finding.memory.id,
                memory_type: finding.memory.memory_type.to_string(),
                source: finding.memory.source,
                importance: finding.memory.importance,
                created_at: finding.memory.created_at.to_rfc3339(),
                content: finding.memory.content,
            })
            .collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::map_cortex_chat_send_error;
    use crate::agent::cortex_chat::CortexChatSendError;
    use axum::http::StatusCode;

    #[test]
    fn maps_busy_send_error_to_conflict() {
        assert_eq!(
            map_cortex_chat_send_error(&CortexChatSendError::Busy),
            StatusCode::CONFLICT
        );
    }

    #[test]
    fn maps_database_send_error_to_internal_server_error() {
        assert_eq!(
            map_cortex_chat_send_error(&CortexChatSendError::Database(sqlx::Error::RowNotFound)),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
