//! RS-06 Integration Test: Fail-Closed Health Gating
//!
//! Verifies that the Risk Engine strictly halts signal emission when:
//! 1. Protocol or Oracle is in Freeze Mode.
//! 2. Indexer heartbeat lag exceeds the configured threshold.
//! 3. Chain head has unresolved reorg retractions.
//! 4. Engine is catching up or replaying historical blocks.

use risk_engine::engine::health_gate::{FailClosedReason, HealthGate};

#[test]
fn test_rs06_fail_closed_protocol_frozen() {
    let gate = HealthGate::new(30);
    gate.record_heartbeat(1_000);

    // Initial state: healthy
    assert_eq!(gate.can_emit_signals(1_010), Ok(()));

    // Oracle / Protocol freezes due to detected TWAP manipulation
    gate.set_frozen(true);
    assert_eq!(
        gate.can_emit_signals(1_010),
        Err(FailClosedReason::ProtocolFrozen)
    );

    let summary = gate.summary(1_010);
    assert_eq!(summary.status, "DEGRADED");
    assert!(!summary.can_emit_signals);
    assert!(summary.is_frozen);

    // Unfreeze recovers health
    gate.set_frozen(false);
    assert_eq!(gate.can_emit_signals(1_010), Ok(()));
}

#[test]
fn test_rs06_fail_closed_stale_indexer_heartbeat() {
    let gate = HealthGate::new(15); // 15 seconds max lag
    gate.record_heartbeat(2_000);

    // 10s lag: healthy
    assert_eq!(gate.can_emit_signals(2_010), Ok(()));

    // 20s lag (> 15s max lag): fail-closed triggers
    assert_eq!(
        gate.can_emit_signals(2_020),
        Err(FailClosedReason::StaleIndexerLag {
            lag_secs: 20,
            max_lag: 15,
        })
    );

    let summary = gate.summary(2_020);
    assert_eq!(summary.status, "DEGRADED");
    assert_eq!(summary.lag_seconds, 20);
    assert!(!summary.can_emit_signals);

    // Heartbeat update recovers
    gate.record_heartbeat(2_019);
    assert_eq!(gate.can_emit_signals(2_020), Ok(()));
}

#[test]
fn test_rs06_fail_closed_unresolved_reorg_retractions() {
    let gate = HealthGate::new(60);
    gate.record_heartbeat(3_000);

    // Reorg detected: retractions arrive
    gate.increment_unresolved_retracts();
    gate.increment_unresolved_retracts();

    assert_eq!(
        gate.can_emit_signals(3_005),
        Err(FailClosedReason::UnresolvedRetracts(2))
    );

    // Unwind finishes
    gate.decrement_unresolved_retracts();
    gate.decrement_unresolved_retracts();

    assert_eq!(gate.can_emit_signals(3_005), Ok(()));
}

#[test]
fn test_rs06_fail_closed_catching_up() {
    let gate = HealthGate::new(60);
    gate.record_heartbeat(4_000);

    gate.set_catching_up(true);
    assert_eq!(
        gate.can_emit_signals(4_005),
        Err(FailClosedReason::CatchingUp)
    );

    gate.set_catching_up(false);
    assert_eq!(gate.can_emit_signals(4_005), Ok(()));
}
