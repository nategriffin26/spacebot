//! Practical operator-facing diagnosis over the capability graph.

use crate::tooling::capabilities::{CapabilityRecord, CapabilitySnapshot, CapabilityStatus};

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct DoctorDiagnosis {
    pub class: String,
    pub summary: String,
    pub remediation: String,
    pub capability_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolDoctorReport {
    pub diagnoses: Vec<DoctorDiagnosis>,
    pub degraded_count: usize,
    pub unavailable_count: usize,
}

fn diagnosis_for_reason(record: &CapabilityRecord) -> Option<(String, String, String)> {
    let reason = record.reason_code.as_deref()?;
    let remediation = record
        .remediation
        .clone()
        .unwrap_or_else(|| "Inspect the runtime config and live process logs.".to_string());
    let class = match reason {
        "invalid_config" => "invalid_config",
        "binary_missing" => "binary_missing",
        "auth_missing" => "auth_missing",
        "browser_profile_issue" | "stale_profile_lock" | "active_profile_in_use" => {
            "browser_profile_issue"
        }
        "websocket_unreachable" | "launch_failed" | "executable_not_found" => "browser_launch",
        "mcp_startup_failure" | "configured_but_not_connected" => "mcp_startup_failure",
        "naming_mismatch" => "naming_mismatch",
        "prompt_runtime_truth_mismatch" => "prompt_runtime_truth_mismatch",
        "disabled_in_config" => "disabled_in_config",
        _ => "degraded_runtime",
    };
    Some((
        class.to_string(),
        format!("{} is {}", record.name, reason.replace('_', " ")),
        remediation,
    ))
}

pub fn build_tool_doctor_report(snapshot: &CapabilitySnapshot) -> ToolDoctorReport {
    let mut grouped = std::collections::BTreeMap::<String, DoctorDiagnosis>::new();

    for record in snapshot.capabilities.iter().filter(|record| {
        matches!(
            record.status,
            CapabilityStatus::Degraded | CapabilityStatus::Unavailable
        )
    }) {
        let Some((class, summary, remediation)) = diagnosis_for_reason(record) else {
            continue;
        };
        let entry = grouped.entry(class.clone()).or_insert(DoctorDiagnosis {
            class,
            summary,
            remediation,
            capability_ids: Vec::new(),
        });
        entry.capability_ids.push(record.id.clone());
    }

    ToolDoctorReport {
        diagnoses: grouped.into_values().collect(),
        degraded_count: snapshot.summary.degraded,
        unavailable_count: snapshot.summary.unavailable,
    }
}
