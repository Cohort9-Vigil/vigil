//! Position state store and mutations with reversible delta tracking (RS-04, RS-05).

use crate::db::events::AppliedDelta;
use sqlx::{PgConnection, Row};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionRecord {
    pub account: String,
    pub collateral_asset: String,
    pub collateral_amount: u128,
    pub debt_amount: u128,
    pub health_factor: u128,
    pub last_updated_block: u64,
}

/// Retrieves a borrower's position for a specific collateral asset.
pub async fn get_position(
    conn: &mut PgConnection,
    account: &str,
    collateral_asset: &str,
) -> Result<Option<PositionRecord>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT
            account,
            collateral_asset,
            collateral_amount::TEXT AS collateral_amount_str,
            debt_amount::TEXT AS debt_amount_str,
            health_factor::TEXT AS health_factor_str,
            last_updated_block
        FROM positions
        WHERE LOWER(account) = LOWER($1) AND LOWER(collateral_asset) = LOWER($2)
        "#,
    )
    .bind(account)
    .bind(collateral_asset)
    .fetch_optional(&mut *conn)
    .await?;

    if let Some(r) = row {
        let col_str: String = r.get("collateral_amount_str");
        let debt_str: String = r.get("debt_amount_str");
        let hf_str: String = r.get("health_factor_str");
        let last_block: i64 = r.get("last_updated_block");

        Ok(Some(PositionRecord {
            account: r.get("account"),
            collateral_asset: r.get("collateral_asset"),
            collateral_amount: col_str.parse().unwrap_or(0),
            debt_amount: debt_str.parse().unwrap_or(0),
            health_factor: hf_str.parse().unwrap_or(0),
            last_updated_block: last_block as u64,
        }))
    } else {
        Ok(None)
    }
}

/// Applies a Deposit event and records the reversible delta.
pub async fn apply_deposit(
    conn: &mut PgConnection,
    account: &str,
    asset: &str,
    amount: u128,
    block_number: u64,
) -> Result<AppliedDelta, sqlx::Error> {
    let current = get_position(conn, account, asset).await?;
    let prev_collateral = current.as_ref().map(|p| p.collateral_amount).unwrap_or(0);
    let prev_debt = current.as_ref().map(|p| p.debt_amount).unwrap_or(0);
    let new_collateral = prev_collateral.saturating_add(amount);

    sqlx::query(
        r#"
        INSERT INTO positions (
            account, collateral_asset, collateral_amount, debt_amount, health_factor, last_updated_block, updated_at
        ) VALUES (LOWER($1), LOWER($2), $3::NUMERIC, $4::NUMERIC, 0, $5, NOW())
        ON CONFLICT (account, collateral_asset) DO UPDATE SET
            collateral_amount = $3::NUMERIC,
            last_updated_block = $5,
            updated_at = NOW()
        "#,
    )
    .bind(account)
    .bind(asset)
    .bind(new_collateral.to_string())
    .bind(prev_debt.to_string())
    .bind(block_number as i64)
    .execute(&mut *conn)
    .await?;

    Ok(AppliedDelta {
        prev_collateral,
        new_collateral,
        prev_debt,
        new_debt: prev_debt,
    })
}

/// Applies a Borrow event and records the reversible delta.
pub async fn apply_borrow(
    conn: &mut PgConnection,
    account: &str,
    collateral_asset: &str,
    amount: u128,
    block_number: u64,
) -> Result<AppliedDelta, sqlx::Error> {
    let current = get_position(conn, account, collateral_asset).await?;
    let prev_collateral = current.as_ref().map(|p| p.collateral_amount).unwrap_or(0);
    let prev_debt = current.as_ref().map(|p| p.debt_amount).unwrap_or(0);
    let new_debt = prev_debt.saturating_add(amount);

    sqlx::query(
        r#"
        INSERT INTO positions (
            account, collateral_asset, collateral_amount, debt_amount, health_factor, last_updated_block, updated_at
        ) VALUES (LOWER($1), LOWER($2), $3::NUMERIC, $4::NUMERIC, 0, $5, NOW())
        ON CONFLICT (account, collateral_asset) DO UPDATE SET
            debt_amount = $4::NUMERIC,
            last_updated_block = $5,
            updated_at = NOW()
        "#,
    )
    .bind(account)
    .bind(collateral_asset)
    .bind(prev_collateral.to_string())
    .bind(new_debt.to_string())
    .bind(block_number as i64)
    .execute(&mut *conn)
    .await?;

    Ok(AppliedDelta {
        prev_collateral,
        new_collateral: prev_collateral,
        prev_debt,
        new_debt,
    })
}

