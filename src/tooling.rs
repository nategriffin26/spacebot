//! Runtime capability truth and tool-health diagnosis.

pub mod capabilities;
pub mod doctor;
pub mod receipts;
pub mod swarm;

pub use capabilities::{
    CapabilityConfidence, CapabilityRecord, CapabilitySnapshot, CapabilityStatus,
    CapabilitySubject, ProcessCapabilitySummary, ToolingSummary, build_capability_snapshot,
    build_verified_capabilities,
};
pub use doctor::{DoctorDiagnosis, ToolDoctorReport, build_tool_doctor_report};
pub use receipts::{ReceiptReference, summarize_worker_receipt};
pub use swarm::{
    SwarmCompletionPolicy, SwarmMergeSummary, SwarmPlan, SwarmRunSummary, SwarmShardResult,
    SwarmShardSpec, merge_receipt_references,
};
