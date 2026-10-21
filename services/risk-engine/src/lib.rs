//! Vigil Risk Engine Microservice Library
//!
//! Provides:
//! 1. Parity math valuation and health factor calculations (RS-03).
//! 2. Idempotent event ledger and Postgres transactional mutations (RS-04).
//! 3. Deterministic reorg unwinding and outbox signal cancellations (RS-05).
//! 4. Fail-closed health gating and HTTP endpoints (RS-06).

pub mod api;
pub mod config;
pub mod consumer;
pub mod db;
pub mod engine;
pub mod publisher;

pub use config::Config;
