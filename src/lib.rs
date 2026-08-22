//! Ontic — stochastic specification compiler.
//!
//! Users write wishes (`.ont`); the forge proposes sketch candidates; the
//! deterministic sieve decides. Verified winners lower to MLIR and land in
//! the content-addressed vault.

pub mod check;
pub mod forge;
pub mod http;
pub mod interp;
pub mod lower;
pub mod overfit;
pub mod pipeline;
pub mod probes;
pub mod rng;
pub mod sha256;
pub mod sieve;
pub mod sketch;
pub mod vault;
pub mod wish;
