ALTER TABLE cron_executions ADD COLUMN channel_id TEXT;
ALTER TABLE cron_executions ADD COLUMN terminal_reason TEXT;
ALTER TABLE cron_executions ADD COLUMN delivered BOOLEAN;
ALTER TABLE cron_executions ADD COLUMN result_count INTEGER;
ALTER TABLE cron_executions ADD COLUMN worker_ids_json TEXT;
ALTER TABLE cron_executions ADD COLUMN branch_ids_json TEXT;
ALTER TABLE cron_executions ADD COLUMN receipt_refs_json TEXT;

CREATE INDEX IF NOT EXISTS idx_cron_executions_cron_time
ON cron_executions(cron_id, executed_at DESC);
