//! Cortex chat: persistent admin conversation with the cortex.
//!
//! One session per agent. The admin talks to the cortex interactively,
//! with the full toolset (memory, shell, file, exec, browser, web search).
//! When opened on a channel page, the channel's recent history is injected
//! into the system prompt as context.

use crate::conversation::history::ProcessRunLogger;
use crate::hooks::SpacebotHook;
use crate::llm::SpacebotModel;
use crate::{AgentDeps, ProcessId, ProcessType};

use futures::StreamExt as _;
use rig::agent::MultiTurnStreamItem;
use rig::agent::{AgentBuilder, HookAction, PromptHook, ToolCallHookAction};
use rig::completion::{
    AssistantContent, CompletionModel, CompletionResponse, GetTokenUsage, Message, Prompt,
};
use rig::streaming::StreamingPrompt;
use rig::tool::server::ToolServerHandle;
use serde::Serialize;
use sqlx::SqlitePool;
use tokio::sync::mpsc;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};

/// A persisted cortex chat message.
#[derive(Debug, Clone, Serialize)]
pub struct CortexChatMessage {
    pub id: String,
    pub thread_id: String,
    pub role: String,
    pub content: String,
    pub channel_context: Option<String>,
    pub created_at: String,
}

/// Events emitted during a cortex chat response (sent via SSE to the client).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CortexChatEvent {
    /// The cortex is processing (before LLM response).
    Thinking,
    /// A partial text delta arrived from the model stream.
    TextDelta { text_delta: String },
    /// A tool call started.
    ToolStarted { tool: String },
    /// A tool call completed.
    ToolCompleted {
        tool: String,
        result_preview: String,
    },
    /// The full response is ready.
    Done { full_text: String },
    /// An error occurred.
    Error { message: String },
}

#[derive(Debug, thiserror::Error)]
pub enum CortexChatSendError {
    #[error("cortex chat session is busy")]
    Busy,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Prompt(#[from] crate::error::Error),
}

/// Prompt hook that forwards tool events to an mpsc channel for SSE streaming.
#[derive(Clone)]
struct CortexChatHook {
    event_tx: mpsc::Sender<CortexChatEvent>,
    spacebot_hook: SpacebotHook,
}

impl CortexChatHook {
    fn new(event_tx: mpsc::Sender<CortexChatEvent>, spacebot_hook: SpacebotHook) -> Self {
        Self {
            event_tx,
            spacebot_hook,
        }
    }

    async fn send(&self, event: CortexChatEvent) {
        let _ = self.event_tx.send(event).await;
    }

