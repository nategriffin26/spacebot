-- Request-group orchestration, worker receipts, and operator policy storage.

CREATE TABLE IF NOT EXISTS request_groups (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    origin_message_id TEXT,
    origin_user_text TEXT NOT NULL,
    mode TEXT NOT NULL DEFAULT 'single_reply',
    state TEXT NOT NULL DEFAULT 'planning',
    task_class TEXT,
    source_manifest_json TEXT,
    required_receipts_json TEXT,
    completion_policy_json TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_request_groups_channel ON request_groups(channel_id, created_at);
CREATE INDEX IF NOT EXISTS idx_request_groups_agent ON request_groups(agent_id, created_at);

ALTER TABLE worker_runs ADD COLUMN request_group_id TEXT;
ALTER TABLE worker_runs ADD COLUMN task_class TEXT;
ALTER TABLE worker_runs ADD COLUMN source_manifest_json TEXT;
ALTER TABLE worker_runs ADD COLUMN outcome_receipt_json TEXT;

ALTER TABLE branch_runs ADD COLUMN request_group_id TEXT;

CREATE INDEX IF NOT EXISTS idx_worker_runs_request_group ON worker_runs(request_group_id);
CREATE INDEX IF NOT EXISTS idx_branch_runs_request_group ON branch_runs(request_group_id);

CREATE TABLE IF NOT EXISTS operator_policies (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    policy_key TEXT NOT NULL,
    content TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 100,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_reaffirmed_at TIMESTAMP,
    source_channel_id TEXT,
    UNIQUE(agent_id, policy_key)
);

CREATE INDEX IF NOT EXISTS idx_operator_policies_agent_priority
    ON operator_policies(agent_id, priority DESC, updated_at DESC);
