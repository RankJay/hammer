//! Access list extraction via revm execution tracing.

use alloy_primitives::Address;
use alloy_rpc_types_eth::AccessList;
use revm::context::{BlockEnv, TxEnv};
use revm::context_interface::ContextTr;
use revm::database::Database;
use revm::inspector::{Inspector, JournalExt};
use revm::{Context, InspectEvm, MainBuilder, MainContext};
use revm_inspectors::access_list::AccessListInspector;
use std::collections::HashSet;

use crate::error::HammerError;
use crate::types::RawTraceResult;

/// Inspector wrapper that extends AccessListInspector with tracking of
/// contracts created via nested CREATE/CREATE2.
pub struct HammerInspector {
    inner: AccessListInspector,
    created_contracts: HashSet<Address>,
}

impl Default for HammerInspector {
    fn default() -> Self {
        Self {
            inner: AccessListInspector::default(),
            created_contracts: HashSet::new(),
        }
    }
}

impl HammerInspector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn created_contracts(&self) -> &HashSet<Address> {
        &self.created_contracts
    }

    pub fn into_access_list(self) -> AccessList {
        self.inner.into_access_list()
    }
}

// Implement Inspector by delegating to inner and overriding create_end.
// Use same bounds as AccessListInspector (CTX: ContextTr<Journal: JournalExt>)
// and default INTR (EthInterpreter) so we can delegate to inner.
impl<CTX> Inspector<CTX> for HammerInspector
where
    CTX: ContextTr<Journal: JournalExt>,
{
    fn step(&mut self, interp: &mut revm::interpreter::Interpreter, context: &mut CTX) {
        self.inner.step(interp, context);
    }

    fn call(
        &mut self,
        context: &mut CTX,
        inputs: &mut revm::interpreter::CallInputs,
    ) -> Option<revm::interpreter::CallOutcome> {
        self.inner.call(context, inputs)
    }

    fn create(
        &mut self,
        context: &mut CTX,
        inputs: &mut revm::interpreter::CreateInputs,
    ) -> Option<revm::interpreter::CreateOutcome> {
        self.inner.create(context, inputs)
    }

    fn create_end(
        &mut self,
        context: &mut CTX,
        inputs: &revm::interpreter::CreateInputs,
        outcome: &mut revm::interpreter::CreateOutcome,
    ) {
        self.inner.create_end(context, inputs, outcome);

        if let Some(addr) = outcome.address {
            self.created_contracts.insert(addr.into());
        }
    }
}

/// Generate access list by tracing transaction execution.
///
/// Runs the transaction in a local EVM with the given database,
/// collects all accessed addresses and storage slots, and returns
/// the raw result (before warm-address optimization).
///
/// When `disable_nonce_check` is true, skips nonce validation (for replaying mined txs).
pub fn generate_access_list<DB>(
    db: DB,
    tx: TxEnv,
    block: BlockEnv,
    disable_nonce_check: bool,
) -> Result<RawTraceResult, HammerError>
where
    DB: Database,
    DB::Error: std::error::Error + Send + Sync + 'static,
{
    let inspector = HammerInspector::new();

    let mut ctx_builder = Context::mainnet()
        .with_db(db)
        .with_block(block)
        .with_tx(tx.clone());
    if disable_nonce_check {
        ctx_builder = ctx_builder.modify_cfg_chained(|cfg| cfg.disable_nonce_check = true);
    }

    let mut evm = ctx_builder.build_mainnet_with_inspector(inspector);

    let result = evm
        .inspect_one_tx(tx)
        .map_err(|e| HammerError::EvmExecution(e.to_string()))?;

    let inspector = evm.into_inspector();
    let created_contracts: Vec<Address> = inspector.created_contracts().iter().copied().collect();
    let access_list = inspector.into_access_list();

    let gas_used = result.gas_used();
    let success = result.is_success();

    Ok(RawTraceResult {
        access_list,
        created_contracts,
        gas_used,
        success,
    })
}
