//! Typed runtime capability snapshots shared by prompts, APIs, and self-awareness.

use crate::config::RuntimeConfig;
use crate::self_awareness::VerifiedCapability;
use crate::{ProcessType, TaskClass};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySubject {
    Tool,
    Skill,
    McpServer,
    Bundle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    Available,
    Degraded,
    Unavailable,
}

impl CapabilityStatus {
    pub const fn as_health_state(self) -> &'static str {
        match self {
            Self::Available => "verified",
            Self::Degraded => "degraded",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityConfidence {
    RuntimeVerifiedNow,
    ConfiguredButNotConnected,
    PromptVisibleOnly,
    DerivedBundle,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityRecord {
    pub id: String,
    pub name: String,
    pub subject: CapabilitySubject,
    pub status: CapabilityStatus,
    pub process_types: Vec<ProcessType>,
    pub invocation_path: String,
    pub confidence: CapabilityConfidence,
    pub reason_code: Option<String>,
    pub detail: Option<String>,
    pub remediation: Option<String>,
    pub source: Option<String>,
    pub last_verified_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessCapabilitySummary {
    pub process_type: ProcessType,
    pub available_tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolingSummary {
    pub available: usize,
    pub degraded: usize,
    pub unavailable: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilitySnapshot {
    pub agent_id: String,
    pub generated_at: String,
    pub capabilities: Vec<CapabilityRecord>,
    pub process_summaries: Vec<ProcessCapabilitySummary>,
    pub summary: ToolingSummary,
}

fn channel_tool_names() -> &'static [&'static str] {
    &[
        "reply",
        "branch_and_spawn",
        "branch",
        "spawn_worker",
        "route",
        "cancel",
        "skip",
        "react",
        "send_file",
        "project_manage",
    ]
}

fn branch_tool_names() -> &'static [&'static str] {
    &[
        "memory_save",
        "memory_recall",
        "memory_delete",
        "channel_recall",
        "spacebot_docs",
        "email_search",
        "worker_inspect",
        "task_create",
        "task_list",
        "task_update",
    ]
}

fn cortex_tool_names() -> &'static [&'static str] {
    &["memory_save"]
}

fn worker_tool_names(
    browser_enabled: bool,
    brave_search_enabled: bool,
    secrets_enabled: bool,
    connected_mcp_tool_names: &[String],
) -> Vec<String> {
    let mut names = vec![
        "shell".to_string(),
        "file_read".to_string(),
        "file_write".to_string(),
        "file_edit".to_string(),
        "file_list".to_string(),
        "task_update".to_string(),
        "set_status".to_string(),
        "record_receipt".to_string(),
        "read_skill".to_string(),
    ];

    if secrets_enabled {
        names.push("secret_set".to_string());
    }
    if browser_enabled {
        names.extend(
            [
                "browser_launch",
                "browser_navigate",
                "browser_snapshot",
                "browser_click",
                "browser_type",
                "browser_press_key",
                "browser_screenshot",
                "browser_evaluate",
                "browser_tab_open",
                "browser_tab_list",
                "browser_tab_close",
                "browser_close",
            ]
            .into_iter()
            .map(str::to_string),
        );
    }
    if brave_search_enabled {
        names.push("web_search".to_string());
    }
    names.extend(connected_mcp_tool_names.iter().cloned());
    names
}

fn push_tool_capabilities(
    capabilities: &mut Vec<CapabilityRecord>,
    names: &[String],
    process_type: ProcessType,
    now: &str,
) {
    for name in names {
        capabilities.push(CapabilityRecord {
            id: format!("tool:{}:{name}", process_type),
            name: name.clone(),
            subject: CapabilitySubject::Tool,
            status: CapabilityStatus::Available,
            process_types: vec![process_type],
            invocation_path: name.clone(),
            confidence: CapabilityConfidence::RuntimeVerifiedNow,
            reason_code: None,
            detail: None,
            remediation: None,
            source: None,
            last_verified_at: Some(now.to_string()),
        });
    }
}

fn push_static_tools(
    capabilities: &mut Vec<CapabilityRecord>,
    names: &[&str],
    process_type: ProcessType,
    now: &str,
) {
    let names = names
        .iter()
        .map(|value| (*value).to_string())
        .collect::<Vec<_>>();
    push_tool_capabilities(capabilities, &names, process_type, now);
}

fn opencode_path_status(path: &str) -> (CapabilityStatus, Option<String>, Option<String>) {
    let resolved = crate::config::resolve_env_value(path).unwrap_or_else(|| path.to_string());
    if resolved.trim().is_empty() {
        return (
            CapabilityStatus::Unavailable,
            Some("invalid_config".to_string()),
            Some("Set [opencode].path to a binary path or env:VAR reference.".to_string()),
        );
    }

    let exists_on_path = if resolved.contains('/') {
        std::path::Path::new(&resolved).exists()
    } else {
        std::process::Command::new("which")
            .arg(&resolved)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    };

    if exists_on_path {
        (CapabilityStatus::Available, None, None)
    } else {
        (
            CapabilityStatus::Degraded,
            Some("binary_missing".to_string()),
            Some(format!(
                "Install or expose the OpenCode binary at '{resolved}'."
            )),
        )
    }
}

fn classify_mcp_failure(message: &str) -> (&'static str, String) {
    let lower = message.to_lowercase();
    if lower.contains("authorization") || lower.contains("api key") || lower.contains("token") {
        (
            "auth_missing",
            "Provide the required MCP auth secret or authorization header.".to_string(),
        )
    } else if lower.contains("no such file")
        || lower.contains("not found")
        || lower.contains("enoent")
    {
        (
            "binary_missing",
            "Install the MCP server binary or correct the configured command path.".to_string(),
        )
    } else if lower.contains("transport")
        || lower.contains("unexpectedcontenttype")
        || lower.contains("streamable")
        || lower.contains("content-type")
    {
        (
            "mcp_startup_failure",
            "Verify the MCP transport type, URL, and headers match the server.".to_string(),
        )
    } else if lower.contains("header")
        || lower.contains("invalid")
        || lower.contains("parse")
        || lower.contains("url")
    {
        (
            "invalid_config",
            "Fix the MCP config values so the runtime can parse them cleanly.".to_string(),
        )
    } else {
        (
            "mcp_startup_failure",
            "Inspect the server stderr and reconnect after correcting the underlying failure."
                .to_string(),
        )
    }
}

fn build_process_summaries(
    worker_tools: &[String],
    snapshot_tools: &[CapabilityRecord],
) -> Vec<ProcessCapabilitySummary> {
    let collect_names = |process_type: ProcessType| {
        let mut names = snapshot_tools
            .iter()
            .filter(|record| {
                record.subject == CapabilitySubject::Tool
                    && record.status == CapabilityStatus::Available
                    && record.process_types.contains(&process_type)
            })
            .map(|record| record.name.clone())
            .collect::<Vec<_>>();
        names.sort();
        names
    };

    vec![
        ProcessCapabilitySummary {
            process_type: ProcessType::Channel,
            available_tools: collect_names(ProcessType::Channel),
        },
        ProcessCapabilitySummary {
            process_type: ProcessType::Branch,
            available_tools: collect_names(ProcessType::Branch),
        },
        ProcessCapabilitySummary {
            process_type: ProcessType::Worker,
            available_tools: {
                let mut names = worker_tools.to_vec();
                names.sort();
                names
            },
        },
        ProcessCapabilitySummary {
            process_type: ProcessType::Cortex,
            available_tools: collect_names(ProcessType::Cortex),
        },
    ]
}

pub fn build_capability_snapshot(
    agent_id: &str,
    runtime_config: &RuntimeConfig,
    connected_mcp_servers: &[String],
    connected_mcp_tool_names: &[String],
) -> CapabilitySnapshot {
    let now = chrono::Utc::now().to_rfc3339();
    let mut capabilities = Vec::new();

    push_static_tools(
        &mut capabilities,
        channel_tool_names(),
        ProcessType::Channel,
        &now,
    );
    push_static_tools(
        &mut capabilities,
        branch_tool_names(),
        ProcessType::Branch,
        &now,
    );
    push_static_tools(
        &mut capabilities,
        cortex_tool_names(),
        ProcessType::Cortex,
        &now,
    );

    let browser = runtime_config.browser_config.load();
    let worker_tools = worker_tool_names(
        browser.enabled,
        runtime_config.brave_search_key.load().is_some(),
        runtime_config.secrets.load().as_ref().is_some(),
        connected_mcp_tool_names,
    );
    push_tool_capabilities(&mut capabilities, &worker_tools, ProcessType::Worker, &now);

    if !browser.enabled {
        capabilities.push(CapabilityRecord {
            id: "bundle:browser".to_string(),
            name: "browser".to_string(),
            subject: CapabilitySubject::Bundle,
            status: CapabilityStatus::Unavailable,
            process_types: vec![ProcessType::Worker],
            invocation_path: "browser_*".to_string(),
            confidence: CapabilityConfidence::ConfiguredButNotConnected,
            reason_code: Some("disabled_in_config".to_string()),
            detail: Some("browser tools are disabled in runtime config".to_string()),
            remediation: Some("Enable [browser].enabled to expose browser tools.".to_string()),
            source: None,
            last_verified_at: None,
        });
    }

    if runtime_config.brave_search_key.load().is_none() {
        capabilities.push(CapabilityRecord {
            id: "bundle:web_search".to_string(),
            name: "web_search".to_string(),
            subject: CapabilitySubject::Bundle,
            status: CapabilityStatus::Unavailable,
            process_types: vec![ProcessType::Worker],
            invocation_path: "web_search".to_string(),
            confidence: CapabilityConfidence::ConfiguredButNotConnected,
            reason_code: Some("auth_missing".to_string()),
            detail: Some("Brave Search key is not configured".to_string()),
            remediation: Some(
                "Configure `BRAVE_SEARCH_API_KEY` (or the matching secret) to enable web search."
                    .to_string(),
            ),
            source: None,
            last_verified_at: None,
        });
    }

    let opencode = runtime_config.opencode.load();
    let (opencode_status, opencode_reason, opencode_remediation) = if opencode.enabled {
        opencode_path_status(&opencode.path)
    } else {
        (
            CapabilityStatus::Unavailable,
            Some("disabled_in_config".to_string()),
            Some("Enable [opencode].enabled to allow OpenCode workers.".to_string()),
        )
    };
    capabilities.push(CapabilityRecord {
        id: "bundle:opencode".to_string(),
        name: "opencode".to_string(),
        subject: CapabilitySubject::Bundle,
        status: opencode_status,
        process_types: vec![
            ProcessType::Channel,
            ProcessType::Branch,
            ProcessType::Worker,
        ],
        invocation_path: "spawn_worker(worker_type=opencode)".to_string(),
        confidence: if opencode.enabled {
            CapabilityConfidence::RuntimeVerifiedNow
        } else {
            CapabilityConfidence::ConfiguredButNotConnected
        },
        reason_code: opencode_reason,
        detail: Some(format!("path={}", opencode.path)),
        remediation: opencode_remediation,
        source: None,
        last_verified_at: (opencode.enabled && opencode_status == CapabilityStatus::Available)
            .then(|| now.clone()),
    });

    for skill in runtime_config.skills.load().list() {
        let source = match skill.source {
            crate::skills::SkillSource::Instance => "instance",
            crate::skills::SkillSource::Workspace => "workspace",
        };
        capabilities.push(CapabilityRecord {
            id: format!("skill:{}", skill.name.to_lowercase()),
            name: skill.name,
            subject: CapabilitySubject::Skill,
            status: CapabilityStatus::Available,
            process_types: vec![ProcessType::Worker],
            invocation_path: format!("read_skill(name={})", skill.file_path.display()),
            confidence: CapabilityConfidence::RuntimeVerifiedNow,
            reason_code: None,
            detail: Some(skill.description),
            remediation: None,
            source: Some(source.to_string()),
            last_verified_at: Some(now.clone()),
        });
    }

    for server in runtime_config
        .mcp
        .load()
        .iter()
        .filter(|server| server.enabled)
    {
        let connected = connected_mcp_servers
            .iter()
            .any(|name| name == &server.name);
        let status = if connected {
            CapabilityStatus::Available
        } else {
            CapabilityStatus::Degraded
        };
        let (reason_code, remediation, detail) = if connected {
            (
                None,
                None,
                Some(format!("transport={}", server.transport.kind())),
            )
        } else {
            let previous =
                crate::self_awareness::load_verified_capabilities(agent_id, runtime_config)
                    .unwrap_or_default()
                    .into_iter()
                    .find(|entry| entry.capability_name == server.name)
                    .and_then(|entry| entry.failure_reason);
            let (reason_code, remediation) = previous
                .as_deref()
                .map(classify_mcp_failure)
                .map(|(code, remediation)| (Some(code.to_string()), Some(remediation)))
                .unwrap_or((
                    Some("configured_but_not_connected".to_string()),
                    Some("Reconnect the MCP server and inspect its runtime logs.".to_string()),
                ));
            (
                reason_code,
                remediation,
                Some(format!("configured transport={}", server.transport.kind())),
            )
        };

        capabilities.push(CapabilityRecord {
            id: format!("mcp:{}", server.name),
            name: server.name.clone(),
            subject: CapabilitySubject::McpServer,
            status,
            process_types: vec![ProcessType::Worker],
            invocation_path: format!("mcp:{}", server.name),
            confidence: if connected {
                CapabilityConfidence::RuntimeVerifiedNow
            } else {
                CapabilityConfidence::ConfiguredButNotConnected
            },
            reason_code,
            detail,
            remediation,
            source: None,
            last_verified_at: connected.then(|| now.clone()),
        });
    }

    for tool_name in connected_mcp_tool_names {
        capabilities.push(CapabilityRecord {
            id: format!("tool:mcp:{tool_name}"),
            name: tool_name.clone(),
            subject: CapabilitySubject::Tool,
            status: CapabilityStatus::Available,
            process_types: vec![ProcessType::Worker],
            invocation_path: tool_name.clone(),
            confidence: CapabilityConfidence::RuntimeVerifiedNow,
            reason_code: None,
            detail: Some("MCP-backed tool on a connected server".to_string()),
            remediation: None,
            source: Some("mcp".to_string()),
            last_verified_at: Some(now.clone()),
        });
    }

    let summary = ToolingSummary {
        available: capabilities
            .iter()
            .filter(|record| record.status == CapabilityStatus::Available)
            .count(),
        degraded: capabilities
            .iter()
            .filter(|record| record.status == CapabilityStatus::Degraded)
            .count(),
        unavailable: capabilities
            .iter()
            .filter(|record| record.status == CapabilityStatus::Unavailable)
            .count(),
    };

    let process_summaries = build_process_summaries(&worker_tools, &capabilities);

    CapabilitySnapshot {
        agent_id: agent_id.to_string(),
        generated_at: now,
        capabilities,
        process_summaries,
        summary,
    }
}

pub fn build_verified_capabilities(
    agent_id: &str,
    runtime_config: &RuntimeConfig,
    connected_mcp_servers: &[String],
) -> Vec<VerifiedCapability> {
    let snapshot = build_capability_snapshot(agent_id, runtime_config, connected_mcp_servers, &[]);
    snapshot
        .capabilities
        .into_iter()
        .map(|record| VerifiedCapability {
            capability_name: record.name,
            category: match record.subject {
                CapabilitySubject::Tool => "tool",
                CapabilitySubject::Skill => "skill",
                CapabilitySubject::McpServer => "mcp",
                CapabilitySubject::Bundle => "bundle",
            }
            .to_string(),
            invocation_path: record.invocation_path,
            required_secrets_present: !matches!(
                record.reason_code.as_deref(),
                Some("auth_missing" | "invalid_config")
            ),
            health_state: record.status.as_health_state().to_string(),
            last_verified_at: record.last_verified_at,
            last_failure_at: (record.status != CapabilityStatus::Available)
                .then_some(snapshot.generated_at.clone()),
            failure_reason: record.reason_code,
            note: record.detail,
        })
        .collect()
}

impl TaskClass {
    pub const fn requires_receipt_contract(self) -> bool {
        matches!(
            self,
            Self::SourceVerification
                | Self::ArtifactGroundedCopy
                | Self::CapabilityDiagnosis
                | Self::MultiLaneSynthesis
                | Self::SimpleBoundedExecution
                | Self::Coding
        )
    }
}
