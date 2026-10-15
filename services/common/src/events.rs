//! NATS JetStream messaging events and schema definitions.

use crate::types::{Address, OraclePrice};
use serde::{Deserialize, Serialize};

pub const SUBJECT_CHAIN_EVENTS: &str = "vigil.events.chain";
pub const SUBJECT_RETRACT_EVENTS: &str = "vigil.events.retract";
pub const SUBJECT_LIQUIDATION_SIGNALS: &str = "vigil.signals.liquidation";
pub const SUBJECT_REORG_NOTICES: &str = "vigil.notices.reorg";

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IdempotencyKey {
    pub block_hash: String,
    pub log_index: u64,
}

impl IdempotencyKey {
    pub fn new(block_hash: impl Into<String>, log_index: u64) -> Self {
        Self {
            block_hash: block_hash.into(),
            log_index,
        }
    }

    pub fn to_key_string(&self) -> String {
        format!("{}:{}", self.block_hash, self.log_index)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceContext {
    pub trace_id: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChainEvent {
    Deposit {
        account: Address,
        asset: Address,
        amount: u128,
        block_number: u64,
        tx_hash: String,
    },
    Borrow {
        account: Address,
        amount: u128,
        block_number: u64,
        tx_hash: String,
    },
    Repay {
        account: Address,
        amount: u128,
        block_number: u64,
        tx_hash: String,
    },
    LiquidationCall {
        collateral_asset: Address,
        user: Address,
        debt_to_cover: u128,
        liquidated_collateral_amount: u128,
        liquidator: Address,
        block_number: u64,
        tx_hash: String,
    },
    PriceUpdated(OraclePrice),
}

/// Enriched envelope for forward protocol events published by the Indexer (RS-01, RS-04).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedEvent {
    pub trace_id: String,
    pub idempotency_key: IdempotencyKey,
    pub block_number: u64,
    pub block_hash: String,
    pub tx_hash: String,
    pub log_index: u64,
    pub event: ChainEvent,
    pub timestamp: u64,
}

impl IndexedEvent {
    pub fn new(
        trace_id: impl Into<String>,
        block_number: u64,
        block_hash: impl Into<String>,
        tx_hash: impl Into<String>,
        log_index: u64,
        event: ChainEvent,
        timestamp: u64,
    ) -> Self {
        let b_hash = block_hash.into();
        let idempotency_key = IdempotencyKey::new(b_hash.clone(), log_index);
        Self {
            trace_id: trace_id.into(),
            idempotency_key,
            block_number,
            block_hash: b_hash,
            tx_hash: tx_hash.into(),
            log_index,
            event,
            timestamp,
        }
    }
}

/// Retraction envelope published when a block is orphaned during a reorg (RS-02).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetractEvent {
    pub trace_id: String,
    pub idempotency_key: IdempotencyKey,
    pub block_number: u64,
    pub block_hash: String,
    pub tx_hash: String,
    pub log_index: u64,
    pub original_event: Option<ChainEvent>,
    pub timestamp: u64,
}

impl RetractEvent {
    pub fn new(
        trace_id: impl Into<String>,
        block_number: u64,
        block_hash: impl Into<String>,
        tx_hash: impl Into<String>,
        log_index: u64,
        original_event: Option<ChainEvent>,
        timestamp: u64,
    ) -> Self {
        let b_hash = block_hash.into();
        let idempotency_key = IdempotencyKey::new(b_hash.clone(), log_index);
        Self {
            trace_id: trace_id.into(),
            idempotency_key,
            block_number,
            block_hash: b_hash,
            tx_hash: tx_hash.into(),
            log_index,
            original_event,
            timestamp,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiquidationSignal {
    pub trace_id: String,
    pub dedupe_key: String,
    pub borrower: Address,
    pub collateral_asset: Address,
    pub debt_asset: Address,
    pub health_factor_wad: u128,
    pub total_debt_wad: u128,
    pub total_collateral_value_wad: u128,
    pub debt_to_cover: u128,
    pub price_snapshot_wad: u128,
    pub detected_at_block: u64,
    pub block_hash: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReorgNotice {
    pub reorg_depth: u64,
    pub from_block: u64,
    pub to_block: u64,
    pub new_head_hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_idempotency_key_format_and_equality() {
        let key1 = IdempotencyKey::new("0xabc123", 4);
        let key2 = IdempotencyKey::new("0xabc123", 4);
        let key3 = IdempotencyKey::new("0xabc123", 5);
        let key4 = IdempotencyKey::new("0xdef456", 4);

        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
        assert_ne!(key1, key4);
        assert_eq!(key1.to_key_string(), "0xabc123:4");
    }

    #[test]
    fn test_indexed_event_serialization_roundtrip() {
        let event = IndexedEvent::new(
            "trace-001",
            100,
            "0x1111",
            "0x2222",
            1,
            ChainEvent::Deposit {
                account: Address::new("0xUser1"),
                asset: Address::new("0xAsset1"),
                amount: 500_000,
                block_number: 100,
                tx_hash: "0x2222".into(),
            },
            1700000000,
        );

        let json = serde_json::to_string(&event).expect("serialize");
        let deserialized: IndexedEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(event, deserialized);
        assert_eq!(deserialized.idempotency_key.to_key_string(), "0x1111:1");
    }

    #[test]
    fn test_retract_event_serialization_roundtrip() {
        let retract = RetractEvent::new(
            "trace-retract-001",
            100,
            "0x1111",
            "0x2222",
            1,
            Some(ChainEvent::Borrow {
                account: Address::new("0xUser2"),
                amount: 300_000,
                block_number: 100,
                tx_hash: "0x2222".into(),
            }),
            1700000005,
        );

        let json = serde_json::to_string(&retract).expect("serialize");
        let deserialized: RetractEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(retract, deserialized);
        assert_eq!(deserialized.idempotency_key.to_key_string(), "0x1111:1");
    }
}
