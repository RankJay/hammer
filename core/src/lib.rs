//! Hammer core library — EIP-2930 access list generation, optimization, and validation.

use alloy_primitives::Address;
use alloy_rpc_types_eth::AccessList;
use revm::context::{BlockEnv, TxEnv};
use revm::database::Database;

pub mod error;
pub mod gas;
pub mod optimizer;
pub mod tracer;
pub mod types;
pub mod validator;
pub mod warm;

pub use error::HammerError;
pub use gas::{
    access_list_gas_cost, gas_to_eth, ACCESS_LIST_ADDRESS_COST, ACCESS_LIST_STORAGE_KEY_COST,
};
pub use optimizer::optimize;
pub use tracer::generate_access_list;
pub use types::{DiffEntry, GasSummary, OptimizedAccessList, RawTraceResult, ValidationReport};

/// Generate an optimized access list for the given transaction.
pub fn generate<DB>(db: DB, tx: TxEnv, block: BlockEnv) -> Result<OptimizedAccessList, HammerError>
where
    DB: Database,
    DB::Error: std::error::Error + Send + Sync + 'static,
{
    let tx_from = tx.caller;
    let tx_to = match tx.kind {
        revm::primitives::TxKind::Call(addr) => addr,
        revm::primitives::TxKind::Create => Address::ZERO,
    };
    let coinbase = block.beneficiary;
    let raw = generate_access_list(db, tx, block, false)?;
    Ok(optimize(raw, tx_from, tx_to, coinbase))
}

/// Validate a declared access list against the optimal one from execution trace.
pub fn validate<DB>(
    db: DB,
    tx: TxEnv,
    block: BlockEnv,
    declared: AccessList,
) -> Result<ValidationReport, HammerError>
where
    DB: Database,
    DB::Error: std::error::Error + Send + Sync + 'static,
{
    let tx_from = tx.caller;
    let tx_to = match tx.kind {
        revm::primitives::TxKind::Call(addr) => addr,
        revm::primitives::TxKind::Create => Address::ZERO,
    };
    let coinbase = block.beneficiary;
    let raw = generate_access_list(db, tx, block, false)?;
    let optimal = optimize(raw, tx_from, tx_to, coinbase);

    Ok(validator::validate(
        &declared, &optimal, tx_from, tx_to, coinbase,
    ))
}

/// Validate for replay (e.g. compare): skips nonce check so mined txs can be replayed.
pub fn validate_replay<DB>(
    db: DB,
    tx: TxEnv,
    block: BlockEnv,
    declared: AccessList,
) -> Result<ValidationReport, HammerError>
where
    DB: Database,
    DB::Error: std::error::Error + Send + Sync + 'static,
{
    let tx_from = tx.caller;
    let tx_to = match tx.kind {
        revm::primitives::TxKind::Call(addr) => addr,
        revm::primitives::TxKind::Create => Address::ZERO,
    };
    let coinbase = block.beneficiary;
    let raw = generate_access_list(db, tx, block, true)?;
    let optimal = optimize(raw, tx_from, tx_to, coinbase);

    Ok(validator::validate(
        &declared, &optimal, tx_from, tx_to, coinbase,
    ))
}
