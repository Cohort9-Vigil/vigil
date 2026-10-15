//! Idempotent event ledger and deduplication management (RS-04, RS-05).

use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, Row};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppliedDelta {
    pub prev_collateral: u128,
    pub new_collateral: u128,
    pub prev_debt: u128,
    pub new_debt: u128,
}

#[derive(Debug, Clone, Copy)]
pub struct EventRecordParams<'a> {
    pub block_hash: &'a str,
    pub log_index: u64,
    pub block_number: u64,
    pub event_type: &'a str,
    pub account: Option<&'a str>,
    pub asset: Option<&'a str>,
}

/// Attempts to insert a new event into the `processed_events` ledger.
/// Returns `Ok(true)` if inserted (fresh event), or `Ok(false)` if already seen (idempotent duplicate).
pub async fn record_event_if_new(
    conn: &mut PgConnection,
    params: EventRecordParams<'_>,
    delta: &AppliedDelta,
) -> Result<bool, sqlx::Error> {
    let delta_json = serde_json::to_value(delta).unwrap_or(serde_json::Value::Null);

    let rows_affected = sqlx::query(
        r#"
        INSERT INTO processed_events (
            block_hash, log_index, block_number, event_type, account, asset, applied_delta, is_retracted, applied_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, FALSE, NOW())
        ON CONFLICT (block_hash, log_index) DO NOTHING
        "#,
    )
    .bind(params.block_hash)
    .bind(params.log_index as i64)
    .bind(params.block_number as i64)
    .bind(params.event_type)
    .bind(params.account)
    .bind(params.asset)
    .bind(delta_json)
    .execute(&mut *conn)
    .await?
    .rows_affected();

    Ok(rows_affected > 0)
}

/// Marks an event as retracted in the `processed_events` ledger and returns the original delta to reverse.
/// Returns None if already retracted or not found (idempotent retract).
pub async fn mark_retracted(
    conn: &mut PgConnection,
    block_hash: &str,
    log_index: u64,
) -> Result<Option<(Option<String>, Option<String>, AppliedDelta)>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        UPDATE processed_events
        SET is_retracted = TRUE
        WHERE block_hash = $1 AND log_index = $2 AND is_retracted = FALSE
        RETURNING account, asset, applied_delta
        "#,
    )
    .bind(block_hash)
    .bind(log_index as i64)
    .fetch_optional(&mut *conn)
    .await?;

    if let Some(r) = row {
        let account: Option<String> = r.try_get("account").ok();
        let asset: Option<String> = r.try_get("asset").ok();
        let delta_val: serde_json::Value = r.try_get("applied_delta")?;
        let delta: AppliedDelta =
            serde_json::from_value(delta_val).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        Ok(Some((account, asset, delta)))
    } else {
        Ok(None)
    }
}