    fn try_send(&self, event: CortexChatEvent) {
        if let Err(error) = self.event_tx.try_send(event)
            && matches!(error, tokio::sync::mpsc::error::TrySendError::Full(_))
        {
            tracing::debug!("dropping cortex chat streaming event because the SSE channel is full");
        }
    }
}

fn try_acquire_send_lock(
    send_lock: &Arc<Mutex<()>>,
) -> std::result::Result<tokio::sync::OwnedMutexGuard<()>, CortexChatSendError> {
    send_lock
        .clone()
        .try_lock_owned()
        .map_err(|_| CortexChatSendError::Busy)
}

async fn persist_and_emit_cortex_chat_error(
    store: &CortexChatStore,
    event_tx: &mpsc::Sender<CortexChatEvent>,
    thread_id: &str,
    channel_ref: Option<&str>,
    message: String,
) {
    if let Err(error) = store
        .save_message(thread_id, "assistant", &message, channel_ref)
        .await
    {
        tracing::error!(%error, %thread_id, "failed to persist cortex chat error message");
    }
    if let Err(error) = event_tx.send(CortexChatEvent::Error { message }).await {
        tracing::debug!(%error, %thread_id, "failed to emit cortex chat error event");
    }
}

async fn persist_and_emit_cortex_chat_done(
    store: &CortexChatStore,
    event_tx: &mpsc::Sender<CortexChatEvent>,
    thread_id: &str,
    channel_ref: Option<&str>,
    response: String,
) {
    if let Err(error) = store
        .save_message(thread_id, "assistant", &response, channel_ref)
        .await
    {
        tracing::error!(%error, %thread_id, "failed to persist cortex chat response");
    }
    if let Err(error) = event_tx
        .send(CortexChatEvent::Done {
            full_text: response,
        })
        .await
    {
        tracing::debug!(%error, %thread_id, "failed to emit cortex chat done event");
    }
}

fn streaming_error_to_prompt_error(
    error: rig::agent::StreamingError,
) -> rig::completion::PromptError {
    match error {
        rig::agent::StreamingError::Prompt(error) => *error,
        rig::agent::StreamingError::Completion(error) => {
            rig::completion::PromptError::CompletionError(error)
        }
        rig::agent::StreamingError::Tool(error) => rig::completion::PromptError::ToolError(error),
    }
}

fn missing_stream_final_response_error() -> rig::completion::PromptError {
    rig::completion::PromptError::CompletionError(rig::completion::CompletionError::ProviderError(
        "stream ended without final response".to_string(),
    ))
}

async fn prompt_once_streaming<M, P>(
    agent: &rig::agent::Agent<M>,
    hook: P,
    history: Vec<Message>,
    prompt: &str,
) -> std::result::Result<String, rig::completion::PromptError>
where
    M: CompletionModel + 'static,
    M::StreamingResponse: GetTokenUsage,
    P: rig::agent::PromptHook<M> + 'static,
{
    let mut stream = agent
        .stream_prompt(prompt)
        .with_hook(hook)
        .with_history(history)
        .await;

    while let Some(item) = stream.next().await {
        match item {
            Ok(MultiTurnStreamItem::FinalResponse(final_response)) => {
                return Ok(final_response.response().to_string());
            }
            Ok(_) => {}
            Err(error) => return Err(streaming_error_to_prompt_error(error)),
        }
    }

    Err(missing_stream_final_response_error())
}

impl<M: CompletionModel> PromptHook<M> for CortexChatHook {
    async fn on_text_delta(&self, text_delta: &str, aggregated_text: &str) -> HookAction {
        let action = <SpacebotHook as PromptHook<M>>::on_text_delta(
            &self.spacebot_hook,
            text_delta,
            aggregated_text,
        )
        .await;
        if !matches!(action, HookAction::Continue) {
            return action;
        }

        self.try_send(CortexChatEvent::TextDelta {
            text_delta: text_delta.to_string(),
        });
        HookAction::Continue
    }

    async fn on_tool_call(
        &self,
        tool_name: &str,
        tool_call_id: Option<String>,
        internal_call_id: &str,
        args: &str,
    ) -> ToolCallHookAction {
        let action = <SpacebotHook as PromptHook<M>>::on_tool_call(
            &self.spacebot_hook,
            tool_name,
            tool_call_id,
            internal_call_id,
            args,
        )
        .await;
        if !matches!(action, ToolCallHookAction::Continue) {
            return action;
        }

        self.send(CortexChatEvent::ToolStarted {
            tool: tool_name.to_string(),
        })
        .await;
        action
    }

    async fn on_tool_result(
        &self,
        tool_name: &str,
        _tool_call_id: Option<String>,
        internal_call_id: &str,
        _args: &str,
        result: &str,
    ) -> HookAction {
        let guard_action = self.spacebot_hook.guard_tool_result(tool_name, result);
        if !matches!(guard_action, HookAction::Continue) {
            return guard_action;
        }
        let preview = crate::tools::truncate_utf8_ellipsis(result, 200);
        self.spacebot_hook
            .emit_tool_completed_event_from_capped(tool_name, preview.clone());
        self.spacebot_hook
            .record_tool_result_metrics(tool_name, internal_call_id);
        self.send(CortexChatEvent::ToolCompleted {
            tool: tool_name.to_string(),
            result_preview: preview,
        })
        .await;
        HookAction::Continue
    }

    async fn on_completion_call(&self, prompt: &Message, history: &[Message]) -> HookAction {
        <SpacebotHook as PromptHook<M>>::on_completion_call(&self.spacebot_hook, prompt, history)
            .await
    }