/// Applies a Repay event and records the reversible delta.
pub async fn apply_repay(
    conn: &mut PgConnection,
    account: &str,
    collateral_asset: &str,
    amount: u128,
    block_number: u64,
) -> Result<AppliedDelta, sqlx::Error> {
    let current = get_position(conn, account, collateral_asset).await?;
    let prev_collateral = current.as_ref().map(|p| p.collateral_amount).unwrap_or(0);
    let prev_debt = current.as_ref().map(|p| p.debt_amount).unwrap_or(0);
    let new_debt = prev_debt.saturating_sub(amount);

    sqlx::query(
        r#"
        UPDATE positions
        SET debt_amount = $1::NUMERIC,
            last_updated_block = $2,
            updated_at = NOW()
        WHERE LOWER(account) = LOWER($3) AND LOWER(collateral_asset) = LOWER($4)
        "#,
    )
    .bind(new_debt.to_string())
    .bind(block_number as i64)
    .bind(account)
    .bind(collateral_asset)
    .execute(&mut *conn)
    .await?;

    Ok(AppliedDelta {
        prev_collateral,
        new_collateral: prev_collateral,
        prev_debt,
        new_debt,
    })
}

/// Applies a Liquidation event (debt covered and collateral seized).
pub async fn apply_liquidation(
    conn: &mut PgConnection,
    account: &str,
    collateral_asset: &str,
    debt_covered: u128,
    collateral_seized: u128,
    block_number: u64,
) -> Result<AppliedDelta, sqlx::Error> {
    let current = get_position(conn, account, collateral_asset).await?;
    let prev_collateral = current.as_ref().map(|p| p.collateral_amount).unwrap_or(0);
    let prev_debt = current.as_ref().map(|p| p.debt_amount).unwrap_or(0);

    let new_collateral = prev_collateral.saturating_sub(collateral_seized);
    let new_debt = prev_debt.saturating_sub(debt_covered);

    sqlx::query(
        r#"
        UPDATE positions
        SET collateral_amount = $1::NUMERIC,
            debt_amount = $2::NUMERIC,
            last_updated_block = $3,
            updated_at = NOW()
        WHERE LOWER(account) = LOWER($4) AND LOWER(collateral_asset) = LOWER($5)
        "#,
    )
    .bind(new_collateral.to_string())
    .bind(new_debt.to_string())
    .bind(block_number as i64)
    .bind(account)
    .bind(collateral_asset)
    .execute(&mut *conn)
    .await?;

    Ok(AppliedDelta {
        prev_collateral,
        new_collateral,
        prev_debt,
        new_debt,
    })
}

/// Reverts an applied delta when a block is reorged out (RS-05).
pub async fn revert_delta(
    conn: &mut PgConnection,
    account: &str,
    collateral_asset: &str,
    delta: &AppliedDelta,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE positions
        SET collateral_amount = $1::NUMERIC,
            debt_amount = $2::NUMERIC,
            updated_at = NOW()
        WHERE LOWER(account) = LOWER($3) AND LOWER(collateral_asset) = LOWER($4)
        "#,
    )
    .bind(delta.prev_collateral.to_string())
    .bind(delta.prev_debt.to_string())
    .bind(account)
    .bind(collateral_asset)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Updates the persisted health factor for a position.
pub async fn update_health_factor(
    conn: &mut PgConnection,
    account: &str,
    collateral_asset: &str,
    health_factor: u128,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE positions
        SET health_factor = $1::NUMERIC,
            updated_at = NOW()
        WHERE LOWER(account) = LOWER($2) AND LOWER(collateral_asset) = LOWER($3)
        "#,
    )
    .bind(health_factor.to_string())
    .bind(account)
    .bind(collateral_asset)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Finds all positions holding a specific collateral asset (used for batch re-evaluation on price updates).
pub async fn get_positions_by_collateral(
    conn: &mut PgConnection,
    collateral_asset: &str,
) -> Result<Vec<PositionRecord>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT
            account,
            collateral_asset,
            collateral_amount::TEXT AS collateral_amount_str,
            debt_amount::TEXT AS debt_amount_str,
            health_factor::TEXT AS health_factor_str,
            last_updated_block
        FROM positions
        WHERE LOWER(collateral_asset) = LOWER($1)
        "#,
    )
    .bind(collateral_asset)
    .fetch_all(&mut *conn)
    .await?;

    let mut records = Vec::with_capacity(rows.len());
    for r in rows {
        let col_str: String = r.get("collateral_amount_str");
        let debt_str: String = r.get("debt_amount_str");
        let hf_str: String = r.get("health_factor_str");
        let last_block: i64 = r.get("last_updated_block");

        records.push(PositionRecord {
            account: r.get("account"),
            collateral_asset: r.get("collateral_asset"),
            collateral_amount: col_str.parse().unwrap_or(0),
            debt_amount: debt_str.parse().unwrap_or(0),
            health_factor: hf_str.parse().unwrap_or(0),
            last_updated_block: last_block as u64,
        });
    }

    Ok(records)
}
