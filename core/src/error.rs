//! Typed errors for hammer-core library.

use thiserror::Error;

/// Errors that can occur during access list generation or validation.
#[derive(Debug, Error)]
pub enum HammerError {
    #[error("EVM execution failed: {0}")]
    EvmExecution(String),

    #[error("Invalid calldata: {0}")]
    InvalidCalldata(String),

    #[error("RPC/state fetch failed: {0}")]
    RpcError(#[from] Box<dyn std::error::Error + Send + Sync>),

    #[error("Invalid access list: {0}")]
    InvalidAccessList(String),
}