    async fn on_completion_response(
        &self,
        prompt: &Message,
        response: &CompletionResponse<M::Response>,
    ) -> HookAction {
        <SpacebotHook as PromptHook<M>>::on_completion_response(
            &self.spacebot_hook,
            prompt,
            response,
        )
        .await
    }
}

/// SQLite CRUD for cortex chat messages.
#[derive(Debug, Clone)]
pub struct CortexChatStore {
    pool: SqlitePool,
}

#[derive(sqlx::FromRow)]
struct ChatMessageRow {
    id: String,
    thread_id: String,
    role: String,
    content: String,
    channel_context: Option<String>,
    created_at: chrono::NaiveDateTime,
}

impl ChatMessageRow {
    fn into_message(self) -> CortexChatMessage {
        CortexChatMessage {
            id: self.id,
            thread_id: self.thread_id,
            role: self.role,
            content: self.content,
            channel_context: self.channel_context,
            created_at: self.created_at.and_utc().to_rfc3339(),
        }
    }
}

impl CortexChatStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Load chat history for a thread, newest first, then reverse to chronological order.
    pub async fn load_history(
        &self,
        thread_id: &str,
        limit: i64,
    ) -> Result<Vec<CortexChatMessage>, sqlx::Error> {
        let rows: Vec<ChatMessageRow> = sqlx::query_as(
            "SELECT id, thread_id, role, content, channel_context, created_at \
             FROM cortex_chat_messages WHERE thread_id = ? ORDER BY created_at DESC LIMIT ?",
        )
        .bind(thread_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        let mut messages: Vec<CortexChatMessage> =
            rows.into_iter().map(|row| row.into_message()).collect();
        messages.reverse();
        Ok(messages)
    }

    /// Save a message to a thread. Returns the generated ID.
    pub async fn save_message(
        &self,
        thread_id: &str,
        role: &str,
        content: &str,
        channel_context: Option<&str>,
    ) -> Result<String, sqlx::Error> {
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO cortex_chat_messages (id, thread_id, role, content, channel_context) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(thread_id)
        .bind(role)
        .bind(content)
        .bind(channel_context)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// Get the most recent thread_id, or None if no threads exist.
    pub async fn latest_thread_id(&self) -> Result<Option<String>, sqlx::Error> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT thread_id FROM cortex_chat_messages ORDER BY created_at DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.0))
    }
}

/// The cortex chat session for a single agent.
///
/// Holds the deps, tool server, store, and a mutex to prevent concurrent sends.
/// Tracked worker entry: maps a cortex-spawned worker to the thread it was
/// spawned from so the event loop can deliver results to the right conversation.
#[derive(Debug, Clone)]
pub struct TrackedWorker {
    pub thread_id: String,
    pub channel_context: Option<String>,
}

pub struct CortexChatSession {
    pub deps: AgentDeps,
    pub tool_server: ToolServerHandle,
    pub store: CortexChatStore,
    /// Whether factory tools (agent creation/refinement) are available.
    pub factory_enabled: bool,
    /// Prevent concurrent sends — only one request at a time per agent.
    send_lock: Arc<Mutex<()>>,
    /// Shared context between the session and DetachedSpawnWorkerTool.
    cortex_ctx: crate::tools::spawn_worker::CortexChatContext,
}

impl CortexChatSession {
    fn truncate_task_context_text(value: &str, max_bytes: usize) -> &str {
        if value.len() <= max_bytes {
            return value;
        }

        let mut end = max_bytes;
        while end > 0 && !value.is_char_boundary(end) {
            end -= 1;
        }
        &value[..end]
    }

