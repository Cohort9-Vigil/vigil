//! Transactional outbox for liquidation signals (RS-04, RS-05).

use common::events::LiquidationSignal;
use sqlx::{PgConnection, PgPool, Row};

#[derive(Debug, Clone)]
pub struct OutboxRecord {
    pub id: String,
    pub dedupe_key: String,
    pub position_owner: String,
    pub collateral_asset: String,
    pub debt_asset: String,
    pub block_number: u64,
    pub block_hash: String,
    pub signal: LiquidationSignal,
}

/// Enqueues a liquidation signal in the transactional outbox table.
/// Safe against duplicate insertions via UNIQUE (dedupe_key).
pub async fn enqueue_signal(
    conn: &mut PgConnection,
    signal: &LiquidationSignal,
) -> Result<bool, sqlx::Error> {
    let payload = serde_json::to_value(signal).unwrap_or(serde_json::Value::Null);

    let rows_affected = sqlx::query(
        r#"
        INSERT INTO signals_outbox (
            dedupe_key, position_owner, collateral_asset, debt_asset, block_number, block_hash, payload, status, created_at
        ) VALUES ($1, LOWER($2), LOWER($3), LOWER($4), $5, $6, $7, 'PENDING', NOW())
        ON CONFLICT (dedupe_key) DO NOTHING
        "#,
    )
    .bind(&signal.dedupe_key)
    .bind(&signal.borrower.0)
    .bind(&signal.collateral_asset.0)
    .bind(&signal.debt_asset.0)
    .bind(signal.detected_at_block as i64)
    .bind(&signal.block_hash)
    .bind(payload)
    .execute(&mut *conn)
    .await?
    .rows_affected();

    Ok(rows_affected > 0)
}

/// Marks all pending signals for a position as SUPERSEDED when it becomes healthy again.
pub async fn supersede_pending_signals(
    conn: &mut PgConnection,
    borrower: &str,
    collateral_asset: &str,
) -> Result<u64, sqlx::Error> {
    let rows_affected = sqlx::query(
        r#"
        UPDATE signals_outbox
        SET status = 'SUPERSEDED'
        WHERE LOWER(position_owner) = LOWER($1)
          AND LOWER(collateral_asset) = LOWER($2)
          AND status = 'PENDING'
        "#,
    )
    .bind(borrower)
    .bind(collateral_asset)
    .execute(&mut *conn)
    .await?
    .rows_affected();

    Ok(rows_affected)
}

/// Cancels all pending signals for a specific block hash when that block is reorged out (RS-05).
pub async fn cancel_pending_signals_for_block(
    conn: &mut PgConnection,
    block_hash: &str,
) -> Result<u64, sqlx::Error> {
    let rows_affected = sqlx::query(
        r#"
        UPDATE signals_outbox
        SET status = 'CANCELLED'
        WHERE block_hash = $1
          AND status = 'PENDING'
        "#,
    )
    .bind(block_hash)
    .execute(&mut *conn)
    .await?
    .rows_affected();

    Ok(rows_affected)
}

/// Fetches up to `limit` pending outbox records to be published by the outbox worker.
pub async fn fetch_pending_signals(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<OutboxRecord>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT id::TEXT AS id, dedupe_key, position_owner, collateral_asset, debt_asset, block_number, block_hash, payload
        FROM signals_outbox
        WHERE status = 'PENDING'
        ORDER BY created_at ASC
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    let mut records = Vec::with_capacity(rows.len());
    for r in rows {
        let payload_val: serde_json::Value = r.try_get("payload")?;
        let signal: LiquidationSignal =
            serde_json::from_value(payload_val).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        let block_num: i64 = r.get("block_number");

        records.push(OutboxRecord {
            id: r.get("id"),
            dedupe_key: r.get("dedupe_key"),
            position_owner: r.get("position_owner"),
            collateral_asset: r.get("collateral_asset"),
            debt_asset: r.get("debt_asset"),
            block_number: block_num as u64,
            block_hash: r.get("block_hash"),
            signal,
        });
    }

    Ok(records)
}

/// Marks an outbox record as SENT.
pub async fn mark_signal_sent(pool: &PgPool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE signals_outbox
        SET status = 'SENT',
            sent_at = NOW()
        WHERE id = $1::UUID
        "#,
    )
    .bind(id)
    .execute(pool)
    .await?;

    Ok(())
}
