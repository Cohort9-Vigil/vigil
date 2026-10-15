-- Migration: Indexer Cursor and Reorg History Tracking (RS-03)
-- Tracks the last committed block cursor and history for crash recovery and ancestor verification.

CREATE TABLE IF NOT EXISTS indexer_cursor (
    id VARCHAR(32) PRIMARY KEY DEFAULT 'default',
    last_processed_block BIGINT NOT NULL,
    last_processed_hash VARCHAR(66) NOT NULL,
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS cursor_history (
    block_number BIGINT NOT NULL,
    block_hash VARCHAR(66) NOT NULL,
    parent_hash VARCHAR(66) NOT NULL,
    status VARCHAR(32) NOT NULL DEFAULT 'CANONICAL', -- 'CANONICAL' or 'ORPHANED'
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (block_number, block_hash)
);

CREATE INDEX IF NOT EXISTS idx_cursor_history_status ON cursor_history(status);
CREATE INDEX IF NOT EXISTS idx_cursor_history_number ON cursor_history(block_number);
