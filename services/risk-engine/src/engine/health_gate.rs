//! Fail-closed health gating for Vigil Risk Engine (RS-06).
//! Strictly halts signal emission when upstream conditions are degraded:
//! 1. Protocol or Oracle is Frozen.
//! 2. Indexer heartbeat is stale beyond max_lag_seconds.
//! 3. Chain head has unresolved reorg retractions.
//! 4. Engine is catching up / replaying historical logs.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum FailClosedReason {
    #[error("Protocol or oracle is frozen (Freeze Mode Active)")]
    ProtocolFrozen,
    #[error("Indexer heartbeat is stale: lag {lag_secs}s exceeds threshold {max_lag}s")]
    StaleIndexerLag { lag_secs: u64, max_lag: u64 },
    #[error("Unresolved chain reorg retractions pending: count {0}")]
    UnresolvedRetracts(u32),
    #[error("Risk engine is catching up or replaying historical blocks")]
    CatchingUp,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct HealthSummary {
    pub status: &'static str,
    pub is_frozen: bool,
    pub is_catching_up: bool,
    pub unresolved_retracts: u32,
    pub lag_seconds: u64,
    pub max_lag_seconds: u64,
    pub can_emit_signals: bool,
    pub failure_reason: Option<String>,
}

#[derive(Debug)]
pub struct HealthGate {
    is_frozen: AtomicBool,
    is_catching_up: AtomicBool,
    last_heartbeat_timestamp: AtomicU64,
    unresolved_retracts: AtomicU32,
    max_lag_seconds: u64,
}

impl HealthGate {
    pub fn new(max_lag_seconds: u64) -> Arc<Self> {
        Arc::new(Self {
            is_frozen: AtomicBool::new(false),
            is_catching_up: AtomicBool::new(false),
            last_heartbeat_timestamp: AtomicU64::new(0),
            unresolved_retracts: AtomicU32::new(0),
            max_lag_seconds,
        })
    }

    pub fn set_frozen(&self, frozen: bool) {
        self.is_frozen.store(frozen, Ordering::SeqCst);
    }

    pub fn is_frozen(&self) -> bool {
        self.is_frozen.load(Ordering::SeqCst)
    }

    pub fn set_catching_up(&self, catching_up: bool) {
        self.is_catching_up.store(catching_up, Ordering::SeqCst);
    }

    pub fn record_heartbeat(&self, timestamp: u64) {
        self.last_heartbeat_timestamp
            .store(timestamp, Ordering::SeqCst);
    }

    pub fn increment_unresolved_retracts(&self) {
        self.unresolved_retracts.fetch_add(1, Ordering::SeqCst);
    }

    pub fn decrement_unresolved_retracts(&self) {
        let _ = self
            .unresolved_retracts
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |val| {
                Some(val.saturating_sub(1))
            });
    }

    pub fn reset_unresolved_retracts(&self) {
        self.unresolved_retracts.store(0, Ordering::SeqCst);
    }

    /// Evaluates if the engine is healthy enough to emit liquidation signals.
    /// Returns Ok(()) if fully healthy, or Err(FailClosedReason) if any fail-closed rule triggers.
    pub fn can_emit_signals(&self, current_time_secs: u64) -> Result<(), FailClosedReason> {
        if self.is_frozen.load(Ordering::SeqCst) {
            return Err(FailClosedReason::ProtocolFrozen);
        }

        if self.is_catching_up.load(Ordering::SeqCst) {
            return Err(FailClosedReason::CatchingUp);
        }

        let retractions = self.unresolved_retracts.load(Ordering::SeqCst);
        if retractions > 0 {
            return Err(FailClosedReason::UnresolvedRetracts(retractions));
        }

        let last_hb = self.last_heartbeat_timestamp.load(Ordering::SeqCst);
        if last_hb > 0 && current_time_secs > last_hb {
            let lag = current_time_secs - last_hb;
            if lag > self.max_lag_seconds {
                return Err(FailClosedReason::StaleIndexerLag {
                    lag_secs: lag,
                    max_lag: self.max_lag_seconds,
                });
            }
        }

        Ok(())
    }

    pub fn summary(&self, current_time_secs: u64) -> HealthSummary {
        let check = self.can_emit_signals(current_time_secs);
        let is_frozen = self.is_frozen.load(Ordering::SeqCst);
        let is_catching_up = self.is_catching_up.load(Ordering::SeqCst);
        let unresolved_retracts = self.unresolved_retracts.load(Ordering::SeqCst);
        let last_hb = self.last_heartbeat_timestamp.load(Ordering::SeqCst);
        let lag_seconds = if last_hb > 0 && current_time_secs > last_hb {
            current_time_secs - last_hb
        } else {
            0
        };

        let (status, failure_reason) = match check {
            Ok(_) => ("UP", None),
            Err(ref err) => ("DEGRADED", Some(err.to_string())),
        };

        HealthSummary {
            status,
            is_frozen,
            is_catching_up,
            unresolved_retracts,
            lag_seconds,
            max_lag_seconds: self.max_lag_seconds,
            can_emit_signals: check.is_ok(),
            failure_reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_gate_normal_operation() {
        let gate = HealthGate::new(60);
        gate.record_heartbeat(100);
        assert_eq!(gate.can_emit_signals(120), Ok(()));
        let summary = gate.summary(120);
        assert_eq!(summary.status, "UP");
        assert!(summary.can_emit_signals);
    }

    #[test]
    fn test_health_gate_protocol_frozen() {
        let gate = HealthGate::new(60);
        gate.record_heartbeat(100);
        gate.set_frozen(true);
        assert_eq!(
            gate.can_emit_signals(110),
            Err(FailClosedReason::ProtocolFrozen)
        );
        let summary = gate.summary(110);
        assert_eq!(summary.status, "DEGRADED");
        assert!(!summary.can_emit_signals);
    }

    #[test]
    fn test_health_gate_stale_indexer_lag() {
        let gate = HealthGate::new(30);
        gate.record_heartbeat(100);
        assert_eq!(
            gate.can_emit_signals(140),
            Err(FailClosedReason::StaleIndexerLag {
                lag_secs: 40,
                max_lag: 30
            })
        );
    }

    #[test]
    fn test_health_gate_unresolved_retracts() {
        let gate = HealthGate::new(60);
        gate.record_heartbeat(100);
        gate.increment_unresolved_retracts();
        assert_eq!(
            gate.can_emit_signals(110),
            Err(FailClosedReason::UnresolvedRetracts(1))
        );
        gate.decrement_unresolved_retracts();
        assert_eq!(gate.can_emit_signals(110), Ok(()));
    }
}
