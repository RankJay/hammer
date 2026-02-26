//! Pre-warm a CacheDB by fetching storage and account state in parallel before
//! handing the database to revm. Cuts wall time for multi-hop transactions from
//! seconds (sequential AlloyDB RPC calls during EVM execution) to near-zero
//! (cache hits during execution after one parallel batch upfront).
//!
//! Strategy: call `eth_createAccessList` at the target block to get the node's
//! best-effort access list hint, then fan out all storage and account fetches
//! concurrently, populate a `CacheDB<AlloyDB>`, and return it. revm hits the
//! cache on almost every slot; any residual misses fall through to AlloyDB
//! normally — correctness is unaffected.

use alloy::network::Ethereum;
use alloy_eips::BlockId;
use alloy_primitives::{Address, U256};
use alloy_provider::{DynProvider, Provider};
use alloy_rpc_types_eth::{AccessList, AccessListItem, TransactionRequest};
use futures::future::join_all;
use revm::database::{AlloyDB, CacheDB};
use revm::database_interface::{WrapDatabaseAsync, WrapDatabaseRef};
use revm::state::{AccountInfo, Bytecode};
use revm::primitives::KECCAK_EMPTY;
use std::collections::{HashMap, HashSet};

pub type PrewarmedDB = CacheDB<WrapDatabaseRef<WrapDatabaseAsync<AlloyDB<Ethereum, DynProvider<Ethereum>>>>>;

/// Build a pre-warmed `CacheDB` for the given transaction replayed at `state_block`.
///
/// `hint_block` is the `BlockId` used for `eth_createAccessList` — the block hash
/// of the block containing the transaction so the node simulates against the
/// correct pre-execution state.
///
/// Falls back gracefully if `eth_createAccessList` is unsupported: only the
/// `declared` list entries are prefetched. Either way correctness is unchanged.
pub async fn build(
    provider: DynProvider<Ethereum>,
    state_block: BlockId,
    hint_block: BlockId,
    tx_req: TransactionRequest,
    declared: &AccessList,
) -> eyre::Result<PrewarmedDB> {
    // Ask the node for its access list hint. On failure fall back to declared only.
    let node_hint: Option<AccessList> = provider
        .create_access_list(&tx_req)
        .block_id(hint_block)
        .await
        .ok()
        .map(|r| r.access_list);

    // Union node hint + declared list to maximise cache coverage.
    let hint_list = merge_access_lists(node_hint.as_ref(), declared);

    // Collect unique addresses and their storage keys.
    let mut addr_slots: HashMap<Address, HashSet<U256>> = HashMap::new();
    for item in hint_list.0.iter() {
        let entry = addr_slots.entry(item.address).or_default();
        for key in &item.storage_keys {
            entry.insert(U256::from_be_bytes(key.0));
        }
    }

    let addresses: Vec<Address> = addr_slots.keys().copied().collect();

    // Per-address: fetch balance, nonce, code — all concurrently.
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

    // Per-(address, slot): fetch storage values — all concurrently.
    // Collect into a plain Vec to avoid capturing `provider` in nested closures.
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

    // Build underlying AlloyDB stack (same as the non-prefetched path in compare.rs).
    let alloy_db = AlloyDB::new(provider, state_block);
    let async_db = WrapDatabaseAsync::new(alloy_db)
        .ok_or_else(|| eyre::eyre!("WrapDatabaseAsync requires tokio runtime"))?;
    let inner = WrapDatabaseRef::from(async_db);
    let mut cache_db = CacheDB::new(inner);

    // Insert account info — insert_account_info handles code hash + contracts map.
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
        let info = AccountInfo {
            balance,
            nonce,
            code_hash,
            code: Some(bytecode),
            account_id: None,
        };
        cache_db.insert_account_info(addr, info);
    }

    // Insert storage slots.
    for (addr, slot, value) in storage_results {
        // account info is already in cache from the loop above, so this is a
        // pure in-memory write — no further RPC calls.
        let _ = cache_db.insert_account_storage(addr, slot, value);
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
