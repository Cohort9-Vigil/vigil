//! Contract event definitions and decoding using alloy sol! macro.

use alloy::primitives::U256;
use alloy::rpc::types::eth::Log;
use alloy::sol;
use alloy::sol_types::SolEvent;
use anyhow::{anyhow, Result};
use common::events::ChainEvent;
use common::types::{Address, OraclePrice};

sol! {
    #[derive(Debug, PartialEq, Eq)]
    event Deposit(address indexed user, address indexed asset, uint256 amount);

    #[derive(Debug, PartialEq, Eq)]
    event Borrow(address indexed user, uint256 amount);

    #[derive(Debug, PartialEq, Eq)]
    event Repay(address indexed user, uint256 amount);

    #[derive(Debug, PartialEq, Eq)]
    event LiquidationCall(
        address indexed collateralAsset,
        address indexed user,
        uint256 debtToCover,
        uint256 liquidatedCollateralAmount,
        address liquidator
    );

    #[derive(Debug, PartialEq, Eq)]
    event PriceUpdated(address indexed asset, uint256 price, uint256 timestamp);
}

/// Helper to decode an EVM log into a strongly-typed `ChainEvent`.
/// Returns `None` if the log is not one of the monitored Vigil protocol events.
pub fn decode_log(log: &Log, block_number: u64, tx_hash: &str) -> Result<Option<ChainEvent>> {
    let topics = log.topics();
    if topics.is_empty() {
        return Ok(None);
    }

    let topic0 = topics[0];
    let data = log.data().data.as_ref();

    if topic0 == Deposit::SIGNATURE_HASH {
        let decoded = Deposit::decode_raw_log(topics, data)?;
        return Ok(Some(ChainEvent::Deposit {
            account: Address::new(format!("{:?}", decoded.user)),
            asset: Address::new(format!("{:?}", decoded.asset)),
            amount: u256_to_u128(decoded.amount)?,
            block_number,
            tx_hash: tx_hash.to_string(),
        }));
    }

    if topic0 == Borrow::SIGNATURE_HASH {
        let decoded = Borrow::decode_raw_log(topics, data)?;
        return Ok(Some(ChainEvent::Borrow {
            account: Address::new(format!("{:?}", decoded.user)),
            amount: u256_to_u128(decoded.amount)?,
            block_number,
            tx_hash: tx_hash.to_string(),
        }));
    }

    if topic0 == Repay::SIGNATURE_HASH {
        let decoded = Repay::decode_raw_log(topics, data)?;
        return Ok(Some(ChainEvent::Repay {
            account: Address::new(format!("{:?}", decoded.user)),
            amount: u256_to_u128(decoded.amount)?,
            block_number,
            tx_hash: tx_hash.to_string(),
        }));
    }

    if topic0 == LiquidationCall::SIGNATURE_HASH {
        let decoded = LiquidationCall::decode_raw_log(topics, data)?;
        return Ok(Some(ChainEvent::LiquidationCall {
            collateral_asset: Address::new(format!("{:?}", decoded.collateralAsset)),
            user: Address::new(format!("{:?}", decoded.user)),
            debt_to_cover: u256_to_u128(decoded.debtToCover)?,
            liquidated_collateral_amount: u256_to_u128(decoded.liquidatedCollateralAmount)?,
            liquidator: Address::new(format!("{:?}", decoded.liquidator)),
            block_number,
            tx_hash: tx_hash.to_string(),
        }));
    }

    if topic0 == PriceUpdated::SIGNATURE_HASH {
        let decoded = PriceUpdated::decode_raw_log(topics, data)?;
        return Ok(Some(ChainEvent::PriceUpdated(OraclePrice {
            asset: Address::new(format!("{:?}", decoded.asset)),
            price_wad: u256_to_u128(decoded.price)?,
            block_number,
            timestamp: u256_to_u64(decoded.timestamp)?,
        })));
    }

    Ok(None)
}

fn u256_to_u128(val: U256) -> Result<u128> {
    u128::try_from(val).map_err(|e| anyhow!("U256 overflow when converting to u128: {e}"))
}

fn u256_to_u64(val: U256) -> Result<u64> {
    u64::try_from(val).map_err(|e| anyhow!("U256 overflow when converting to u64: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{address, uint};

    #[test]
    fn test_deposit_encode_decode() {
        let user = address!("0000000000000000000000000000000000000001");
        let asset = address!("0000000000000000000000000000000000000002");
        let amount = uint!(1_000_000_000_000_000_000_U256);

        let deposit_event = Deposit {
            user,
            asset,
            amount,
        };
        let raw_log = deposit_event.encode_log_data();

        let mut log = Log::default();
        log.inner.data = raw_log;

        let decoded = decode_log(&log, 42, "0xtxhash")
            .expect("decode log")
            .expect("some event");
        match decoded {
            ChainEvent::Deposit {
                account,
                asset: ast,
                amount: amt,
                block_number,
                tx_hash,
            } => {
                assert_eq!(
                    account.0.to_lowercase(),
                    format!("{:?}", user).to_lowercase()
                );
                assert_eq!(ast.0.to_lowercase(), format!("{:?}", asset).to_lowercase());
                assert_eq!(amt, 1_000_000_000_000_000_000);
                assert_eq!(block_number, 42);
                assert_eq!(tx_hash, "0xtxhash");
            }
            _ => panic!("Expected Deposit event"),
        }
    }

    #[test]
    fn test_borrow_encode_decode() {
        let user = address!("0000000000000000000000000000000000000003");
        let amount = uint!(500_000_U256);

        let borrow_event = Borrow { user, amount };
        let raw_log = borrow_event.encode_log_data();

        let mut log = Log::default();
        log.inner.data = raw_log;

        let decoded = decode_log(&log, 43, "0xtxhash2")
            .expect("decode")
            .expect("event");
        match decoded {
            ChainEvent::Borrow {
                account,
                amount: amt,
                block_number,
                tx_hash,
            } => {
                assert_eq!(
                    account.0.to_lowercase(),
                    format!("{:?}", user).to_lowercase()
                );
                assert_eq!(amt, 500_000);
                assert_eq!(block_number, 43);
                assert_eq!(tx_hash, "0xtxhash2");
            }
            _ => panic!("Expected Borrow event"),
        }
    }

    #[test]
    fn test_price_updated_encode_decode() {
        let asset = address!("0000000000000000000000000000000000000004");
        let price = uint!(2_500_000_000_000_000_000_U256);
        let timestamp = uint!(1_700_000_000_U256);

        let price_event = PriceUpdated {
            asset,
            price,
            timestamp,
        };
        let raw_log = price_event.encode_log_data();

        let mut log = Log::default();
        log.inner.data = raw_log;

        let decoded = decode_log(&log, 44, "0xtxhash3")
            .expect("decode")
            .expect("event");
        match decoded {
            ChainEvent::PriceUpdated(oracle_price) => {
                assert_eq!(
                    oracle_price.asset.0.to_lowercase(),
                    format!("{:?}", asset).to_lowercase()
                );
                assert_eq!(oracle_price.price_wad, 2_500_000_000_000_000_000);
                assert_eq!(oracle_price.timestamp, 1_700_000_000);
                assert_eq!(oracle_price.block_number, 44);
            }
            _ => panic!("Expected PriceUpdated event"),
        }
    }
}
