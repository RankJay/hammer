//! Validation engine — diff declared vs actual access lists.

use alloy_primitives::Address;
use alloy_rpc_types_eth::AccessList;
use std::collections::{BTreeMap, BTreeSet};

use crate::gas::{
    access_list_gas_cost, ACCESS_LIST_ADDRESS_COST, ACCESS_LIST_STORAGE_KEY_COST,
    COLD_ACCOUNT_ACCESS_COST, COLD_SLOAD_COST, WARM_STORAGE_READ_COST,
};
use crate::types::{DiffEntry, GasSummary, OptimizedAccessList, ValidationReport};
use crate::warm::precompile_addresses;

/// Validate a declared access list against the optimal one.
pub fn validate(
    declared: &AccessList,
    optimal: &OptimizedAccessList,
    tx_from: Address,
    tx_to: Address,
    coinbase: Address,
) -> ValidationReport {
    let precompiles = precompile_addresses();

    // Detect duplicate entries before merging into BTreeMap (which silently deduplicates).
    let mut seen_slots: BTreeMap<Address, BTreeSet<alloy_primitives::B256>> = BTreeMap::new();
    let mut duplicate_entries = Vec::new();

    for item in &declared.0 {
        let addr_slots = seen_slots.entry(item.address).or_default();
        for &slot in &item.storage_keys {
            if !addr_slots.insert(slot) {
                duplicate_entries.push(DiffEntry::Duplicate {
                    address: item.address,
                    storage_key: slot,
                    gas_waste: ACCESS_LIST_STORAGE_KEY_COST,
                });
            }
        }
    }

    let declared_map = seen_slots;

    let optimal_map: BTreeMap<Address, BTreeSet<alloy_primitives::B256>> = optimal
        .list
        .0
        .iter()
        .map(|i| {
            let slots: BTreeSet<_> = i.storage_keys.iter().copied().collect();
            (i.address, slots)
        })
        .collect();

    let mut entries = duplicate_entries;

    for (addr, decl_slots) in &declared_map {
        if *addr == tx_from || *addr == tx_to || *addr == coinbase || precompiles.contains(addr) {
            let gas_waste =
                ACCESS_LIST_ADDRESS_COST + (decl_slots.len() as u64) * ACCESS_LIST_STORAGE_KEY_COST;
            entries.push(DiffEntry::Redundant {
                address: *addr,
                gas_waste,
            });
            continue;
        }

        if let Some(opt_slots) = optimal_map.get(addr) {
            let missing: Vec<_> = opt_slots.difference(decl_slots).copied().collect();
            if !missing.is_empty() {
                let gas_waste = (missing.len() as u64) * (COLD_SLOAD_COST - WARM_STORAGE_READ_COST);
                entries.push(DiffEntry::Incomplete {
                    address: *addr,
                    missing_slots: missing,
                    gas_waste,
                });
            }

            let stale: Vec<_> = decl_slots.difference(opt_slots).copied().collect();
            if !stale.is_empty() {
                let gas_waste = (stale.len() as u64) * ACCESS_LIST_STORAGE_KEY_COST;
                entries.push(DiffEntry::Stale {
                    address: *addr,
                    storage_keys: stale,
                    gas_waste,
                });
            }
        } else {
            let gas_waste =
                ACCESS_LIST_ADDRESS_COST + (decl_slots.len() as u64) * ACCESS_LIST_STORAGE_KEY_COST;
            entries.push(DiffEntry::Stale {
                address: *addr,
                storage_keys: decl_slots.iter().copied().collect(),
                gas_waste,
            });
        }
    }

    for (addr, opt_slots) in &optimal_map {
        if !declared_map.contains_key(addr) {
            let gas_waste = (opt_slots.len() as u64) * (COLD_SLOAD_COST - WARM_STORAGE_READ_COST);
            entries.push(DiffEntry::Missing {
                address: *addr,
                storage_keys: opt_slots.iter().copied().collect(),
                gas_waste,
            });
        }
    }

    let declared_list_cost = access_list_gas_cost(declared);
    let optimal_list_cost = access_list_gas_cost(&optimal.list);
    let waste_per_tx = declared_list_cost as i64 - optimal_list_cost as i64;
    let no_list_cost = compute_no_list_cost(&optimal_map);
    let savings_vs_no_list = no_list_cost as i64 - optimal_list_cost as i64;

    let gas_summary = GasSummary {
        declared_list_cost,
        optimal_list_cost,
        no_list_cost,
        waste_per_tx,
        savings_vs_no_list,
    };

    let is_valid = entries.is_empty();

    ValidationReport {
        entries,
        gas_summary,
        optimal_list: optimal.list.clone(),
        is_valid,
    }
}

