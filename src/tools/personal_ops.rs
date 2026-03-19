//! Typed personal-ops tool family over email, browser health, and MCP bundles.

use crate::config::{Config, EmailConfig, RuntimeConfig};
use crate::mcp::{McpManager, McpToolMetadata};
use crate::messaging::traits::HistoryMessage;
use crate::tools::email_search::{EmailSearchArgs, EmailSearchTool};

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

#[derive(Clone)]
pub struct PersonalOpsTool {
    runtime_config: Arc<RuntimeConfig>,
    mcp_manager: Arc<McpManager>,
}

impl PersonalOpsTool {
    pub fn new(runtime_config: Arc<RuntimeConfig>, mcp_manager: Arc<McpManager>) -> Self {
        Self {
            runtime_config,
            mcp_manager,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("personal_ops failed: {0}")]
pub struct PersonalOpsError(String);

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PersonalOpsArgs {
    pub action: String,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default)]
    pub unread_only: bool,
    #[serde(default)]
    pub since_days: Option<u32>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub message_ids: Vec<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub starts_at: Option<String>,
    #[serde(default)]
    pub ends_at: Option<String>,
    #[serde(default)]
    pub destination: Option<String>,
    #[serde(default)]
    pub server_hint: Option<String>,
    #[serde(default)]
    pub confirm_mutation: bool,
}

#[derive(Debug, Serialize)]
pub struct PersonalOpsOutput {
    pub success: bool,
    pub action: String,
    pub bundle: String,
    pub risk_class: String,
    pub requires_approval: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_server: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
}

impl Tool for PersonalOpsTool {
    const NAME: &'static str = "personal_ops";

    type Error = PersonalOpsError;
    type Args = PersonalOpsArgs;
    type Output = PersonalOpsOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: crate::prompts::text::get("tools/personal_ops").to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": [
                            "calendar_search",
                            "calendar_create",
                            "mail_search",
                            "mail_read_thread",
                            "drive_list",
                            "drive_export",
                            "drive_sync",
                            "bookmark_ingest",
                            "browser_auth_status"
                        ]
                    },
                    "query": { "type": "string" },
                    "from": { "type": "string" },
                    "subject": { "type": "string" },
                    "folders": { "type": "array", "items": { "type": "string" } },
                    "unread_only": { "type": "boolean", "default": false },
                    "since_days": { "type": "integer" },
                    "limit": { "type": "integer" },
                    "message_ids": { "type": "array", "items": { "type": "string" } },
                    "url": { "type": "string" },
                    "title": { "type": "string" },
                    "description": { "type": "string" },
                    "starts_at": { "type": "string" },
                    "ends_at": { "type": "string" },
                    "destination": { "type": "string" },
                    "server_hint": { "type": "string" },
                    "confirm_mutation": { "type": "boolean", "default": false }
                },
                "required": ["action"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let (bundle, risk_class, requires_approval) = classify_action(&args.action)?;
        if requires_approval && !args.confirm_mutation {
            return Ok(PersonalOpsOutput {
                success: false,
                action: args.action,
                bundle: bundle.to_string(),
                risk_class: risk_class.to_string(),
                requires_approval,
                message: "This personal-ops action mutates an external system. Re-run with confirm_mutation=true after approval.".to_string(),
                tool_server: None,
                tool_name: None,
                result: None,
            });
        }

        match args.action.as_str() {
            "mail_search" => self.mail_search(args, bundle, risk_class).await,
            "mail_read_thread" => self.mail_read_thread(args, bundle, risk_class).await,
            "browser_auth_status" => self.browser_auth_status(bundle, risk_class).await,
            "calendar_search" | "calendar_create" | "drive_list" | "drive_export"
            | "drive_sync" | "bookmark_ingest" => {
                self.mcp_action(args, bundle, risk_class, requires_approval)
                    .await
            }
            other => Err(PersonalOpsError(format!(
                "unsupported personal_ops action: {other}"
            ))),
        }
    }
}

