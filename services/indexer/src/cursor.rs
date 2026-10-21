//! Crash-resilient cursor persistence in PostgreSQL (RS-03).
//!
//! Enforces:
//! 1. Commit cursor to Postgres ONLY AFTER NATS JetStream publish ack succeeds.
//! 2. Startup recovery loads saved cursor and recent block window.
//! 3. Restart resumes from cursor + 1 with zero gaps.

use std::sync::{Arc, Mutex};
use anyhow::{anyhow, Result};
use common::events::IndexedEvent;
use sqlx::{PgPool, Row};
use tracing::{debug, info};

/// Record of an indexed block in the cursor history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockCursorRecord {
    pub block_number: u64,
    pub block_hash: String,
    pub parent_hash: String,
}

/// Abstract contract for cursor persistence, allowing seamless swapping between
/// PostgreSQL in production and thread-safe mock storage in unit/chaos tests.
#[allow(async_fn_in_trait)]
pub trait CursorStore: Send + Sync {
    /// Loads the latest committed cursor `(block_number, block_hash)` on startup.
    async fn get_last_cursor(&self) -> Result<Option<(u64, String)>>;

    /// Commits the newly processed block to the database AFTER JetStream ack.
    async fn commit_block(
        &self,
        block_number: u64,
        block_hash: &str,
        parent_hash: &str,
        timestamp: u64,
    ) -> Result<()>;

    /// Saves newly indexed chain events into PostgreSQL for direct queryability.
    async fn save_events(&self, events: &[IndexedEvent]) -> Result<()>;

    /// Marks a range of blocks as ORPHANED during a chain reorganization.
    async fn mark_reorged(&self, from_block: u64, to_block: u64) -> Result<()>;

    /// Loads recent canonical blocks (ordered oldest to newest) to warm the reorg ring buffer on startup.
    async fn load_recent_window(&self, limit: usize) -> Result<Vec<BlockCursorRecord>>;
}

/// PostgreSQL-backed crash-safe cursor store.
#[derive(Clone)]
pub struct PostgresCursorStore {
    pool: PgPool,
}

impl PostgresCursorStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = PgPool::connect(database_url).await
            .map_err(|e| anyhow!("Failed to connect to PostgreSQL at {database_url}: {e}"))?;
        let store = Self::new(pool);
        store.init_schema().await?;
        Ok(store)
    }

    /// Idempotently ensures the required schema tables exist.
    pub async fn init_schema(&self) -> Result<()> {
        let statements = [
            r#"
            CREATE TABLE IF NOT EXISTS blocks (
                block_number BIGINT PRIMARY KEY,
                block_hash VARCHAR(66) NOT NULL,
                parent_hash VARCHAR(66) NOT NULL,
                timestamp BIGINT NOT NULL,
                is_reorged BOOLEAN DEFAULT FALSE,
                created_at TIMESTAMPTZ DEFAULT NOW()
            )
            "#,
            r#"
            CREATE TABLE IF NOT EXISTS indexer_cursor (
                id VARCHAR(32) PRIMARY KEY DEFAULT 'default',
                last_processed_block BIGINT NOT NULL,
                last_processed_hash VARCHAR(66) NOT NULL,
                updated_at TIMESTAMPTZ DEFAULT NOW()
            )
            "#,
            r#"
            CREATE TABLE IF NOT EXISTS cursor_history (
                block_number BIGINT NOT NULL,
                block_hash VARCHAR(66) NOT NULL,
                parent_hash VARCHAR(66) NOT NULL,
                status VARCHAR(32) NOT NULL DEFAULT 'CANONICAL',
                updated_at TIMESTAMPTZ DEFAULT NOW(),
                PRIMARY KEY (block_number, block_hash)
            )
            "#,
            r#"
            CREATE INDEX IF NOT EXISTS idx_cursor_history_status ON cursor_history(status)
            "#,
            r#"
            CREATE TABLE IF NOT EXISTS indexed_events (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                trace_id VARCHAR(64) NOT NULL,
                block_number BIGINT NOT NULL,
                block_hash VARCHAR(66) NOT NULL,
                tx_hash VARCHAR(66) NOT NULL,
                log_index BIGINT NOT NULL,
                event_type VARCHAR(32) NOT NULL,
                account VARCHAR(42),
                asset VARCHAR(42),
                amount VARCHAR(78),
                data JSONB NOT NULL,
                timestamp BIGINT NOT NULL,
                created_at TIMESTAMPTZ DEFAULT NOW(),
                CONSTRAINT uq_indexed_event_log UNIQUE (block_hash, log_index)
            )
            "#,
            r#"
            CREATE INDEX IF NOT EXISTS idx_indexed_events_type ON indexed_events(event_type)
            "#,
            r#"
            CREATE INDEX IF NOT EXISTS idx_indexed_events_account ON indexed_events(account)
            "#,
            r#"
            CREATE INDEX IF NOT EXISTS idx_indexed_events_block ON indexed_events(block_number)
            "#,
        ];

        for stmt in statements {
            sqlx::query(stmt)
                .execute(&self.pool)
                .await
                .map_err(|e| anyhow!("Failed to initialize cursor schema: {e}"))?;
        }

        info!("Postgres cursor schema verified");
        Ok(())
    }
}

