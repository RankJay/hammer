//! Pre-warm a CacheDB by fetching the complete pre-execution state before
//! handing the database to revm. Cuts wall time for complex transactions from
//! tens of seconds (sequential AlloyDB RPC calls during EVM execution) to
//! near-zero (cache hits after one parallel batch upfront).
//!
//! Strategy: call `debug_traceCall` with `prestateTracer` to get the node's
//! complete pre-execution state for every address and storage slot the
//! transaction will touch, then populate a `CacheDB<AlloyDB>` and return it.
//! revm hits the cache on every access; any residual misses fall through to
//! AlloyDB normally — correctness is unaffected.
//!
//! Falls back to the `eth_createAccessList` hint + parallel fetch approach if
//! the node does not support `debug_traceCall` (e.g. Infura).

use alloy::network::Ethereum;
use alloy_eips::BlockId;
use alloy_primitives::{Address, U256};
use alloy_provider::{DynProvider, Provider};
use alloy_rpc_types_eth::{AccessList, AccessListItem, TransactionRequest};
use alloy_rpc_types_trace::geth::{
    pre_state::PreStateFrame, GethDebugBuiltInTracerType, GethDebugTracerType,
    GethDebugTracingCallOptions, GethDebugTracingOptions,
};
use futures::future::join_all;
use revm::database::{AlloyDB, CacheDB};
use revm::database_interface::{WrapDatabaseAsync, WrapDatabaseRef};
use revm::primitives::KECCAK_EMPTY;
use revm::state::{AccountInfo, Bytecode};
use std::collections::{BTreeMap, HashMap, HashSet};

pub type PrewarmedDB =
    CacheDB<WrapDatabaseRef<WrapDatabaseAsync<AlloyDB<Ethereum, DynProvider<Ethereum>>>>>;