impl PersonalOpsTool {
    async fn mail_search(
        &self,
        args: PersonalOpsArgs,
        bundle: &str,
        risk_class: &str,
    ) -> Result<PersonalOpsOutput, PersonalOpsError> {
        let tool = EmailSearchTool::new(self.runtime_config.clone());
        let result = tool
            .call(EmailSearchArgs {
                query: args.query,
                from: args.from,
                subject: args.subject,
                folders: args.folders,
                unread_only: args.unread_only,
                since_days: args.since_days,
                limit: args.limit,
            })
            .await
            .map_err(|error| PersonalOpsError(error.to_string()))?;

        Ok(PersonalOpsOutput {
            success: true,
            action: "mail_search".to_string(),
            bundle: bundle.to_string(),
            risk_class: risk_class.to_string(),
            requires_approval: false,
            message: format!("Found {} matching email result(s).", result.result_count),
            tool_server: Some("email".to_string()),
            tool_name: Some("search_mailbox".to_string()),
            result: Some(serde_json::to_value(result).unwrap_or(serde_json::Value::Null)),
        })
    }

    async fn mail_read_thread(
        &self,
        args: PersonalOpsArgs,
        bundle: &str,
        risk_class: &str,
    ) -> Result<PersonalOpsOutput, PersonalOpsError> {
        if args.message_ids.is_empty() {
            return Err(PersonalOpsError(
                "mail_read_thread requires at least one message_id".to_string(),
            ));
        }

        let instance_dir = self.runtime_config.instance_dir.clone();
        let message_ids = args.message_ids.clone();
        let limit = args.limit.unwrap_or(20).clamp(1, 100);
        let history = tokio::task::spawn_blocking(move || {
            let email_config = load_email_config(&instance_dir)?;
            crate::messaging::email::fetch_mailbox_history_by_message_ids(
                &email_config,
                message_ids,
                limit,
            )
            .map_err(|error| PersonalOpsError(error.to_string()))
        })
        .await
        .map_err(|error| PersonalOpsError(format!("mail history task failed: {error}")))??;

        Ok(PersonalOpsOutput {
            success: true,
            action: "mail_read_thread".to_string(),
            bundle: bundle.to_string(),
            risk_class: risk_class.to_string(),
            requires_approval: false,
            message: format!("Loaded {} email thread message(s).", history.len()),
            tool_server: Some("email".to_string()),
            tool_name: Some("fetch_mailbox_history_by_message_ids".to_string()),
            result: Some(
                serde_json::to_value(render_history(history)).unwrap_or(serde_json::Value::Null),
            ),
        })
    }

    async fn browser_auth_status(
        &self,
        bundle: &str,
        risk_class: &str,
    ) -> Result<PersonalOpsOutput, PersonalOpsError> {
        let browser = self.runtime_config.browser_config.load();
        let health = crate::tools::browser::load_browser_health(&self.runtime_config);
        let launch_mode = if browser.persist_session {
            "persistent_profile"
        } else {
            "ephemeral_profile"
        };

        Ok(PersonalOpsOutput {
            success: health.is_none() && browser.enabled,
            action: "browser_auth_status".to_string(),
            bundle: bundle.to_string(),
            risk_class: risk_class.to_string(),
            requires_approval: false,
            message: if !browser.enabled {
                "Browser automation is disabled in runtime config.".to_string()
            } else if let Some(health) = &health {
                format!("Browser auth lane is degraded: {}", health.detail)
            } else {
                "Browser auth lane looks healthy.".to_string()
            },
            tool_server: Some("browser".to_string()),
            tool_name: Some("browser_auth_status".to_string()),
            result: Some(serde_json::json!({
                "enabled": browser.enabled,
                "launch_mode": launch_mode,
                "persist_session": browser.persist_session,
                "health": health,
            })),
        })
    }

