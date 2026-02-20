//! Warm-address stripping — remove entries that are already warm by default.

use alloy_primitives::{Address, B256};
use alloy_rpc_types_eth::{AccessList, AccessListItem};
use std::collections::{BTreeMap, BTreeSet};

use crate::types::{OptimizedAccessList, RawTraceResult};
use crate::warm::precompile_addresses;

/// Optimize access list by removing warm-by-default addresses.
///
/// Removes: tx.from, tx.to (EIP-2929), block.coinbase (EIP-3651), precompiles,
/// contracts created during execution. Deduplicates/sorts for deterministic output.
pub fn optimize(
    raw: RawTraceResult,
    tx_from: Address,
    tx_to: Address,
    coinbase: Address,
) -> OptimizedAccessList {
    let precompiles = precompile_addresses();
    let created_set: BTreeSet<Address> = raw.created_contracts.into_iter().collect();

    let warm_by_default: BTreeSet<Address> = [tx_from, tx_to, coinbase]
        .into_iter()
        .filter(|a| *a != Address::ZERO)
        .collect();

    let mut removed = Vec::new();
    let mut optimized: BTreeMap<Address, BTreeSet<B256>> = BTreeMap::new();

    for item in raw.access_list.0.into_iter() {
        let addr = item.address;

        if warm_by_default.contains(&addr) {
            removed.push(addr);
            continue;
        }
        if precompiles.contains(&addr) {
            removed.push(addr);
            continue;
        }
        if created_set.contains(&addr) {
            removed.push(addr);
            continue;
        }

        let slots: BTreeSet<B256> = item.storage_keys.into_iter().collect();
        if !slots.is_empty() || !optimized.contains_key(&addr) {
            optimized.entry(addr).or_default().extend(slots);
        }
    }

    let list = AccessList(
        optimized
            .into_iter()
            .map(|(address, storage_keys)| AccessListItem {
                address,
                storage_keys: storage_keys.into_iter().collect(),
            })
            .collect(),
    );

    OptimizedAccessList::new(list, removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_rpc_types_eth::AccessListItem;

    fn addr(n: u8) -> Address {
        Address::from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, n])
    }

    fn slot(n: u8) -> B256 {
        let mut bytes = [0u8; 32];
        bytes[31] = n;
        B256::from(bytes)
    }

    fn raw(items: Vec<AccessListItem>, created: Vec<Address>) -> RawTraceResult {
        RawTraceResult {
            access_list: AccessList(items),
            created_contracts: created,
            gas_used: 21000,
            success: true,
        }
    }

    fn item(address: Address, slots: Vec<B256>) -> AccessListItem {
        AccessListItem {
            address,
            storage_keys: slots,
        }
    }

    #[test]
    fn test_removes_tx_from() {
        let from = addr(1);
        let to = addr(2);
        let coinbase = addr(3);
        let result = optimize(raw(vec![item(from, vec![])], vec![]), from, to, coinbase);
        assert!(result.list.0.is_empty());
        assert!(result.removed_addresses.contains(&from));
    }

    #[test]
    fn test_removes_tx_to() {
        let from = addr(1);
        let to = addr(2);
        let coinbase = addr(3);
        let result = optimize(raw(vec![item(to, vec![])], vec![]), from, to, coinbase);
        assert!(result.list.0.is_empty());
        assert!(result.removed_addresses.contains(&to));
    }

    #[test]
    fn test_removes_coinbase() {
        let from = addr(1);
        let to = addr(2);
        let coinbase = addr(3);
        let result = optimize(
            raw(vec![item(coinbase, vec![])], vec![]),
            from,
            to,
            coinbase,
        );
        assert!(result.list.0.is_empty());
        assert!(result.removed_addresses.contains(&coinbase));
    }

    #[test]
    fn test_removes_precompiles() {
        let from = addr(20);
        let to = addr(21);
        let coinbase = addr(22);
        // Build items for precompiles 0x01..0x0a
        let precompile_items: Vec<AccessListItem> =
            (1u8..=10).map(|i| item(addr(i), vec![])).collect();
        let result = optimize(raw(precompile_items, vec![]), from, to, coinbase);
        assert!(result.list.0.is_empty());
        assert_eq!(result.removed_addresses.len(), 10);
    }

    #[test]
    fn test_removes_created_contracts() {
        let from = addr(1);
        let to = addr(2);
        let coinbase = addr(3);
        let created = addr(50);
        let result = optimize(
            raw(vec![item(created, vec![])], vec![created]),
            from,
            to,
            coinbase,
        );
        assert!(result.list.0.is_empty());
        assert!(result.removed_addresses.contains(&created));
    }

    #[test]
    fn test_keeps_normal_addresses() {
        let from = addr(1);
        let to = addr(2);
        let coinbase = addr(3);
        let normal = addr(50);
        let result = optimize(
            raw(vec![item(normal, vec![slot(1)])], vec![]),
            from,
            to,
            coinbase,
        );
        assert_eq!(result.list.0.len(), 1);
        assert_eq!(result.list.0[0].address, normal);
        assert!(result.removed_addresses.is_empty());
    }

    #[test]
    fn test_deduplicates_slots() {
        let from = addr(1);
        let to = addr(2);
        let coinbase = addr(3);
        let normal = addr(50);
        let s1 = slot(1);
        // Same slot appears twice in the raw list for the same address.
        let items = vec![item(normal, vec![s1, s1])];
        let result = optimize(raw(items, vec![]), from, to, coinbase);
        assert_eq!(result.list.0[0].storage_keys.len(), 1);
    }

    #[test]
    fn test_deduplicates_addresses() {
        let from = addr(1);
        let to = addr(2);
        let coinbase = addr(3);
        let normal = addr(50);
        // Same address in two separate AccessListItems.
        let items = vec![item(normal, vec![slot(1)]), item(normal, vec![slot(2)])];
        let result = optimize(raw(items, vec![]), from, to, coinbase);
        assert_eq!(result.list.0.len(), 1);
        assert_eq!(result.list.0[0].storage_keys.len(), 2);
    }

    #[test]
    fn test_deterministic_ordering() {
        let from = addr(1);
        let to = addr(2);
        let coinbase = addr(3);
        // Insert addresses in descending order.
        let items = vec![
            item(addr(50), vec![]),
            item(addr(30), vec![]),
            item(addr(40), vec![]),
        ];
        let result = optimize(raw(items, vec![]), from, to, coinbase);
        let addresses: Vec<Address> = result.list.0.iter().map(|i| i.address).collect();
        let mut sorted = addresses.clone();
        sorted.sort();
        assert_eq!(addresses, sorted);
    }

    #[test]
    fn test_zero_address_not_stripped_unconditionally() {
        // Address::ZERO is only stripped when it equals tx_to (or tx_from/coinbase).
        // If tx_to is some other address, ZERO should survive.
        let from = addr(1);
        let to = addr(2);
        let coinbase = addr(3);
        let zero = Address::ZERO;
        let result = optimize(
            raw(vec![item(zero, vec![slot(1)])], vec![]),
            from,
            to,
            coinbase,
        );
        // ZERO != from, to, or coinbase, so it must be kept.
        assert_eq!(result.list.0.len(), 1);
        assert_eq!(result.list.0[0].address, zero);
    }

    #[test]
    fn test_removed_addresses_populated() {
        let from = addr(1);
        let to = addr(2);
        let coinbase = addr(3);
        let normal = addr(50);
        let items = vec![
            item(from, vec![]),
            item(to, vec![]),
            item(normal, vec![slot(1)]),
        ];
        let result = optimize(raw(items, vec![]), from, to, coinbase);
        assert!(result.removed_addresses.contains(&from));
        assert!(result.removed_addresses.contains(&to));
        assert!(!result.removed_addresses.contains(&normal));
        assert_eq!(result.list.0.len(), 1);
    }
}
