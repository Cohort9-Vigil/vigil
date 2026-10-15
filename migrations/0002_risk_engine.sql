-- Migration 0002: Risk Engine Tables and Outbox Schema (RS-04, RS-05, RS-06)

-- 1. Deduplication and Reversible Event Ledger
CREATE TABLE IF NOT EXISTS processed_events (
    block_hash VARCHAR(66) NOT NULL,
    log_index BIGINT NOT NULL,
    block_number BIGINT NOT NULL,
    event_type VARCHAR(32) NOT NULL,
    account VARCHAR(42),
    asset VARCHAR(42),
    applied_delta JSONB NOT NULL,
    is_retracted BOOLEAN NOT NULL DEFAULT FALSE,
    applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (block_hash, log_index)
);

CREATE INDEX IF NOT EXISTS idx_processed_events_block_hash ON processed_events(block_hash);
CREATE INDEX IF NOT EXISTS idx_processed_events_account ON processed_events(account);

-- 2. Asset Risk Configurations
CREATE TABLE IF NOT EXISTS asset_configs (
    asset VARCHAR(42) PRIMARY KEY,
    symbol VARCHAR(16) NOT NULL,
    decimals SMALLINT NOT NULL,
    ltv_wad NUMERIC(78, 0) NOT NULL,
    liq_threshold_wad NUMERIC(78, 0) NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT TRUE
);

-- 3. Protocol State & Heartbeat Gating
CREATE TABLE IF NOT EXISTS protocol_state (
    id INT PRIMARY KEY DEFAULT 1,
    is_frozen BOOLEAN NOT NULL DEFAULT FALSE,
    last_indexed_block BIGINT NOT NULL DEFAULT 0,
    last_indexed_block_hash VARCHAR(66),
    last_heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    unresolved_retracts_count INT NOT NULL DEFAULT 0,
    CONSTRAINT single_row CHECK (id = 1)
);

INSERT INTO protocol_state (id, is_frozen, last_indexed_block, last_heartbeat_at)
VALUES (1, FALSE, 0, NOW())
ON CONFLICT (id) DO NOTHING;

-- 4. Transactional Outbox for Liquidation Signals
CREATE TABLE IF NOT EXISTS signals_outbox (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    dedupe_key VARCHAR(160) UNIQUE NOT NULL,
    position_owner VARCHAR(42) NOT NULL,
    collateral_asset VARCHAR(42) NOT NULL,
    debt_asset VARCHAR(42) NOT NULL,
    block_number BIGINT NOT NULL,
    block_hash VARCHAR(66) NOT NULL,
    payload JSONB NOT NULL,
    status VARCHAR(20) NOT NULL DEFAULT 'PENDING',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    sent_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_signals_outbox_pending ON signals_outbox(status) WHERE status = 'PENDING';
CREATE INDEX IF NOT EXISTS idx_signals_outbox_block_hash ON signals_outbox(block_hash);

-- 5. Extend positions with scaled debt and interest index if not present
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'positions' AND column_name = 'scaled_debt') THEN
        ALTER TABLE positions ADD COLUMN scaled_debt NUMERIC(78, 0) NOT NULL DEFAULT 0;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'positions' AND column_name = 'last_index_ray') THEN
        ALTER TABLE positions ADD COLUMN last_index_ray NUMERIC(78, 0) NOT NULL DEFAULT 1000000000000000000000000000;
    END IF;
END $$;
