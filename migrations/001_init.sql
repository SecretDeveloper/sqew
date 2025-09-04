-- 001_init.sql
-- Initial schema for Sqew message queue

CREATE TABLE queue (
  id            INTEGER PRIMARY KEY,
  name          TEXT UNIQUE NOT NULL,
  dlq_id        INTEGER REFERENCES queue(id) ON DELETE SET NULL,
  max_attempts  INTEGER NOT NULL DEFAULT 5,
  visibility_ms INTEGER NOT NULL DEFAULT 30000
);

CREATE TABLE message (
  id               INTEGER PRIMARY KEY,
  queue_id         INTEGER NOT NULL REFERENCES queue(id) ON DELETE CASCADE,
  payload_json     TEXT NOT NULL,
  priority         INTEGER NOT NULL DEFAULT 0,
  idempotency_key  TEXT,
  attempts         INTEGER NOT NULL DEFAULT 0,
  available_at     INTEGER NOT NULL,
  lease_expires_at INTEGER,
  leased_by        TEXT,
  created_at       INTEGER NOT NULL,
  expires_at       INTEGER,
  UNIQUE(queue_id, idempotency_key)
);

CREATE INDEX ix_msg_ready ON message(queue_id, lease_expires_at, available_at, priority DESC);
CREATE INDEX ix_msg_visible ON message(queue_id, available_at) WHERE lease_expires_at IS NULL;
CREATE INDEX ix_msg_leased ON message(queue_id, lease_expires_at) WHERE lease_expires_at IS NOT NULL;
