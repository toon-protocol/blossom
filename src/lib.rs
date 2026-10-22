//! The TOON Protocol blob app: a Blossom server behind connector routes.
//!
//! Two listeners share one [`state::AppState`]: the write port, reached only by the connector,
//! and the public port, which serves blob reads for free. The app validates no payment; it records
//! the statement the connector makes and trusts it.

pub mod auth;
pub mod blob_files;
pub mod config;
pub mod error;
pub mod hash;
pub mod holdings;
pub mod payment;
pub mod public;
pub mod report;
pub mod state;
pub mod sunset;
pub mod upload;
