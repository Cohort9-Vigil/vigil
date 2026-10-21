-- Migration: 0003_indexed_events.sql
-- Direct queryable table for raw indexed protocol events (Deposit, Borrow, Repay, LiquidationCall, PriceUpdated)

CREATE TABLE IF NOT EXISTS indexed_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    trace_id VARCHAR(64) NOT NULL,
    block_number BIGINT NOT NULL,
    block_hash VARCHAR(66) NOT NULL,
    tx_hash VARCHAR(66) NOT NULL,
    log_index BIGINT NOT NULL,
    event_type VARCHAR(64) NOT NULL,
    account VARCHAR(42),
    asset VARCHAR(42),
    amount VARCHAR(78),
    data JSONB NOT NULL,
    timestamp BIGINT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    CONSTRAINT uq_indexed_event_log UNIQUE (block_hash, log_index)
);

CREATE INDEX IF NOT EXISTS idx_indexed_events_type ON indexed_events(event_type);
CREATE INDEX IF NOT EXISTS idx_indexed_events_account ON indexed_events(account);
CREATE INDEX IF NOT EXISTS idx_indexed_events_block ON indexed_events(block_number);
CREATE INDEX IF NOT EXISTS idx_indexed_events_tx ON indexed_events(tx_hash);