    pub fn new(
        deps: AgentDeps,
        tool_server: ToolServerHandle,
        store: CortexChatStore,
        cortex_ctx: crate::tools::spawn_worker::CortexChatContext,
    ) -> Self {
        Self {
            deps,
            tool_server,
            store,
            cortex_ctx,
            factory_enabled: false,
            send_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Create a default `CortexChatContext` for use when building the tool server
    /// and session together. Pass the same context to both.
    pub fn create_context() -> crate::tools::spawn_worker::CortexChatContext {
        crate::tools::spawn_worker::CortexChatContext {
            current_thread_id: Arc::new(RwLock::new(None)),
            current_channel_context: Arc::new(RwLock::new(None)),
            current_task_number: Arc::new(RwLock::new(None)),
            tracked_workers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Enable factory tools for this session (agent creation/refinement).
    pub fn with_factory(mut self, enabled: bool) -> Self {
        self.factory_enabled = enabled;
        self
    }

    /// Get the cortex chat context (used by DetachedSpawnWorkerTool).
    pub fn cortex_context(&self) -> crate::tools::spawn_worker::CortexChatContext {
        self.cortex_ctx.clone()
    }

    /// Start the background event loop that listens for worker completions and
    /// auto-triggers follow-up cortex chat turns.
    pub fn start_event_loop(self: &Arc<Self>) {
        let session = Arc::clone(self);
        let mut event_rx = session.deps.event_tx.subscribe();
        let agent_id = session.deps.agent_id.clone();

        tokio::spawn(async move {
            loop {
                let event = match event_rx.recv().await {
                    Ok(event) => event,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                        tracing::debug!(count, "cortex chat event loop lagged");
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };

                let (worker_id, result, success) = match &event {
                    ProcessEvent::WorkerComplete {
                        agent_id: event_agent_id,
                        worker_id,
                        result,
                        success,
                        ..
                    } if *event_agent_id == agent_id => (*worker_id, result.clone(), *success),
                    _ => continue,
                };

                let tracked: Option<TrackedWorker> = session
                    .cortex_ctx
                    .tracked_workers
                    .write()
                    .await
                    .remove(&worker_id);
                let Some(tracked) = tracked else {
                    continue;
                };

                // Persist completion to worker_runs (no channel handler does this
                // for channel_id: None workers).
                let run_logger = ProcessRunLogger::new(session.deps.sqlite_pool.clone());
                run_logger.log_worker_completed(worker_id, &result, success);

                tracing::info!(
                    %worker_id,
                    thread_id = %tracked.thread_id,
                    success,
                    "cortex chat worker completed, auto-triggering follow-up"
                );

                let status_label = if success { "completed" } else { "failed" };
                let retrigger_message = format!("[Worker {worker_id} {status_label}]\n\n{result}");

                let channel_ref = tracked.channel_context.as_deref();

                // The send_lock may be held if the admin is mid-conversation.
                // Wait for it rather than dropping the result.
                let event_rx = match session
                    .send_message_blocking(
                        &tracked.thread_id,
                        &retrigger_message,
                        channel_ref,
                        None,
                    )
                    .await
                {
                    Ok(rx) => rx,
                    Err(error) => {
                        tracing::warn!(
                            %worker_id,
                            %error,
                            "failed to auto-trigger cortex chat for worker result"
                        );
                        continue;
                    }
                };

                // Drain the event stream and emit the final response through the
                // ProcessEvent pipeline so the frontend SSE picks it up.
                Self::drain_auto_trigger_events(event_rx, &session.deps, &tracked.thread_id).await;
            }
        });
    }

    /// Drain events from an auto-triggered cortex chat turn and emit the final
    /// response as a `ProcessEvent::CortexChatUpdate`.
    async fn drain_auto_trigger_events(
        mut event_rx: mpsc::Receiver<CortexChatEvent>,
        deps: &AgentDeps,
        thread_id: &str,
    ) {
        while let Some(event) = event_rx.recv().await {
            match event {
                CortexChatEvent::Done {
                    full_text,
                    tool_calls,
                } => {
                    let tool_calls_json = if tool_calls.is_empty() {
                        None
                    } else {
                        serde_json::to_string(&tool_calls).ok()
                    };
                    let _ = deps.event_tx.send(ProcessEvent::CortexChatUpdate {
                        agent_id: deps.agent_id.clone(),
                        thread_id: thread_id.to_string(),
                        content: full_text,
                        tool_calls_json,
                    });
                }
                CortexChatEvent::Error { message } => {
                    tracing::warn!(
                        thread_id,
                        error = %message,
                        "cortex chat auto-trigger turn errored"
                    );
                }
                _ => {}
            }
        }
    }

    /// Send a message and stream events (tool calls, completion) back via an mpsc channel.
    ///
    /// Returns a receiver that yields `CortexChatEvent` items as the agent works.
    /// The agent runs in a spawned task so the caller can forward events to SSE
    /// without blocking.
    pub async fn send_message_with_events(
        self: &Arc<Self>,
        thread_id: &str,
        user_text: &str,
        channel_context_id: Option<&str>,
        task_number: Option<i64>,
    ) -> std::result::Result<mpsc::Receiver<CortexChatEvent>, CortexChatSendError> {
        let send_guard = try_acquire_send_lock(&self.send_lock)?;
        self.send_message_impl(
            send_guard,
            thread_id,
            user_text,
            channel_context_id,
            task_number,
        )
        .await
    }

    /// Like `send_message_with_events` but waits for the send lock instead of
    /// returning `Busy`. Used by the background event loop for auto-triggered turns.
    /// The input message is NOT persisted — only the assistant's synthesis gets saved.
    async fn send_message_blocking(
        self: &Arc<Self>,
        thread_id: &str,
        user_text: &str,
        channel_context_id: Option<&str>,
        task_number: Option<i64>,
    ) -> std::result::Result<mpsc::Receiver<CortexChatEvent>, CortexChatSendError> {
        let send_guard = self.send_lock.clone().lock_owned().await;
        self.send_message_inner(
            send_guard,
            thread_id,
            user_text,
            channel_context_id,
            task_number,
            false,
        )
        .await
    }

    async fn send_message_impl(
        self: &Arc<Self>,
        send_guard: tokio::sync::OwnedMutexGuard<()>,
        thread_id: &str,
        user_text: &str,
        channel_context_id: Option<&str>,
        task_number: Option<i64>,
    ) -> std::result::Result<mpsc::Receiver<CortexChatEvent>, CortexChatSendError> {
        self.send_message_inner(
            send_guard,
            thread_id,
            user_text,
            channel_context_id,
            task_number,
            true,
        )
        .await
    }

    /// Core send implementation. When `persist_input` is false the incoming
    /// message is sent to the LLM but not saved to the thread — used for
    /// auto-triggered worker results where only the assistant's synthesis
    /// should appear in history.
    async fn send_message_inner(
        self: &Arc<Self>,
        send_guard: tokio::sync::OwnedMutexGuard<()>,
        thread_id: &str,
        user_text: &str,
        channel_context_id: Option<&str>,
        task_number: Option<i64>,
        persist_input: bool,
    ) -> std::result::Result<mpsc::Receiver<CortexChatEvent>, CortexChatSendError> {
        // Update the shared context so DetachedSpawnWorkerTool knows which
        // thread to associate spawned workers with.
        *self.cortex_ctx.current_thread_id.write().await = Some(thread_id.to_string());
        *self.cortex_ctx.current_channel_context.write().await =
            channel_context_id.map(|s| s.to_string());
        *self.cortex_ctx.current_task_number.write().await = task_number;

        if persist_input {
            self.store
                .save_message(thread_id, "user", user_text, channel_context_id)
                .await?;
        }

        // Build the system prompt
        let system_prompt = self
            .build_system_prompt(channel_context_id, task_number)
            .await?;

        // Load chat history and convert to Rig messages.
        // When we persisted the input, the last message in history is the one
        // we just saved — skip it since Rig adds it via `agent.prompt()`.
        // When the input wasn't persisted (auto-triggered turns), include all
        // messages since none of them duplicate the prompt.
        let chat_messages = self.store.load_history(thread_id, 100).await?;
        let history_end = if persist_input {
            chat_messages.len().saturating_sub(1)
        } else {
            chat_messages.len()
        };
        let mut history: Vec<rig::message::Message> = Vec::new();
        for message in &chat_messages[..history_end] {
            match message.role.as_str() {
                "user" => {
                    history.push(rig::message::Message::from(message.content.as_str()));
                }
                "assistant" => {
                    let content = AssistantContent::from(message.content.clone());
                    history.push(rig::message::Message::from(content));
                }
                _ => {}
            }
        }

        // Resolve model and build agent
        let routing = self.deps.runtime_config.routing.load();
        let model_name = routing.resolve(ProcessType::Cortex, None).to_string();
        let model = SpacebotModel::make(&self.deps.llm_manager, &model_name)
            .with_context(self.deps.agent_id.as_ref(), "cortex")
            .with_rig_alignment((**self.deps.runtime_config.rig_alignment.load()).clone())
            .with_routing(routing.as_ref().clone());

        let agent = AgentBuilder::new(model)
            .preamble(&system_prompt)
            .default_max_turns(50)
            .tool_server_handle(self.tool_server.clone())
            .build();

        let (event_tx, event_rx) = mpsc::channel(256);
        let spacebot_hook = SpacebotHook::new(
            self.deps.agent_id.clone(),
            ProcessId::Worker(uuid::Uuid::new_v4()),
            ProcessType::Cortex,
            channel_context_id.map(std::sync::Arc::<str>::from),
            self.deps.event_tx.clone(),
        );
        let hook = CortexChatHook::new(event_tx.clone(), spacebot_hook);

        // Clone what the spawned task needs
        let user_text = user_text.to_string();
        let thread_id = thread_id.to_string();
        let channel_context_id = channel_context_id.map(|s| s.to_string());
        let store = self.store.clone();
        let prompt_timeout = Duration::from_secs(
            self.deps
                .runtime_config
                .cortex
                .load()
                .branch_timeout_secs
                .max(1),
        );
        let streaming_enabled = self
            .deps
            .runtime_config
            .rig_alignment
            .load()
            .cortex_chat_streaming;

        tokio::spawn(async move {
            let _send_guard = send_guard;
            let channel_ref = channel_context_id.as_deref();
            let prompt_result = tokio::time::timeout(prompt_timeout, async move {
                if streaming_enabled {
                    prompt_once_streaming(&agent, hook.clone(), history, &user_text).await
                } else {
                    agent
                        .prompt(&user_text)
                        .with_hook(hook.clone())
                        .with_history(&mut history)
                        .await
                }
            })
            .await;

            match prompt_result {
                Ok(Ok(response)) => {
                    persist_and_emit_cortex_chat_done(
                        &store,
                        &event_tx,
                        &thread_id,
                        channel_ref,
                        response,
                    )
                    .await;
                }
                Ok(Err(error)) => {
                    let error_text = format!("Cortex chat error: {error}");
                    persist_and_emit_cortex_chat_error(
                        &store,
                        &event_tx,
                        &thread_id,
                        channel_ref,
                        error_text,
                    )
                    .await;
                }
                Err(_) => {
                    tracing::warn!(
                        timeout_secs = prompt_timeout.as_secs(),
                        "cortex chat prompt timed out"
                    );
                    let error_text = format!(
                        "Cortex chat timed out after {}s while waiting for a model response.",
                        prompt_timeout.as_secs()
                    );
                    persist_and_emit_cortex_chat_error(
                        &store,
                        &event_tx,
                        &thread_id,
                        channel_ref,
                        error_text,
                    )
                    .await;
                }
            }
        });

        Ok(event_rx)
    }

    async fn build_system_prompt(
        &self,
        channel_context_id: Option<&str>,
        task_number: Option<i64>,
    ) -> crate::error::Result<String> {
        let runtime_config = &self.deps.runtime_config;
        let prompt_engine = runtime_config.prompts.load();

        let identity_context = runtime_config.identity.load().render();
        let agents_manifest = crate::self_awareness::agents_manifest_for_prompt();
        let changelog_highlights = crate::self_awareness::changelog_highlights();
        let runtime_config_snapshot = crate::self_awareness::runtime_snapshot_pretty(
            self.deps.agent_id.as_ref(),
            runtime_config,
        );

        let context_content = runtime_config.memory_bulletin.load().to_string();

        let browser_enabled = runtime_config.browser_config.load().enabled;
        let web_search_enabled = runtime_config.brave_search_key.load().is_some();
        let opencode_enabled = runtime_config.opencode.load().enabled;
        let worker_capabilities = prompt_engine.render_worker_capabilities(
            browser_enabled,
            web_search_enabled,
            opencode_enabled,
        )?;

        // Load channel transcript if a channel context is active
        let channel_transcript = if let Some(channel_id) = channel_context_id {
            self.load_channel_transcript(channel_id).await
        } else {
            None
        };
        let task_context = if let Some(number) = task_number {
            self.load_task_context(number).await
        } else {
            None
        };

        let empty_to_none = |s: String| if s.is_empty() { None } else { Some(s) };

        prompt_engine.render_cortex_chat_prompt(
            empty_to_none(identity_context),
            empty_to_none(context_content),
            channel_transcript,
            task_context,
            empty_to_none(agents_manifest),
            empty_to_none(changelog_highlights),
            empty_to_none(runtime_config_snapshot),
            worker_capabilities,
        )
    }

    async fn load_task_context(&self, task_number: i64) -> Option<String> {
        let task = match self
            .deps
            .task_store
            .get_by_number(self.deps.agent_id.as_ref(), task_number)
            .await
        {
            Ok(Some(task)) => task,
            Ok(None) => return None,
            Err(error) => {
                tracing::warn!(%error, task_number, "failed to load task for corpilot context");
                return None;
            }
        };

        let mut out = format!(
            "- task_number: {}\n- title: {}\n- status: {}\n- priority: {}\n- created_by: {}\n- created_at: {}\n- updated_at: {}\n",
            task.task_number,
            task.title,
            task.status,
            task.priority,
            task.created_by,
            task.created_at,
            task.updated_at,
        );

        if let Some(description) = task.description.as_deref()
            && !description.trim().is_empty()
        {
            out.push_str("\n### Description\n");
            let preview = Self::truncate_task_context_text(description, 4_000);
            out.push_str(preview);
            if preview.len() < description.len() {
                out.push_str("\n\n[description truncated]\n");
            }
            out.push('\n');
        }

        if !task.subtasks.is_empty() {
            out.push_str("\n### Subtasks\n");
            for (index, subtask) in task.subtasks.iter().enumerate() {
                let status = if subtask.completed { "done" } else { "open" };
                out.push_str(&format!("{}. [{}] {}\n", index + 1, status, subtask.title));
            }
        }

        if task.metadata != serde_json::json!({})
            && let Ok(pretty_metadata) = serde_json::to_string_pretty(&task.metadata)
        {
            out.push_str("\n### Metadata\n```json\n");
            out.push_str(&pretty_metadata);
            out.push_str("\n```\n");
        }

        if let Some(worker_id) = task.worker_id.as_deref() {
            out.push_str("\n### Active Worker\n");
            out.push_str(&format!("- worker_id: {}\n", worker_id));

            let logger = ProcessRunLogger::new(self.deps.sqlite_pool.clone());
            match logger
                .get_worker_detail(self.deps.agent_id.as_ref(), worker_id)
                .await
            {
                Ok(Some(worker)) => {
                    out.push_str(&format!(
                        "- status: {}\n- worker_type: {}\n- started_at: {}\n",
                        worker.status, worker.worker_type, worker.started_at
                    ));
                    if let Some(directory) = worker.directory.as_deref() {
                        out.push_str(&format!("- directory: {}\n", directory));
                    }
                    if let Some(result) = worker.result.as_deref()
                        && !result.trim().is_empty()
                    {
                        let preview = if result.len() > 800 {
                            let boundary = result.floor_char_boundary(800);
                            format!("{}...", &result[..boundary])
                        } else {
                            result.to_string()
                        };
                        out.push_str("\n### Worker Result Preview\n");
                        out.push_str(&preview);
                        out.push('\n');
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(%error, worker_id, "failed to load worker detail for corpilot context");
                }
            }
        }

        Some(out)
    }

    /// Load the last 50 messages from a channel as a formatted transcript.
    async fn load_channel_transcript(&self, channel_id: &str) -> Option<String> {
        let logger = ProcessRunLogger::new(self.deps.sqlite_pool.clone());

        match logger.load_channel_timeline(channel_id, 50, None).await {
            Ok(items) if !items.is_empty() => {
                let mut transcript = String::new();
                for item in &items {
                    match item {
                        crate::conversation::history::TimelineItem::Message {
                            role,
                            content,
                            sender_name,
                            ..
                        } => {
                            let name = sender_name.as_deref().unwrap_or(role);
                            transcript.push_str(&format!("**{name}**: {content}\n\n"));
                        }
                        crate::conversation::history::TimelineItem::BranchRun {
                            description,
                            conclusion,
                            ..
                        } => {
                            if let Some(conclusion) = conclusion {
                                transcript.push_str(&format!(
                                    "*[Branch: {description}]*: {conclusion}\n\n"
                                ));
                            }
                        }
                        crate::conversation::history::TimelineItem::WorkerRun {
                            task,
                            result,
                            ..
                        } => {
                            if let Some(result) = result {
                                transcript.push_str(&format!("*[Worker: {task}]*: {result}\n\n"));
                            }
                        }
                    }
                }
                Some(transcript)
            }
            Ok(_) => None,
            Err(error) => {
                tracing::warn!(%error, channel_id, "failed to load channel transcript for cortex chat");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CortexChatEvent, CortexChatHook, CortexChatSendError, CortexChatStore,
        missing_stream_final_response_error, persist_and_emit_cortex_chat_done,
        try_acquire_send_lock,
    };
    use crate::hooks::SpacebotHook;
    use crate::llm::SpacebotModel;
    use crate::{ProcessId, ProcessType};
    use rig::agent::{HookAction, PromptHook};
    use sqlx::SqlitePool;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Mutex;

    #[test]
    fn preview_utf8_truncates_on_char_boundary() {
        let text = "🙂🙂🙂";
        // For max_bytes=5: can fit "🙂" (4 bytes) but not "🙂..." (7 bytes)
        let preview = crate::tools::truncate_utf8_ellipsis(text, 5);
        assert_eq!(preview, "🙂");
        // For max_bytes=10: can fit "🙂..." (7 bytes)
        let preview = crate::tools::truncate_utf8_ellipsis(text, 10);
        assert_eq!(preview, "🙂...");
    }

    #[test]
    fn preview_utf8_keeps_short_text() {
        let text = "done";
        let preview = crate::tools::truncate_utf8_ellipsis(text, 200);
        assert_eq!(preview, text);
    }

    #[tokio::test]
    async fn send_lock_returns_busy_when_already_held() {
        let send_lock = Arc::new(Mutex::new(()));
        let _first_guard = try_acquire_send_lock(&send_lock).expect("first lock should succeed");
        let second = try_acquire_send_lock(&send_lock);
        assert!(matches!(second, Err(CortexChatSendError::Busy)));
    }

    #[tokio::test]
    async fn send_lock_released_after_timeout_path() {
        let send_lock = Arc::new(Mutex::new(()));
        let send_guard = try_acquire_send_lock(&send_lock).expect("first lock should succeed");
        let timed_task = tokio::spawn(async move {
            let _send_guard = send_guard;
            tokio::time::timeout(Duration::from_millis(5), std::future::pending::<()>()).await
        });
        let timeout_result = timed_task.await.expect("timeout task should complete");
        assert!(timeout_result.is_err(), "pending prompt should time out");

        let second = try_acquire_send_lock(&send_lock);
        assert!(
            second.is_ok(),
            "single-flight lock should be released after timeout path"
        );
    }

    #[tokio::test]
    async fn cortex_chat_streaming_persists_final() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should connect");
        sqlx::query(
            "CREATE TABLE cortex_chat_messages ( \
                id TEXT PRIMARY KEY, \
                thread_id TEXT NOT NULL, \
                role TEXT NOT NULL, \
                content TEXT NOT NULL, \
                channel_context TEXT, \
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP \
            )",
        )
        .execute(&pool)
        .await
        .expect("cortex chat table should be created");

        let store = CortexChatStore::new(pool);
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(8);
        let (process_event_tx, _process_event_rx) = tokio::sync::broadcast::channel(8);

        let spacebot_hook = SpacebotHook::new(
            Arc::from("agent"),
            ProcessId::Worker(uuid::Uuid::new_v4()),
            ProcessType::Cortex,
            None,
            process_event_tx,
        );
        let hook = CortexChatHook::new(event_tx.clone(), spacebot_hook);

        let hook_action = <CortexChatHook as PromptHook<SpacebotModel>>::on_text_delta(
            &hook,
            "partial ",
            "partial response",
        )
        .await;
        assert!(
            matches!(hook_action, HookAction::Continue),
            "streaming text deltas should continue"
        );

        let final_response = "final response".to_string();
        persist_and_emit_cortex_chat_done(&store, &event_tx, "thread-1", None, final_response)
            .await;

        let first_event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
            .await
            .expect("should receive first event")
            .expect("first event should exist");
        match first_event {
            CortexChatEvent::TextDelta { text_delta } => {
                assert_eq!(text_delta, "partial ");
            }
            other => panic!("expected TextDelta event, got {other:?}"),
        }

        let second_event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
            .await
            .expect("should receive done event")
            .expect("done event should exist");
        match second_event {
            CortexChatEvent::Done { full_text } => {
                assert_eq!(full_text, "final response");
            }
            other => panic!("expected Done event, got {other:?}"),
        }

        let history = store
            .load_history("thread-1", 10)
            .await
            .expect("history should load");
        assert_eq!(
            history.len(),
            1,
            "assistant final response should be persisted"
        );
        assert_eq!(history[0].role, "assistant");
        assert_eq!(history[0].content, "final response");
    }

    #[tokio::test]
    async fn text_delta_backpressure_does_not_timeout_prompt() {
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(1);
        event_tx
            .send(CortexChatEvent::Thinking)
            .await
            .expect("prefill event channel");
        let (process_event_tx, _process_event_rx) = tokio::sync::broadcast::channel(8);

        let spacebot_hook = SpacebotHook::new(
            Arc::from("agent"),
            ProcessId::Worker(uuid::Uuid::new_v4()),
            ProcessType::Cortex,
            None,
            process_event_tx,
        );
        let hook = CortexChatHook::new(event_tx, spacebot_hook);

        let action = tokio::time::timeout(
            Duration::from_millis(20),
            <CortexChatHook as PromptHook<SpacebotModel>>::on_text_delta(
                &hook,
                "partial ",
                "partial response",
            ),
        )
        .await
        .expect("text delta should not block on a full SSE channel");

        assert!(matches!(action, HookAction::Continue));
        let first_event = event_rx
            .recv()
            .await
            .expect("prefilled event should remain");
        assert!(matches!(first_event, CortexChatEvent::Thinking));
        assert!(
            event_rx.try_recv().is_err(),
            "full SSE channel should drop text deltas instead of backpressuring the prompt"
        );
    }

    #[test]
    fn missing_stream_final_response_is_an_error() {
        let error = missing_stream_final_response_error();
        assert!(matches!(
            error,
            rig::completion::PromptError::CompletionError(
                rig::completion::CompletionError::ProviderError(_)
            )
        ));
        assert!(
            error
                .to_string()
                .contains("stream ended without final response")
        );
    }
}
