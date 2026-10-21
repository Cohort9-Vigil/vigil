//! Protocol state and indexer heartbeat tracking.

use sqlx::{PgConnection, PgPool, Row};

#[derive(Debug, Clone)]
pub struct ProtocolStateRecord {
    pub is_frozen: bool,
    pub last_indexed_block: u64,
    pub last_indexed_block_hash: Option<String>,
    pub unresolved_retracts_count: i32,
}

/// Updates the indexer heartbeat and latest block number in protocol_state.
pub async fn update_heartbeat(
    conn: &mut PgConnection,
    block_number: u64,
    block_hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE protocol_state
        SET last_indexed_block = GREATEST(last_indexed_block, $1),
            last_indexed_block_hash = $2,
            last_heartbeat_at = NOW()
        WHERE id = 1
        "#,
    )
    .bind(block_number as i64)
    .bind(block_hash)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Sets the protocol freeze flag.
pub async fn set_protocol_frozen(conn: &mut PgConnection, frozen: bool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE protocol_state
        SET is_frozen = $1,
            last_heartbeat_at = NOW()
        WHERE id = 1
        "#,
    )
    .bind(frozen)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Fetches the current protocol state.
pub async fn get_protocol_state(pool: &PgPool) -> Result<Option<ProtocolStateRecord>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT is_frozen, last_indexed_block, last_indexed_block_hash, unresolved_retracts_count
        FROM protocol_state
        WHERE id = 1
        "#,
    )
    .fetch_optional(pool)
    .await?;

    if let Some(r) = row {
        let block_num: i64 = r.get("last_indexed_block");
        Ok(Some(ProtocolStateRecord {
            is_frozen: r.get("is_frozen"),
            last_indexed_block: block_num as u64,
            last_indexed_block_hash: r.get("last_indexed_block_hash"),
            unresolved_retracts_count: r.get("unresolved_retracts_count"),
        }))
    } else {
        Ok(None)
    }
}
