//! Swarm planning and receipt merge summaries built on request groups.

use crate::tooling::ReceiptReference;

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmShardSpec {
    pub key: String,
    pub task: String,
    #[serde(default)]
    pub suggested_skills: Vec<String>,
    #[serde(default)]
    pub topic_ids: Vec<String>,
    #[serde(default)]
    pub interactive: bool,
    #[serde(default = "default_required")]
    pub required: bool,
}

const fn default_required() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmPlan {
    pub summary: String,
    pub shards: Vec<SwarmShardSpec>,
    #[serde(default)]
    pub completion_policy: SwarmCompletionPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmCompletionPolicy {
    #[serde(default = "default_merge_policy")]
    pub merge_policy: String,
    #[serde(default)]
    pub require_all_required_shards: bool,
    #[serde(default = "default_prefer_verified_claims")]
    pub prefer_merge_ready_receipts: bool,
}

fn default_merge_policy() -> String {
    "all_lanes_then_synthesize".to_string()
}

const fn default_prefer_verified_claims() -> bool {
    true
}

impl Default for SwarmCompletionPolicy {
    fn default() -> Self {
        Self {
            merge_policy: default_merge_policy(),
            require_all_required_shards: false,
            prefer_merge_ready_receipts: default_prefer_verified_claims(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmShardResult {
    pub worker_id: String,
    #[serde(default)]
    pub shard_key: Option<String>,
    #[serde(default)]
    pub task: Option<String>,
    #[serde(default)]
    pub receipt: Option<ReceiptReference>,
    #[serde(default)]
    pub merge_ready: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmMergeSummary {
    pub ready_receipt_count: usize,
    pub blocked_receipt_count: usize,
    pub merged_summary: String,
    #[serde(default)]
    pub contradiction_notes: Vec<String>,
    #[serde(default)]
    pub sources_used: Vec<String>,
    #[serde(default)]
    pub artifacts_created: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmRunSummary {
    pub request_group_id: String,
    pub shard_count: usize,
    pub merge: SwarmMergeSummary,
    #[serde(default)]
    pub shards: Vec<SwarmShardResult>,
}

pub fn merge_receipt_references(receipts: &[ReceiptReference]) -> SwarmMergeSummary {
    let mut summaries = Vec::new();
    let mut contradiction_notes = Vec::new();
    let mut sources_used = BTreeSet::new();
    let mut artifacts_created = BTreeSet::new();
    let mut ready_receipt_count = 0usize;
    let mut blocked_receipt_count = 0usize;

    for receipt in receipts {
        if receipt.merge_ready {
            ready_receipt_count += 1;
        } else {
            blocked_receipt_count += 1;
        }

        summaries.push(format!("{}: {}", receipt.worker_id, receipt.summary));

        if receipt.wrong_source_detected {
            contradiction_notes.push(format!(
                "worker {} reported wrong_source_detected",
                receipt.worker_id
            ));
        }

        if !receipt.merge_ready {
            contradiction_notes.push(format!("worker {} is not merge-ready", receipt.worker_id));
        }

        for source in &receipt.sources_used {
            sources_used.insert(source.clone());
        }

        for artifact in &receipt.artifacts_created {
            artifacts_created.insert(artifact.clone());
        }
    }

    SwarmMergeSummary {
        ready_receipt_count,
        blocked_receipt_count,
        merged_summary: if summaries.is_empty() {
            "No shard receipts recorded yet.".to_string()
        } else {
            summaries.join("\n")
        },
        contradiction_notes,
        sources_used: sources_used.into_iter().collect(),
        artifacts_created: artifacts_created.into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::merge_receipt_references;
    use crate::WorkerTerminalReason;
    use crate::tooling::ReceiptReference;

    #[test]
    fn merge_receipt_references_collects_contradictions_and_sources() {
        let merged = merge_receipt_references(&[
            ReceiptReference {
                worker_id: "worker-a".to_string(),
                summary: "verified the calendar data".to_string(),
                terminal_reason: Some(WorkerTerminalReason::Success),
                retries_attempted: 0,
                merge_ready: true,
                wrong_source_detected: false,
                sources_used: vec!["calendar:mcp".to_string()],
                artifacts_created: vec!["/tmp/report-a.json".to_string()],
            },
            ReceiptReference {
                worker_id: "worker-b".to_string(),
                summary: "found a conflicting mailbox thread".to_string(),
                terminal_reason: Some(WorkerTerminalReason::Blocked),
                retries_attempted: 1,
                merge_ready: false,
                wrong_source_detected: true,
                sources_used: vec!["mail:imap".to_string()],
                artifacts_created: vec![],
            },
        ]);

        assert_eq!(merged.ready_receipt_count, 1);
        assert_eq!(merged.blocked_receipt_count, 1);
        assert!(merged.merged_summary.contains("worker-a"));
        assert!(
            merged
                .contradiction_notes
                .iter()
                .any(|note| note.contains("wrong_source_detected"))
        );
        assert_eq!(
            merged.sources_used,
            vec!["calendar:mcp".to_string(), "mail:imap".to_string()]
        );
    }
}
