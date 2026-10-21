//! Contract event definitions and decoding using alloy sol! macro.
//! Real events are generated from the official contract ABIs (VigilPool and LiquidationSettlement).

use alloy::sol;
use alloy::sol_types::SolEvent;
use alloy::primitives::U256;
use alloy::rpc::types::eth::Log;
use anyhow::{anyhow, Result};
use common::events::ChainEvent;
use common::types::{Address, OraclePrice};

// Real contract ABIs generated bindings
sol!(
    #[sol(rpc)]
    VigilPoolContract,
    "abi/VigilPool.json"
);

sol!(
    #[sol(rpc)]
    LiquidationSettlementContract,
    "abi/LiquidationSettlement.json"
);

// Re-export contract events generated from the ABIs
pub use VigilPoolContract::{
    BadDebtWrittenOff, Borrowed, CollateralDeposited, CollateralEnabledChanged,
    CollateralListed, CollateralWithdrawn, InterestAccrued, Liquidated, Repaid,
    Supplied, SupplyWithdrawn,
};

pub use LiquidationSettlementContract::{
    AuctionReset, AuctionStarted, BadDebtSettled, LiquidationExecuted, NonceUsed,
};

// Legacy & Mock events for backward compatibility and mock feed pricing
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

    #[derive(Debug, PartialEq, Eq)]
    event AnswerSet(int128 answer, uint32 roundId);
}