impl CursorStore for PostgresCursorStore {
    async fn get_last_cursor(&self) -> Result<Option<(u64, String)>> {
        let row = sqlx::query(
            "SELECT last_processed_block, last_processed_hash FROM indexer_cursor WHERE id = 'default'"
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| anyhow!("Failed to load indexer_cursor: {e}"))?;

        if let Some(r) = row {
            let block_num: i64 = r.try_get("last_processed_block")
                .map_err(|e| anyhow!("Failed to read last_processed_block: {e}"))?;
            let block_hash: String = r.try_get("last_processed_hash")
                .map_err(|e| anyhow!("Failed to read last_processed_hash: {e}"))?;
            return Ok(Some((block_num as u64, block_hash)));
        }

        // Fallback: check blocks table if cursor table has no entry
        let fallback_row = sqlx::query(
            "SELECT block_number, block_hash FROM blocks WHERE is_reorged = FALSE ORDER BY block_number DESC LIMIT 1"
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| anyhow!("Failed to check fallback blocks table: {e}"))?;

        if let Some(r) = fallback_row {
            let block_num: i64 = r.try_get("block_number")
                .map_err(|e| anyhow!("Failed to read block_number: {e}"))?;
            let block_hash: String = r.try_get("block_hash")
                .map_err(|e| anyhow!("Failed to read block_hash: {e}"))?;
            return Ok(Some((block_num as u64, block_hash)));
        }

        Ok(None)
    }

    async fn commit_block(
        &self,
        block_number: u64,
        block_hash: &str,
        parent_hash: &str,
        timestamp: u64,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await
            .map_err(|e| anyhow!("Failed to begin Postgres transaction: {e}"))?;

        // 1. Upsert indexer_cursor
        sqlx::query(
            r#"
            INSERT INTO indexer_cursor (id, last_processed_block, last_processed_hash, updated_at)
            VALUES ('default', $1, $2, NOW())
            ON CONFLICT (id) DO UPDATE SET
                last_processed_block = EXCLUDED.last_processed_block,
                last_processed_hash = EXCLUDED.last_processed_hash,
                updated_at = NOW()
            "#
        )
        .bind(block_number as i64)
        .bind(block_hash)
        .execute(&mut *tx)
        .await
        .map_err(|e| anyhow!("Failed to update indexer_cursor: {e}"))?;

        // 2. Upsert blocks table
        sqlx::query(
            r#"
            INSERT INTO blocks (block_number, block_hash, parent_hash, timestamp, is_reorged, created_at)
            VALUES ($1, $2, $3, $4, FALSE, NOW())
            ON CONFLICT (block_number) DO UPDATE SET
                block_hash = EXCLUDED.block_hash,
                parent_hash = EXCLUDED.parent_hash,
                timestamp = EXCLUDED.timestamp,
                is_reorged = FALSE
            "#
        )
        .bind(block_number as i64)
        .bind(block_hash)
        .bind(parent_hash)
        .bind(timestamp as i64)
        .execute(&mut *tx)
        .await
        .map_err(|e| anyhow!("Failed to upsert blocks table: {e}"))?;

        // 3. Insert or update cursor_history
        sqlx::query(
            r#"
            INSERT INTO cursor_history (block_number, block_hash, parent_hash, status, updated_at)
            VALUES ($1, $2, $3, 'CANONICAL', NOW())
            ON CONFLICT (block_number, block_hash) DO UPDATE SET
                status = 'CANONICAL',
                updated_at = NOW()
            "#
        )
        .bind(block_number as i64)
        .bind(block_hash)
        .bind(parent_hash)
        .execute(&mut *tx)
        .await
        .map_err(|e| anyhow!("Failed to record cursor_history: {e}"))?;

        tx.commit().await
            .map_err(|e| anyhow!("Failed to commit cursor transaction: {e}"))?;

        debug!(block_number, %block_hash, "Committed block cursor to PostgreSQL");
        Ok(())
    }

