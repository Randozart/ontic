//! Ontic — stochastic specification compiler.
//!
//! Users write gens (`.ont`); the forge proposes sketch candidates; the
//! deterministic sieve decides. Verified winners lower to MLIR and land in
//! the content-addressed vault.

pub mod ask;
pub mod check;
pub mod corpus;
pub mod cloud;
pub mod dotenv;
pub mod forge;
pub mod genrand;
pub mod http;
pub mod interp;
pub mod lower;
pub mod lower_llvm;
pub mod ous;
pub mod overfit;
pub mod pipeline;
pub mod probes;
pub mod nous;
pub mod probes_solver;
pub mod program;
pub mod recipe;
pub mod rng;
pub mod sampler;
pub mod sha256;
pub mod sieve;
pub mod sketch;
pub mod vault;
pub mod gen;