fn compute_no_list_cost(optimal_map: &BTreeMap<Address, BTreeSet<alloy_primitives::B256>>) -> u64 {
    let mut cost = 0u64;
    for (_, slots) in optimal_map {
        cost += COLD_ACCOUNT_ACCESS_COST;
        cost += (slots.len() as u64) * COLD_SLOAD_COST;
    }
    cost
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gas::access_list_gas_cost;
    use crate::types::{DiffEntry, OptimizedAccessList};
    use alloy_primitives::B256;
    use alloy_rpc_types_eth::AccessListItem;

    fn addr(n: u8) -> Address {
        Address::from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, n])
    }

    fn slot(n: u8) -> B256 {
        let mut bytes = [0u8; 32];
        bytes[31] = n;
        B256::from(bytes)
    }

    fn make_declared(items: Vec<(Address, Vec<B256>)>) -> AccessList {
        AccessList(
            items
                .into_iter()
                .map(|(address, storage_keys)| AccessListItem {
                    address,
                    storage_keys,
                })
                .collect(),
        )
    }

    fn make_optimal(items: Vec<(Address, Vec<B256>)>) -> OptimizedAccessList {
        OptimizedAccessList::new(make_declared(items), vec![])
    }

    // Use addresses well above the precompile range (0x01..0x0a).
    fn contract_a() -> Address {
        addr(20)
    }
    fn contract_b() -> Address {
        addr(21)
    }
    fn from_addr() -> Address {
        addr(200)
    }
    fn to_addr() -> Address {
        addr(201)
    }
    fn coinbase_addr() -> Address {
        addr(202)
    }

    #[test]
    fn test_perfect_match_is_valid() {
        let optimal = make_optimal(vec![(contract_a(), vec![slot(1)])]);
        let declared = make_declared(vec![(contract_a(), vec![slot(1)])]);
        let report = validate(&declared, &optimal, from_addr(), to_addr(), coinbase_addr());
        assert!(report.is_valid);
        assert!(report.entries.is_empty());
    }

    #[test]
    fn test_missing_address() {
        let optimal = make_optimal(vec![(contract_a(), vec![slot(1)])]);
        let declared = make_declared(vec![]);
        let report = validate(&declared, &optimal, from_addr(), to_addr(), coinbase_addr());
        assert!(!report.is_valid);
        assert!(matches!(report.entries[0], DiffEntry::Missing { .. }));
        if let DiffEntry::Missing { address, .. } = &report.entries[0] {
            assert_eq!(*address, contract_a());
        }
    }

    #[test]
    fn test_stale_address() {
        let optimal = make_optimal(vec![]);
        let declared = make_declared(vec![(contract_a(), vec![])]);
        let report = validate(&declared, &optimal, from_addr(), to_addr(), coinbase_addr());
        assert!(!report.is_valid);
        assert!(matches!(report.entries[0], DiffEntry::Stale { .. }));
    }

    #[test]
    fn test_incomplete_slots() {
        let optimal = make_optimal(vec![(contract_a(), vec![slot(1), slot(2)])]);
        let declared = make_declared(vec![(contract_a(), vec![slot(1)])]);
        let report = validate(&declared, &optimal, from_addr(), to_addr(), coinbase_addr());
        assert!(!report.is_valid);
        let incomplete = report
            .entries
            .iter()
            .find(|e| matches!(e, DiffEntry::Incomplete { .. }));
        assert!(incomplete.is_some());
        if let DiffEntry::Incomplete { missing_slots, .. } = incomplete.unwrap() {
            assert_eq!(missing_slots, &vec![slot(2)]);
        }
    }

    #[test]
    fn test_stale_slots() {
        let optimal = make_optimal(vec![(contract_a(), vec![slot(1)])]);
        let declared = make_declared(vec![(contract_a(), vec![slot(1), slot(2)])]);
        let report = validate(&declared, &optimal, from_addr(), to_addr(), coinbase_addr());
        assert!(!report.is_valid);
        let stale = report.entries.iter().find(
            |e| matches!(e, DiffEntry::Stale { storage_keys, .. } if !storage_keys.is_empty()),
        );
        assert!(stale.is_some());
        if let DiffEntry::Stale { storage_keys, .. } = stale.unwrap() {
            assert!(storage_keys.contains(&slot(2)));
        }
    }

    #[test]
    fn test_incomplete_and_stale_same_address() {
        // Optimal: {s1, s2}; Declared: {s1, s3} → Incomplete(s2) + Stale(s3)
        let optimal = make_optimal(vec![(contract_a(), vec![slot(1), slot(2)])]);
        let declared = make_declared(vec![(contract_a(), vec![slot(1), slot(3)])]);
        let report = validate(&declared, &optimal, from_addr(), to_addr(), coinbase_addr());
        assert!(!report.is_valid);
        assert!(report
            .entries
            .iter()
            .any(|e| matches!(e, DiffEntry::Incomplete { .. })));
        assert!(report.entries.iter().any(|e| matches!(e, DiffEntry::Stale { storage_keys, .. } if storage_keys.contains(&slot(3)))));
    }

    #[test]
    fn test_redundant_tx_from() {
        let optimal = make_optimal(vec![]);
        let declared = make_declared(vec![(from_addr(), vec![])]);
        let report = validate(&declared, &optimal, from_addr(), to_addr(), coinbase_addr());
        assert!(report
            .entries
            .iter()
            .any(|e| matches!(e, DiffEntry::Redundant { address, .. } if *address == from_addr())));
    }

    #[test]
    fn test_redundant_tx_to() {
        let optimal = make_optimal(vec![]);
        let declared = make_declared(vec![(to_addr(), vec![])]);
        let report = validate(&declared, &optimal, from_addr(), to_addr(), coinbase_addr());
        assert!(report
            .entries
            .iter()
            .any(|e| matches!(e, DiffEntry::Redundant { address, .. } if *address == to_addr())));
    }

    #[test]
    fn test_redundant_coinbase() {
        let optimal = make_optimal(vec![]);
        let declared = make_declared(vec![(coinbase_addr(), vec![])]);
        let report = validate(&declared, &optimal, from_addr(), to_addr(), coinbase_addr());
        assert!(report.entries.iter().any(
            |e| matches!(e, DiffEntry::Redundant { address, .. } if *address == coinbase_addr())
        ));
    }

    #[test]
    fn test_redundant_precompile() {
        let precompile = addr(1); // 0x01 — well within precompile range
        let optimal = make_optimal(vec![]);
        let declared = make_declared(vec![(precompile, vec![])]);
        let report = validate(&declared, &optimal, from_addr(), to_addr(), coinbase_addr());
        assert!(report
            .entries
            .iter()
            .any(|e| matches!(e, DiffEntry::Redundant { address, .. } if *address == precompile)));
    }

    #[test]
    fn test_duplicate_slots() {
        let optimal = make_optimal(vec![(contract_a(), vec![slot(1)])]);
        // Duplicate slot(1) in declared for contract_a.
        let declared = AccessList(vec![AccessListItem {
            address: contract_a(),
            storage_keys: vec![slot(1), slot(1)],
        }]);
        let report = validate(&declared, &optimal, from_addr(), to_addr(), coinbase_addr());
        assert!(report
            .entries
            .iter()
            .any(|e| matches!(e, DiffEntry::Duplicate { .. })));
    }

    #[test]
    fn test_gas_summary_waste() {
        // Declared has a stale entry; optimal has nothing.
        let optimal = make_optimal(vec![]);
        let declared = make_declared(vec![(contract_a(), vec![slot(1)])]);
        let report = validate(&declared, &optimal, from_addr(), to_addr(), coinbase_addr());
        let expected_declared_cost = access_list_gas_cost(&declared);
        let expected_optimal_cost = access_list_gas_cost(&optimal.list);
        assert_eq!(
            report.gas_summary.declared_list_cost,
            expected_declared_cost
        );
        assert_eq!(report.gas_summary.optimal_list_cost, expected_optimal_cost);
        assert_eq!(
            report.gas_summary.waste_per_tx,
            expected_declared_cost as i64 - expected_optimal_cost as i64
        );
    }

    #[test]
    fn test_gas_summary_savings_vs_no_list() {
        let optimal = make_optimal(vec![(contract_a(), vec![slot(1)])]);
        let declared = make_declared(vec![(contract_a(), vec![slot(1)])]);
        let report = validate(&declared, &optimal, from_addr(), to_addr(), coinbase_addr());
        // no_list_cost = COLD_ACCOUNT_ACCESS_COST + COLD_SLOAD_COST
        let expected_no_list = COLD_ACCOUNT_ACCESS_COST + COLD_SLOAD_COST;
        assert_eq!(report.gas_summary.no_list_cost, expected_no_list);
        assert_eq!(
            report.gas_summary.savings_vs_no_list,
            expected_no_list as i64 - report.gas_summary.optimal_list_cost as i64
        );
    }

    #[test]
    fn test_no_list_cost_calculation() {
        // 2 contracts: contract_a with 1 slot, contract_b with 2 slots.
        let optimal = make_optimal(vec![
            (contract_a(), vec![slot(1)]),
            (contract_b(), vec![slot(1), slot(2)]),
        ]);
        let declared = make_declared(vec![
            (contract_a(), vec![slot(1)]),
            (contract_b(), vec![slot(1), slot(2)]),
        ]);
        let report = validate(&declared, &optimal, from_addr(), to_addr(), coinbase_addr());
        let expected = 2 * COLD_ACCOUNT_ACCESS_COST + 3 * COLD_SLOAD_COST;
        assert_eq!(report.gas_summary.no_list_cost, expected);
    }

    #[test]
    fn test_redundant_gas_waste_includes_slots() {
        // Redundant address with 2 slots: waste = ADDRESS_COST + 2 * SLOT_COST
        let optimal = make_optimal(vec![]);
        let declared = make_declared(vec![(from_addr(), vec![slot(1), slot(2)])]);
        let report = validate(&declared, &optimal, from_addr(), to_addr(), coinbase_addr());
        let redundant = report
            .entries
            .iter()
            .find(|e| matches!(e, DiffEntry::Redundant { .. }))
            .unwrap();
        assert_eq!(
            redundant.gas_waste(),
            ACCESS_LIST_ADDRESS_COST + 2 * ACCESS_LIST_STORAGE_KEY_COST
        );
    }
}