    async fn mcp_action(
        &self,
        args: PersonalOpsArgs,
        bundle: &str,
        risk_class: &str,
        requires_approval: bool,
    ) -> Result<PersonalOpsOutput, PersonalOpsError> {
        let tools = self.mcp_manager.list_connected_tools().await;
        let Some(tool) = resolve_mcp_tool(&tools, &args.action, args.server_hint.as_deref()) else {
            return Ok(PersonalOpsOutput {
                success: false,
                action: args.action,
                bundle: bundle.to_string(),
                risk_class: risk_class.to_string(),
                requires_approval,
                message: format!(
                    "No connected MCP tool matched {}. Check capability status or provide server_hint.",
                    bundle
                ),
                tool_server: None,
                tool_name: None,
                result: Some(serde_json::json!({
                    "discovered_tools": tools,
                })),
            });
        };

        let payload = mcp_payload_for_action(&args);
        let result = self
            .mcp_manager
            .call_tool_on_server(&tool.server_name, &tool.tool_name, payload)
            .await
            .map_err(|error| PersonalOpsError(error.to_string()))?;

        Ok(PersonalOpsOutput {
            success: !result.is_error.unwrap_or(false),
            action: args.action,
            bundle: bundle.to_string(),
            risk_class: risk_class.to_string(),
            requires_approval,
            message: format!(
                "Called {} on {} for {}.",
                tool.tool_name, tool.server_name, bundle
            ),
            tool_server: Some(tool.server_name),
            tool_name: Some(tool.tool_name),
            result: Some(serde_json::to_value(result).unwrap_or(serde_json::Value::Null)),
        })
    }
}

fn classify_action(action: &str) -> Result<(&'static str, &'static str, bool), PersonalOpsError> {
    let value = match action {
        "calendar_search" => ("calendar", "read_only_external", false),
        "calendar_create" => ("calendar", "privileged_or_external_mutation", true),
        "mail_search" => ("mail", "read_only_external", false),
        "mail_read_thread" => ("mail", "read_only_external", false),
        "drive_list" => ("drive", "read_only_external", false),
        "drive_export" => ("drive", "read_only_external", false),
        "drive_sync" => ("drive", "privileged_or_external_mutation", true),
        "bookmark_ingest" => ("bookmark_ingest", "privileged_or_external_mutation", true),
        "browser_auth_status" => ("browser_auth", "observe_only", false),
        other => {
            return Err(PersonalOpsError(format!(
                "unknown personal_ops action: {other}"
            )));
        }
    };

    Ok(value)
}

fn resolve_mcp_tool(
    tools: &[McpToolMetadata],
    action: &str,
    server_hint: Option<&str>,
) -> Option<McpToolMetadata> {
    let preferred_tools = tools
        .iter()
        .filter(|tool| {
            server_hint
                .is_none_or(|hint| tool.server_name.contains(hint) || tool.tool_name.contains(hint))
        })
        .filter(|tool| mcp_tool_matches_action(tool, action))
        .cloned()
        .collect::<Vec<_>>();

    preferred_tools.into_iter().next()
}

fn mcp_tool_matches_action(tool: &McpToolMetadata, action: &str) -> bool {
    let haystack = format!(
        "{} {} {}",
        tool.server_name.to_ascii_lowercase(),
        tool.tool_name.to_ascii_lowercase(),
        tool.description.to_ascii_lowercase()
    );

    match action {
        "calendar_search" => {
            haystack.contains("calendar")
                && any_keyword(&haystack, &["list", "search", "events", "upcoming", "find"])
        }
        "calendar_create" => {
            haystack.contains("calendar")
                && any_keyword(&haystack, &["create", "insert", "add", "schedule"])
        }
        "drive_list" => {
            any_keyword(&haystack, &["drive", "gdrive", "docs"])
                && any_keyword(&haystack, &["list", "search", "find", "query"])
        }
        "drive_export" => {
            any_keyword(&haystack, &["drive", "gdrive", "docs"])
                && any_keyword(&haystack, &["export", "download", "fetch", "get"])
        }
        "drive_sync" => {
            any_keyword(&haystack, &["drive", "gdrive", "docs"])
                && any_keyword(&haystack, &["sync", "upload", "create", "update", "write"])
        }
        "bookmark_ingest" => any_keyword(
            &haystack,
            &[
                "bookmark",
                "bookmarks",
                "save",
                "ingest",
                "raindrop",
                "pocket",
            ],
        ),
        _ => false,
    }
}