    async fn save_events(&self, events: &[IndexedEvent]) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }

        for evt in events {
            let event_type = evt.event.event_type();
            let account = evt.event.account_str();
            let asset = evt.event.asset_str();
            let amount = evt.event.amount_str();
            let data_json = serde_json::to_value(&evt.event).unwrap_or_default();

            sqlx::query(
                r#"
                INSERT INTO indexed_events (trace_id, block_number, block_hash, tx_hash, log_index, event_type, account, asset, amount, data, timestamp)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                ON CONFLICT (block_hash, log_index) DO NOTHING
                "#
            )
            .bind(&evt.trace_id)
            .bind(evt.block_number as i64)
            .bind(&evt.block_hash)
            .bind(&evt.tx_hash)
            .bind(evt.log_index as i64)
            .bind(event_type)
            .bind(account)
            .bind(asset)
            .bind(amount)
            .bind(data_json)
            .bind(evt.timestamp as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| anyhow!("Failed to insert indexed event {}: {e}", evt.trace_id))?;
        }

        debug!(count = events.len(), "Persisted indexed events to PostgreSQL");
        Ok(())
    }

    async fn mark_reorged(&self, from_block: u64, to_block: u64) -> Result<()> {
        let mut tx = self.pool.begin().await
            .map_err(|e| anyhow!("Failed to begin transaction for reorg mark: {e}"))?;

        sqlx::query(
            "UPDATE blocks SET is_reorged = TRUE WHERE block_number >= $1 AND block_number <= $2"
        )
        .bind(from_block as i64)
        .bind(to_block as i64)
        .execute(&mut *tx)
        .await
        .map_err(|e| anyhow!("Failed to mark blocks as reorged: {e}"))?;

        sqlx::query(
            "UPDATE cursor_history SET status = 'ORPHANED', updated_at = NOW() WHERE block_number >= $1 AND block_number <= $2"
        )
        .bind(from_block as i64)
        .bind(to_block as i64)
        .execute(&mut *tx)
        .await
        .map_err(|e| anyhow!("Failed to mark cursor_history as orphaned: {e}"))?;

        tx.commit().await
            .map_err(|e| anyhow!("Failed to commit reorg mark transaction: {e}"))?;

        info!(from_block, to_block, "Marked reorged blocks in PostgreSQL");
        Ok(())
    }

    async fn load_recent_window(&self, limit: usize) -> Result<Vec<BlockCursorRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT block_number, block_hash, parent_hash
            FROM blocks
            WHERE is_reorged = FALSE
            ORDER BY block_number DESC
            LIMIT $1
            "#
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| anyhow!("Failed to fetch recent block window: {e}"))?;

        let mut records = Vec::with_capacity(rows.len());
        for row in rows {
            let num: i64 = row.try_get("block_number")
                .map_err(|e| anyhow!("Failed to read block_number: {e}"))?;
            let hash: String = row.try_get("block_hash")
                .map_err(|e| anyhow!("Failed to read block_hash: {e}"))?;
            let parent: String = row.try_get("parent_hash")
                .map_err(|e| anyhow!("Failed to read parent_hash: {e}"))?;
            records.push(BlockCursorRecord {
                block_number: num as u64,
                block_hash: hash,
                parent_hash: parent,
            });
        }

        // Reverse so that list is in ascending order (oldest to newest)
        records.reverse();
        Ok(records)
    }
}

