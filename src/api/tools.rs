//! Truthful callable-tool and durable-binary observability.

use super::state::ApiState;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use rig::tool::Tool as _;
use serde::{Deserialize, Serialize};
use std::io::ErrorKind;
use std::sync::Arc;

#[derive(Deserialize)]
pub(super) struct ToolsQuery {
    agent_id: String,
}

#[derive(Serialize)]
pub(super) struct AgentToolsResponse {
    snapshot: crate::tooling::CapabilitySnapshot,
    doctor: crate::tooling::ToolDoctorReport,
}

#[derive(Serialize)]
pub(super) struct ToolBinariesResponse {
    tools_bin: String,
    binaries: Vec<BinaryEntry>,
}

#[derive(Serialize)]
pub(super) struct BinaryEntry {
    name: String,
    size: u64,
    modified: Option<String>,
}

/// List callable tools and capability health for a specific agent.
pub(super) async fn list_tools(
    State(state): State<Arc<ApiState>>,
    Query(query): Query<ToolsQuery>,
) -> Result<Json<AgentToolsResponse>, StatusCode> {
    let runtime_configs = state.runtime_configs.load();
    let runtime_config = runtime_configs
        .get(&query.agent_id)
        .ok_or(StatusCode::NOT_FOUND)?;
    let managers = state.mcp_managers.load();
    let manager = managers.get(&query.agent_id);

    let connected_mcp_servers = if let Some(manager) = manager {
        manager.connected_server_names().await
    } else {
        Vec::new()
    };
    let connected_mcp_tool_names = if let Some(manager) = manager {
        manager
            .get_tools()
            .await
            .into_iter()
            .map(|tool| tool.name())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let snapshot = crate::tooling::build_capability_snapshot(
        &query.agent_id,
        runtime_config,
        &connected_mcp_servers,
        &connected_mcp_tool_names,
    );
    let doctor = crate::tooling::build_tool_doctor_report(&snapshot);

    Ok(Json(AgentToolsResponse { snapshot, doctor }))
}

/// List the contents of the durable `tools/bin` directory.
pub(super) async fn list_tool_binaries(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<ToolBinariesResponse>, StatusCode> {
    let instance_dir = state.instance_dir.load();
    let tools_bin = instance_dir.join("tools/bin");

    let mut binaries = Vec::new();

    let tools_bin_is_dir = match tokio::fs::metadata(&tools_bin).await {
        Ok(metadata) => metadata.is_dir(),
        Err(error) if error.kind() == ErrorKind::NotFound => false,
        Err(error) => {
            tracing::warn!(path = %tools_bin.display(), %error, "failed to stat tools/bin");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    if tools_bin_is_dir {
        let mut entries = tokio::fs::read_dir(&tools_bin).await.map_err(|error| {
            tracing::warn!(path = %tools_bin.display(), %error, "failed to read tools/bin");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        while let Some(entry) = entries.next_entry().await.map_err(|error| {
            tracing::warn!(path = %tools_bin.display(), %error, "failed during tools/bin iteration");
            StatusCode::INTERNAL_SERVER_ERROR
        })? {
            let entry_path = entry.path();
            let metadata = match tokio::fs::symlink_metadata(&entry_path).await {
                Ok(metadata) => metadata,
                Err(error) => {
                    tracing::debug!(
                        path = %entry_path.display(),
                        %error,
                        "skipping entry with unreadable metadata"
                    );
                    continue;
                }
            };

            if metadata.file_type().is_symlink() || !metadata.is_file() {
                continue;
            }

            let modified = metadata.modified().ok().map(|time| {
                let datetime: chrono::DateTime<chrono::Utc> = time.into();
                datetime.to_rfc3339()
            });

            binaries.push(BinaryEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                size: metadata.len(),
                modified,
            });
        }
    }

    binaries.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(Json(ToolBinariesResponse {
        tools_bin: tools_bin.display().to_string(),
        binaries,
    }))
}
