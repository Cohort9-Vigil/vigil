//! Engine core: health factor evaluation and fail-closed safety gate.

pub mod evaluator;
pub mod health_gate;

pub use evaluator::*;
pub use health_gate::*;
