//! Oracle price store and asset risk configuration queries.

use common::types::Address;
use common::types::AssetConfig;
use sqlx::{PgConnection, Row};

/// Upserts an oracle price observation.
pub async fn upsert_oracle_price(
    conn: &mut PgConnection,
    asset: &str,
    price_wad: u128,
    block_number: u64,
    timestamp: u64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO oracle_prices (asset, price_wad, block_number, timestamp, created_at)
        VALUES (LOWER($1), $2::NUMERIC, $3, $4, NOW())
        ON CONFLICT (asset, block_number) DO UPDATE SET
            price_wad = $2::NUMERIC,
            timestamp = $4
        "#,
    )
    .bind(asset)
    .bind(price_wad.to_string())
    .bind(block_number as i64)
    .bind(timestamp as i64)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Retrieves the latest price observation for an asset.
pub async fn get_latest_oracle_price(
    conn: &mut PgConnection,
    asset: &str,
) -> Result<Option<u128>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT price_wad::TEXT AS price_wad_str
        FROM oracle_prices
        WHERE LOWER(asset) = LOWER($1)
        ORDER BY block_number DESC
        LIMIT 1
        "#,
    )
    .bind(asset)
    .fetch_optional(&mut *conn)
    .await?;

    if let Some(r) = row {
        let price_str: String = r.get("price_wad_str");
        Ok(price_str.parse().ok())
    } else {
        Ok(None)
    }
}

/// Upserts an asset risk configuration (decimals, LTV, liquidation threshold).
pub async fn upsert_asset_config(
    conn: &mut PgConnection,
    config: &AssetConfig,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO asset_configs (asset, symbol, decimals, ltv_wad, liq_threshold_wad, is_active)
        VALUES (LOWER($1), $2, $3, $4::NUMERIC, $5::NUMERIC, $6)
        ON CONFLICT (asset) DO UPDATE SET
            symbol = $2,
            decimals = $3,
            ltv_wad = $4::NUMERIC,
            liq_threshold_wad = $5::NUMERIC,
            is_active = $6
        "#,
    )
    .bind(&config.asset.0)
    .bind(&config.symbol)
    .bind(config.decimals as i16)
    .bind(config.ltv_wad.to_string())
    .bind(config.liq_threshold_wad.to_string())
    .bind(config.is_active)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Retrieves the risk configuration for an asset.
pub async fn get_asset_config(
    conn: &mut PgConnection,
    asset: &str,
) -> Result<Option<AssetConfig>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT asset, symbol, decimals, ltv_wad::TEXT AS ltv_str, liq_threshold_wad::TEXT AS liq_str, is_active
        FROM asset_configs
        WHERE LOWER(asset) = LOWER($1)
        "#,
    )
    .bind(asset)
    .fetch_optional(&mut *conn)
    .await?;

    if let Some(r) = row {
        let ltv_str: String = r.get("ltv_str");
        let liq_str: String = r.get("liq_str");
        let decimals: i16 = r.get("decimals");

        Ok(Some(AssetConfig {
            asset: Address::new(r.get::<String, _>("asset")),
            symbol: r.get("symbol"),
            decimals: decimals as u8,
            ltv_wad: ltv_str.parse().unwrap_or(0),
            liq_threshold_wad: liq_str.parse().unwrap_or(0),
            is_active: r.get("is_active"),
        }))
    } else {
        Ok(None)
    }
}
