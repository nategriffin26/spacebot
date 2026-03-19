//! Runtime capability truth and tool-health diagnosis.

pub mod capabilities;
pub mod doctor;

pub use capabilities::{
    CapabilityConfidence, CapabilityRecord, CapabilitySnapshot, CapabilityStatus,
    CapabilitySubject, ProcessCapabilitySummary, ToolingSummary, build_capability_snapshot,
    build_verified_capabilities,
};
pub use doctor::{DoctorDiagnosis, ToolDoctorReport, build_tool_doctor_report};