fn any_keyword(haystack: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|keyword| haystack.contains(keyword))
}

fn mcp_payload_for_action(args: &PersonalOpsArgs) -> serde_json::Value {
    let mut payload = serde_json::Map::new();

    insert_optional(&mut payload, "query", args.query.as_deref());
    insert_optional(&mut payload, "from", args.from.as_deref());
    insert_optional(&mut payload, "subject", args.subject.as_deref());
    insert_optional(&mut payload, "url", args.url.as_deref());
    insert_optional(&mut payload, "title", args.title.as_deref());
    insert_optional(&mut payload, "description", args.description.as_deref());
    insert_optional(&mut payload, "start", args.starts_at.as_deref());
    insert_optional(&mut payload, "end", args.ends_at.as_deref());
    insert_optional(&mut payload, "destination", args.destination.as_deref());

    if !args.message_ids.is_empty() {
        payload.insert(
            "message_ids".to_string(),
            serde_json::json!(args.message_ids),
        );
    }
    if !args.folders.is_empty() {
        payload.insert("folders".to_string(), serde_json::json!(args.folders));
    }
    if let Some(limit) = args.limit {
        payload.insert("limit".to_string(), serde_json::json!(limit));
    }
    if let Some(since_days) = args.since_days {
        payload.insert("since_days".to_string(), serde_json::json!(since_days));
    }
    if args.unread_only {
        payload.insert("unread_only".to_string(), serde_json::json!(true));
    }

    serde_json::Value::Object(payload)
}

fn insert_optional(
    payload: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: Option<&str>,
) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        payload.insert(
            key.to_string(),
            serde_json::Value::String(value.to_string()),
        );
    }
}

fn load_email_config(instance_dir: &Path) -> Result<EmailConfig, PersonalOpsError> {
    let config = Config::load_for_instance(instance_dir).map_err(|error| {
        PersonalOpsError(format!(
            "failed to resolve config for {}: {error}",
            instance_dir.display()
        ))
    })?;

    let email = config
        .messaging
        .email
        .ok_or_else(|| PersonalOpsError("email adapter is not configured".to_string()))?;

    if !email.enabled {
        return Err(PersonalOpsError(
            "email adapter is configured but disabled".to_string(),
        ));
    }

    Ok(email)
}

fn render_history(history: Vec<HistoryMessage>) -> Vec<serde_json::Value> {
    history
        .into_iter()
        .map(|message| {
            serde_json::json!({
                "author": message.author,
                "content": message.content,
                "is_bot": message.is_bot,
                "timestamp": message.timestamp.map(|value| value.to_rfc3339()),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{classify_action, mcp_tool_matches_action};
    use crate::mcp::McpToolMetadata;

    #[test]
    fn classify_action_marks_mutating_personal_ops() {
        let (_, risk_class, requires_approval) =
            classify_action("calendar_create").expect("action should classify");
        assert_eq!(risk_class, "privileged_or_external_mutation");
        assert!(requires_approval);
    }

    #[test]
    fn mcp_tool_match_prefers_calendar_search_keywords() {
        let tool = McpToolMetadata {
            server_name: "google_calendar".to_string(),
            tool_name: "list_events".to_string(),
            description: "List upcoming calendar events".to_string(),
        };

        assert!(mcp_tool_matches_action(&tool, "calendar_search"));
        assert!(!mcp_tool_matches_action(&tool, "drive_sync"));
    }
}