/// Helper to decode an EVM log into a strongly-typed `ChainEvent`.
/// Decodes all official VigilPool and LiquidationSettlement protocol events.
pub fn decode_log(
    log: &Log,
    block_number: u64,
    tx_hash: &str,
) -> Result<Option<ChainEvent>> {
    let topics = log.topics();
    if topics.is_empty() {
        return Ok(None);
    }

    let topic0 = topics[0];
    let data = log.data().data.as_ref();

    // ==========================================
    // 1. VigilPool Events
    // ==========================================

    if topic0 == CollateralDeposited::SIGNATURE_HASH {
        let decoded = CollateralDeposited::decode_raw_log(topics, data)?;
        return Ok(Some(ChainEvent::CollateralDeposited {
            account: Address::new(format!("{:?}", decoded.account)),
            asset: Address::new(format!("{:?}", decoded.asset)),
            amount: u256_to_u128(decoded.amount)?,
            block_number,
            tx_hash: tx_hash.to_string(),
        }));
    }

    if topic0 == Borrowed::SIGNATURE_HASH {
        let decoded = Borrowed::decode_raw_log(topics, data)?;
        return Ok(Some(ChainEvent::Borrowed {
            account: Address::new(format!("{:?}", decoded.account)),
            amount: u256_to_u128(decoded.amount)?,
            scaled_amount: u256_to_u128(decoded.scaledAmount)?,
            block_number,
            tx_hash: tx_hash.to_string(),
        }));
    }

    if topic0 == Repaid::SIGNATURE_HASH {
        let decoded = Repaid::decode_raw_log(topics, data)?;
        return Ok(Some(ChainEvent::Repaid {
            account: Address::new(format!("{:?}", decoded.account)),
            amount: u256_to_u128(decoded.amount)?,
            scaled_amount: u256_to_u128(decoded.scaledAmount)?,
            block_number,
            tx_hash: tx_hash.to_string(),
        }));
    }

    if topic0 == Supplied::SIGNATURE_HASH {
        let decoded = Supplied::decode_raw_log(topics, data)?;
        return Ok(Some(ChainEvent::Supplied {
            account: Address::new(format!("{:?}", decoded.account)),
            amount: u256_to_u128(decoded.amount)?,
            scaled_amount: u256_to_u128(decoded.scaledAmount)?,
            block_number,
            tx_hash: tx_hash.to_string(),
        }));
    }

    if topic0 == SupplyWithdrawn::SIGNATURE_HASH {
        let decoded = SupplyWithdrawn::decode_raw_log(topics, data)?;
        return Ok(Some(ChainEvent::SupplyWithdrawn {
            account: Address::new(format!("{:?}", decoded.account)),
            amount: u256_to_u128(decoded.amount)?,
            scaled_amount: u256_to_u128(decoded.scaledAmount)?,
            block_number,
            tx_hash: tx_hash.to_string(),
        }));
    }

    if topic0 == CollateralWithdrawn::SIGNATURE_HASH {
        let decoded = CollateralWithdrawn::decode_raw_log(topics, data)?;
        return Ok(Some(ChainEvent::CollateralWithdrawn {
            account: Address::new(format!("{:?}", decoded.account)),
            asset: Address::new(format!("{:?}", decoded.asset)),
            amount: u256_to_u128(decoded.amount)?,
            block_number,
            tx_hash: tx_hash.to_string(),
        }));
    }

    if topic0 == CollateralEnabledChanged::SIGNATURE_HASH {
        let decoded = CollateralEnabledChanged::decode_raw_log(topics, data)?;
        return Ok(Some(ChainEvent::CollateralEnabledChanged {
            account: Address::new(format!("{:?}", decoded.account)),
            asset: Address::new(format!("{:?}", decoded.asset)),
            enabled: decoded.enabled,
            block_number,
            tx_hash: tx_hash.to_string(),
        }));
    }

    if topic0 == Liquidated::SIGNATURE_HASH {
        let decoded = Liquidated::decode_raw_log(topics, data)?;
        return Ok(Some(ChainEvent::Liquidated {
            borrower: Address::new(format!("{:?}", decoded.borrower)),
            liquidator: Address::new(format!("{:?}", decoded.liquidator)),
            collateral_asset: Address::new(format!("{:?}", decoded.collateralAsset)),
            repay_amount: u256_to_u128(decoded.repayAmount)?,
            collateral_amount: u256_to_u128(decoded.collateralAmount)?,
            block_number,
            tx_hash: tx_hash.to_string(),
        }));
    }

    if topic0 == BadDebtWrittenOff::SIGNATURE_HASH {
        let decoded = BadDebtWrittenOff::decode_raw_log(topics, data)?;
        return Ok(Some(ChainEvent::BadDebtWrittenOff {
            borrower: Address::new(format!("{:?}", decoded.borrower)),
            covered_by_reserve: u256_to_u128(decoded.coveredByReserve)?,
            deficit: u256_to_u128(decoded.deficit)?,
            block_number,
            tx_hash: tx_hash.to_string(),
        }));
    }

    if topic0 == InterestAccrued::SIGNATURE_HASH {
        let decoded = InterestAccrued::decode_raw_log(topics, data)?;
        return Ok(Some(ChainEvent::InterestAccrued {
            borrow_index: u256_to_u128(decoded.borrowIndex)?,
            liquidity_index: u256_to_u128(decoded.liquidityIndex)?,
            reserve_increase: u256_to_u128(decoded.reserveIncrease)?,
            block_number,
            tx_hash: tx_hash.to_string(),
        }));
    }

    if topic0 == CollateralListed::SIGNATURE_HASH {
        let decoded = CollateralListed::decode_raw_log(topics, data)?;
        return Ok(Some(ChainEvent::CollateralListed {
            asset: Address::new(format!("{:?}", decoded.asset)),
            id: decoded.id,
            liquidation_threshold_bps: decoded.liquidationThresholdBps,
            block_number,
            tx_hash: tx_hash.to_string(),
        }));
    }

    // ==========================================
    // 2. LiquidationSettlement Events
    // ==========================================

    if topic0 == AuctionStarted::SIGNATURE_HASH {
        let decoded = AuctionStarted::decode_raw_log(topics, data)?;
        return Ok(Some(ChainEvent::AuctionStarted {
            borrower: Address::new(format!("{:?}", decoded.borrower)),
            start_block: u256_to_u64(decoded.startBlock)?,
            block_number,
            tx_hash: tx_hash.to_string(),
        }));
    }

    if topic0 == AuctionReset::SIGNATURE_HASH {
        let decoded = AuctionReset::decode_raw_log(topics, data)?;
        return Ok(Some(ChainEvent::AuctionReset {
            borrower: Address::new(format!("{:?}", decoded.borrower)),
            block_number,
            tx_hash: tx_hash.to_string(),
        }));
    }

    if topic0 == LiquidationExecuted::SIGNATURE_HASH {
        let decoded = LiquidationExecuted::decode_raw_log(topics, data)?;
        return Ok(Some(ChainEvent::LiquidationExecuted {
            borrower: Address::new(format!("{:?}", decoded.borrower)),
            liquidator: Address::new(format!("{:?}", decoded.liquidator)),
            collateral_asset: Address::new(format!("{:?}", decoded.collateralAsset)),
            repay_amount: u256_to_u128(decoded.repayAmount)?,
            collateral_seized: u256_to_u128(decoded.collateralSeized)?,
            bonus_bps: u256_to_u128(decoded.bonusBps)?,
            block_number,
            tx_hash: tx_hash.to_string(),
        }));
    }

    if topic0 == NonceUsed::SIGNATURE_HASH {
        let decoded = NonceUsed::decode_raw_log(topics, data)?;
        return Ok(Some(ChainEvent::NonceUsed {
            liquidator: Address::new(format!("{:?}", decoded.liquidator)),
            nonce: u256_to_u128(decoded.nonce)?,
            block_number,
            tx_hash: tx_hash.to_string(),
        }));
    }

    if topic0 == BadDebtSettled::SIGNATURE_HASH {
        let decoded = BadDebtSettled::decode_raw_log(topics, data)?;
        return Ok(Some(ChainEvent::BadDebtSettled {
            borrower: Address::new(format!("{:?}", decoded.borrower)),
            covered_by_reserve: u256_to_u128(decoded.coveredByReserve)?,
            deficit: u256_to_u128(decoded.deficit)?,
            block_number,
            tx_hash: tx_hash.to_string(),
        }));
    }

    // ==========================================
    // 3. Legacy and Mock Feed Events
    // ==========================================

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
    fn test_collateral_deposited_decode() {
        let account = address!("0000000000000000000000000000000000000001");
        let asset = address!("0000000000000000000000000000000000000002");
        let amount = uint!(1_000_000_000_000_000_000_U256);

        let event = CollateralDeposited { account, asset, amount };
        let raw_log = event.encode_log_data();

        let mut log = Log::default();
        log.inner.data = raw_log;

        let decoded = decode_log(&log, 42, "0xtx1").expect("decode").expect("event");
        match decoded {
            ChainEvent::CollateralDeposited { account: acc, asset: ast, amount: amt, block_number, tx_hash } => {
                assert_eq!(acc.0.to_lowercase(), format!("{:?}", account).to_lowercase());
                assert_eq!(ast.0.to_lowercase(), format!("{:?}", asset).to_lowercase());
                assert_eq!(amt, 1_000_000_000_000_000_000);
                assert_eq!(block_number, 42);
                assert_eq!(tx_hash, "0xtx1");
            }
            other => panic!("Unexpected event: {other:?}"),
        }
    }

    #[test]
    fn test_borrowed_decode() {
        let account = address!("0000000000000000000000000000000000000003");
        let amount = uint!(500_000_U256);
        let scaled_amount = uint!(480_000_U256);

        let event = Borrowed { account, amount, scaledAmount: scaled_amount };
        let raw_log = event.encode_log_data();

        let mut log = Log::default();
        log.inner.data = raw_log;

        let decoded = decode_log(&log, 43, "0xtx2").expect("decode").expect("event");
        match decoded {
            ChainEvent::Borrowed { account: acc, amount: amt, scaled_amount: s_amt, block_number, tx_hash } => {
                assert_eq!(acc.0.to_lowercase(), format!("{:?}", account).to_lowercase());
                assert_eq!(amt, 500_000);
                assert_eq!(s_amt, 480_000);
                assert_eq!(block_number, 43);
                assert_eq!(tx_hash, "0xtx2");
            }
            other => panic!("Unexpected event: {other:?}"),
        }
    }

    #[test]
    fn test_interest_accrued_decode() {
        let borrow_index = uint!(1_050_000_000_000_000_000_000_000_000_U256);
        let liquidity_index = uint!(1_020_000_000_000_000_000_000_000_000_U256);
        let reserve_increase = uint!(5_000_000_U256);

        let event = InterestAccrued {
            borrowIndex: borrow_index,
            liquidityIndex: liquidity_index,
            reserveIncrease: reserve_increase,
        };
        let raw_log = event.encode_log_data();

        let mut log = Log::default();
        log.inner.data = raw_log;

        let decoded = decode_log(&log, 44, "0xtx3").expect("decode").expect("event");
        match decoded {
            ChainEvent::InterestAccrued { borrow_index: bi, liquidity_index: li, reserve_increase: ri, block_number, tx_hash } => {
                assert_eq!(bi, 1_050_000_000_000_000_000_000_000_000);
                assert_eq!(li, 1_020_000_000_000_000_000_000_000_000);
                assert_eq!(ri, 5_000_000);
                assert_eq!(block_number, 44);
                assert_eq!(tx_hash, "0xtx3");
            }
            other => panic!("Unexpected event: {other:?}"),
        }
    }

    #[test]
    fn test_auction_started_decode() {
        let borrower = address!("0000000000000000000000000000000000000009");
        let start_block = uint!(500_U256);

        let event = AuctionStarted { borrower, startBlock: start_block };
        let raw_log = event.encode_log_data();

        let mut log = Log::default();
        log.inner.data = raw_log;

        let decoded = decode_log(&log, 500, "0xtx4").expect("decode").expect("event");
        match decoded {
            ChainEvent::AuctionStarted { borrower: b, start_block: sb, block_number, tx_hash } => {
                assert_eq!(b.0.to_lowercase(), format!("{:?}", borrower).to_lowercase());
                assert_eq!(sb, 500);
                assert_eq!(block_number, 500);
                assert_eq!(tx_hash, "0xtx4");
            }
            other => panic!("Unexpected event: {other:?}"),
        }
    }
}