/// Build a pre-warmed `CacheDB` for the given transaction at `state_block`.
///
/// Tries `debug_traceCall` with `prestateTracer` first (one RPC call, 100%
/// coverage). Falls back to `eth_createAccessList` + parallel fetch if the
/// node doesn't support the debug namespace.
pub async fn build(
    provider: DynProvider<Ethereum>,
    state_block: BlockId,
    hint_block: BlockId,
    tx_req: TransactionRequest,
    declared: &AccessList,
) -> eyre::Result<PrewarmedDB> {
    use alloy_provider::ext::DebugApi;

    let trace_opts = GethDebugTracingCallOptions {
        tracing_options: GethDebugTracingOptions {
            tracer: Some(GethDebugTracerType::BuiltInTracer(
                GethDebugBuiltInTracerType::PreStateTracer,
            )),
            ..Default::default()
        },
        ..Default::default()
    };

    // One RPC call returns every account + storage slot the tx will touch.
    let pre_state_map: Option<
        BTreeMap<Address, alloy_rpc_types_trace::geth::pre_state::AccountState>,
    > = provider
        .debug_trace_call_prestate(tx_req.clone(), hint_block, trace_opts)
        .await
        .ok()
        .and_then(|frame| match frame {
            PreStateFrame::Default(mode) => Some(mode.0),
            _ => None,
        });

    // Build the underlying AlloyDB stack.
    let alloy_db = AlloyDB::new(provider.clone(), state_block);
    let async_db = WrapDatabaseAsync::new(alloy_db)
        .ok_or_else(|| eyre::eyre!("WrapDatabaseAsync requires tokio runtime"))?;
    let inner = WrapDatabaseRef::from(async_db);
    let mut cache_db = CacheDB::new(inner);

    if let Some(state) = pre_state_map {
        // Populate the cache directly from the prestate — zero additional RPCs.
        for (addr, account) in state {
            let bytecode = account
                .code
                .filter(|b| !b.is_empty())
                .map(Bytecode::new_raw)
                .unwrap_or_default();
            let code_hash = if bytecode.is_empty() {
                KECCAK_EMPTY
            } else {
                bytecode.hash_slow()
            };
            cache_db.insert_account_info(
                addr,
                AccountInfo {
                    balance: account.balance.unwrap_or(U256::ZERO),
                    nonce: account.nonce.unwrap_or(0),
                    code_hash,
                    code: Some(bytecode),
                    account_id: None,
                },
            );
            for (slot, value) in account.storage {
                let slot_u256 = U256::from_be_bytes(slot.0);
                let value_u256 = U256::from_be_bytes(value.0);
                let _ = cache_db.insert_account_storage(addr, slot_u256, value_u256);
            }
        }
    } else {
        // Fallback: eth_createAccessList hint + parallel fetch.
        // Used when the node doesn't expose the debug namespace.
        let node_hint: Option<AccessList> = provider
            .create_access_list(&tx_req)
            .block_id(hint_block)
            .await
            .ok()
            .map(|r| r.access_list);

        let hint_list = merge_access_lists(node_hint.as_ref(), declared);

        let mut addr_slots: HashMap<Address, HashSet<U256>> = HashMap::new();
        for item in hint_list.0.iter() {
            let entry = addr_slots.entry(item.address).or_default();
            for key in &item.storage_keys {
                entry.insert(U256::from_be_bytes(key.0));
            }
        }

        let addresses: Vec<Address> = addr_slots.keys().copied().collect();

        let account_futs: Vec<_> = addresses
            .iter()
            .map(|&addr| {
                let p = provider.clone();
                let b = state_block;
                async move {
                    let (balance, nonce, code) = tokio::join!(
                        async { p.get_balance(addr).block_id(b).await.unwrap_or(U256::ZERO) },
                        async { p.get_transaction_count(addr).block_id(b).await.unwrap_or(0) },
                        async { p.get_code_at(addr).block_id(b).await.unwrap_or_default() },
                    );
                    (addr, balance, nonce, code)
                }
            })
            .collect();

        let storage_tasks: Vec<(Address, U256)> = addr_slots
            .iter()
            .flat_map(|(&addr, slots)| slots.iter().map(move |&slot| (addr, slot)))
            .collect();

        let storage_futs: Vec<_> = storage_tasks
            .into_iter()
            .map(|(addr, slot)| {
                let p = provider.clone();
                let b = state_block;
                async move {
                    let value = p
                        .get_storage_at(addr, slot)
                        .block_id(b)
                        .await
                        .unwrap_or(U256::ZERO);
                    (addr, slot, value)
                }
            })
            .collect();

        let (account_results, storage_results) =
            tokio::join!(join_all(account_futs), join_all(storage_futs));

        for (addr, balance, nonce, code_bytes) in account_results {
            let bytecode = if code_bytes.is_empty() {
                Bytecode::default()
            } else {
                Bytecode::new_raw(code_bytes)
            };
            let code_hash = if bytecode.is_empty() {
                KECCAK_EMPTY
            } else {
                bytecode.hash_slow()
            };
            cache_db.insert_account_info(
                addr,
                AccountInfo {
                    balance,
                    nonce,
                    code_hash,
                    code: Some(bytecode),
                    account_id: None,
                },
            );
        }

        for (addr, slot, value) in storage_results {
            let _ = cache_db.insert_account_storage(addr, slot, value);
        }
    }

    Ok(cache_db)
}

fn merge_access_lists(a: Option<&AccessList>, b: &AccessList) -> AccessList {
    let mut map: HashMap<Address, HashSet<alloy_primitives::B256>> = HashMap::new();

    let extend = |map: &mut HashMap<Address, HashSet<alloy_primitives::B256>>,
                  list: &AccessList| {
        for item in list.0.iter() {
            let keys = map.entry(item.address).or_default();
            keys.extend(item.storage_keys.iter().copied());
        }
    };

    if let Some(list) = a {
        extend(&mut map, list);
    }
    extend(&mut map, b);

    AccessList(
        map.into_iter()
            .map(|(address, keys)| AccessListItem {
                address,
                storage_keys: keys.into_iter().collect(),
            })
            .collect(),
    )
}
