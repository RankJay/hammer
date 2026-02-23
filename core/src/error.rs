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

    #[error("Unsupported transaction: {0}")]
    UnsupportedTransaction(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hammer_error_display_strings() {
        assert_eq!(
            HammerError::EvmExecution("bad".into()).to_string(),
            "EVM execution failed: bad"
        );
        assert_eq!(
            HammerError::InvalidCalldata("x".into()).to_string(),
            "Invalid calldata: x"
        );
        assert_eq!(
            HammerError::InvalidAccessList("y".into()).to_string(),
            "Invalid access list: y"
        );
        assert_eq!(
            HammerError::UnsupportedTransaction("z".into()).to_string(),
            "Unsupported transaction: z"
        );
    }
}