/// In-memory mock cursor store for testing.
#[derive(Debug, Default, Clone)]
pub struct MockCursorStore {
    pub last_cursor: Arc<Mutex<Option<(u64, String)>>>,
    pub records: Arc<Mutex<Vec<BlockCursorRecord>>>,
    pub reorged: Arc<Mutex<Vec<(u64, u64)>>>,
}

impl MockCursorStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_initial_cursor(block_number: u64, block_hash: &str) -> Self {
        Self {
            last_cursor: Arc::new(Mutex::new(Some((block_number, block_hash.to_string())))),
            records: Arc::new(Mutex::new(vec![BlockCursorRecord {
                block_number,
                block_hash: block_hash.to_string(),
                parent_hash: "0x0".to_string(),
            }])),
            reorged: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn get_committed_blocks(&self) -> Vec<BlockCursorRecord> {
        self.records.lock().unwrap().clone()
    }

    pub fn get_reorged_ranges(&self) -> Vec<(u64, u64)> {
        self.reorged.lock().unwrap().clone()
    }
}

impl CursorStore for MockCursorStore {
    async fn get_last_cursor(&self) -> Result<Option<(u64, String)>> {
        Ok(self.last_cursor.lock().unwrap().clone())
    }

    async fn commit_block(
        &self,
        block_number: u64,
        block_hash: &str,
        parent_hash: &str,
        _timestamp: u64,
    ) -> Result<()> {
        let mut cur = self.last_cursor.lock().unwrap();
        *cur = Some((block_number, block_hash.to_string()));

        let mut recs = self.records.lock().unwrap();
        recs.retain(|r| r.block_number != block_number);
        recs.push(BlockCursorRecord {
            block_number,
            block_hash: block_hash.to_string(),
            parent_hash: parent_hash.to_string(),
        });
        recs.sort_by_key(|r| r.block_number);

        Ok(())
    }

    async fn save_events(&self, _events: &[IndexedEvent]) -> Result<()> {
        Ok(())
    }

    async fn mark_reorged(&self, from_block: u64, to_block: u64) -> Result<()> {
        self.reorged.lock().unwrap().push((from_block, to_block));
        let mut recs = self.records.lock().unwrap();
        recs.retain(|r| r.block_number < from_block || r.block_number > to_block);
        let mut cur = self.last_cursor.lock().unwrap();
        if let Some((b, _)) = *cur {
            if b >= from_block && b <= to_block {
                *cur = recs.last().map(|r| (r.block_number, r.block_hash.clone()));
            }
        }
        Ok(())
    }

    async fn load_recent_window(&self, limit: usize) -> Result<Vec<BlockCursorRecord>> {
        let recs = self.records.lock().unwrap();
        let len = recs.len();
        let start = len.saturating_sub(limit);
        Ok(recs[start..].to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_cursor_lifecycle() {
        let store = MockCursorStore::new();
        assert_eq!(store.get_last_cursor().await.unwrap(), None);

        store.commit_block(1, "0xhash1", "0x0", 1000).await.unwrap();
        assert_eq!(
            store.get_last_cursor().await.unwrap(),
            Some((1, "0xhash1".into()))
        );

        store.commit_block(2, "0xhash2", "0xhash1", 1001).await.unwrap();
        store.commit_block(3, "0xhash3", "0xhash2", 1002).await.unwrap();

        assert_eq!(
            store.get_last_cursor().await.unwrap(),
            Some((3, "0xhash3".into()))
        );

        let window = store.load_recent_window(2).await.unwrap();
        assert_eq!(window.len(), 2);
        assert_eq!(window[0].block_number, 2);
        assert_eq!(window[1].block_number, 3);

        // Simulate reorg of block 3
        store.mark_reorged(3, 3).await.unwrap();
        assert_eq!(
            store.get_last_cursor().await.unwrap(),
            Some((2, "0xhash2".into()))
        );
    }
}
