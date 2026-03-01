//! EIP-2929 and EIP-2930 gas constants and calculations.

use alloy_rpc_types_eth::AccessList;

use crate::types::{GasDecision, OptimizedAccessList, Recommendation};

/// Cost to include an address in the access list (EIP-2930).
pub const ACCESS_LIST_ADDRESS_COST: u64 = 2400;

/// Cost to include a storage key in the access list (EIP-2930).
pub const ACCESS_LIST_STORAGE_KEY_COST: u64 = 1900;

/// Cost of first (cold) access to an account (EIP-2929).
pub const COLD_ACCOUNT_ACCESS_COST: u64 = 2600;

/// Cost of first (cold) SLOAD of a storage slot (EIP-2929).
pub const COLD_SLOAD_COST: u64 = 2100;

/// Cost of subsequent (warm) storage read (EIP-2929).
pub const WARM_STORAGE_READ_COST: u64 = 100;

/// Net gas saved per slot when including an accessed slot in the access list.
/// Cold read costs 2100, warm costs 100. Upfront cost is 1900. Net: 2000 - 1900 = 100.
pub const NET_SAVINGS_PER_ACCESSED_SLOT: i64 = (COLD_SLOAD_COST as i64)
    - (WARM_STORAGE_READ_COST as i64)
    - (ACCESS_LIST_STORAGE_KEY_COST as i64);

/// Net gas saved per address when including an accessed address in the access list.
/// Cold account costs 2600, warm is free. Upfront cost is 2400. Net: 2600 - 2400 = 200.
pub const NET_SAVINGS_PER_ACCESSED_ADDRESS: i64 =
    (COLD_ACCOUNT_ACCESS_COST as i64) - (ACCESS_LIST_ADDRESS_COST as i64);

/// Compute the total gas cost of an access list (address + storage key costs).
pub fn access_list_gas_cost(list: &AccessList) -> u64 {
    let mut cost = 0u64;
    let mut seen_addresses = std::collections::HashSet::new();

    for item in list.0.iter() {
        if seen_addresses.insert(item.address) {
            cost += ACCESS_LIST_ADDRESS_COST;
        }
        cost += (item.storage_keys.len() as u64) * ACCESS_LIST_STORAGE_KEY_COST;
    }
    cost
}

/// Compute gas decision: net delta and attach/skip recommendation.
pub fn compute_gas_decision(optimal: &OptimizedAccessList) -> GasDecision {
    let access_list_cost = access_list_gas_cost(&optimal.list);

    let mut no_list_cost = 0u64;
    let mut cold_addresses = 0u64;
    let mut cold_slots = 0u64;

    for item in &optimal.list.0 {
        cold_addresses += 1;
        no_list_cost += COLD_ACCOUNT_ACCESS_COST;
        let slot_count = item.storage_keys.len() as u64;
        cold_slots += slot_count;
        no_list_cost += slot_count * COLD_SLOAD_COST;
    }

    let net_gas_delta = no_list_cost as i64 - access_list_cost as i64;

    let address_overhead_savings = cold_addresses as i64 * NET_SAVINGS_PER_ACCESSED_ADDRESS;
    let break_even_slots = if address_overhead_savings >= 0 {
        0
    } else {
        ((-address_overhead_savings) as u64)
            .div_ceil(NET_SAVINGS_PER_ACCESSED_SLOT.unsigned_abs())
    };

    let recommendation = if net_gas_delta > 0 {
        Recommendation::Attach
    } else {
        Recommendation::Skip
    };

    GasDecision {
        access_list_cost,
        no_list_cost,
        net_gas_delta,
        break_even_slots,
        cold_addresses,
        cold_slots,
        recommendation,
    }
}

/// Convert gas amount to ETH at given gas price (in gwei).
#[inline]
pub fn gas_to_eth(gas: u64, gas_price_gwei: u64) -> f64 {
    (gas as f64) * (gas_price_gwei as f64) / 1e9
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, B256};
    use alloy_rpc_types_eth::{AccessList, AccessListItem};

    fn addr(n: u8) -> Address {
        Address::from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, n])
    }

    fn slot(n: u8) -> B256 {
        B256::from_slice(&[0u8; 31].iter().chain(&[n]).copied().collect::<Vec<_>>())
    }

    #[test]
    fn test_empty_list_cost() {
        assert_eq!(access_list_gas_cost(&AccessList::default()), 0);
    }

    #[test]
    fn test_single_address_no_slots() {
        let list = AccessList(vec![AccessListItem {
            address: addr(1),
            storage_keys: vec![],
        }]);
        assert_eq!(access_list_gas_cost(&list), ACCESS_LIST_ADDRESS_COST);
    }

    #[test]
    fn test_single_address_with_slots() {
        let list = AccessList(vec![AccessListItem {
            address: addr(1),
            storage_keys: vec![slot(1), slot(2), slot(3)],
        }]);
        assert_eq!(
            access_list_gas_cost(&list),
            ACCESS_LIST_ADDRESS_COST + 3 * ACCESS_LIST_STORAGE_KEY_COST
        );
    }

    #[test]
    fn test_multiple_addresses() {
        let list = AccessList(vec![
            AccessListItem {
                address: addr(1),
                storage_keys: vec![slot(1)],
            },
            AccessListItem {
                address: addr(2),
                storage_keys: vec![slot(1), slot(2)],
            },
        ]);
        let expected = 2 * ACCESS_LIST_ADDRESS_COST + 3 * ACCESS_LIST_STORAGE_KEY_COST;
        assert_eq!(access_list_gas_cost(&list), expected);
    }

    #[test]
    fn test_duplicate_address_counted_once() {
        // Same address in two items: address cost charged once, slot costs for all slots.
        let list = AccessList(vec![
            AccessListItem {
                address: addr(1),
                storage_keys: vec![slot(1)],
            },
            AccessListItem {
                address: addr(1),
                storage_keys: vec![slot(2)],
            },
        ]);
        let expected = ACCESS_LIST_ADDRESS_COST + 2 * ACCESS_LIST_STORAGE_KEY_COST;
        assert_eq!(access_list_gas_cost(&list), expected);
    }

    #[test]
    fn test_gas_to_eth_basic() {
        let result = gas_to_eth(1_000_000, 30);
        assert!((result - 0.03).abs() < 1e-10);
    }

    #[test]
    fn test_gas_to_eth_zero() {
        assert_eq!(gas_to_eth(0, 30), 0.0);
    }

    #[test]
    fn test_constants() {
        // Net savings per slot: cold SLOAD (2100) - warm read (100) - slot upfront (1900) = 100
        assert_eq!(NET_SAVINGS_PER_ACCESSED_SLOT, 100);
        // Net savings per address: cold account (2600) - address upfront (2400) = 200
        assert_eq!(NET_SAVINGS_PER_ACCESSED_ADDRESS, 200);
    }

    // gas_to_eth edge cases

    #[test]
    fn test_gas_to_eth_zero_gas_price() {
        // Zero gas price → zero ETH regardless of gas amount.
        assert_eq!(gas_to_eth(1_000_000, 0), 0.0);
    }

    #[test]
    fn test_gas_to_eth_one_gwei() {
        // 21000 gas at 1 gwei = 0.000021 ETH
        let result = gas_to_eth(21_000, 1);
        assert!((result - 0.000_021).abs() < 1e-12);
    }

    // access_list_gas_cost edge cases

    #[test]
    fn test_duplicate_slots_within_item_still_counted() {
        // gas cost is mechanical: slot count × SLOT_COST, duplicates are not deduplicated here.
        let list = AccessList(vec![AccessListItem {
            address: addr(1),
            storage_keys: vec![slot(1), slot(1)],
        }]);
        // Two slot entries, even though both are the same key.
        assert_eq!(
            access_list_gas_cost(&list),
            ACCESS_LIST_ADDRESS_COST + 2 * ACCESS_LIST_STORAGE_KEY_COST
        );
    }

    #[test]
    fn test_address_only_no_slots_many_addresses() {
        // Five addresses with no slots: cost = 5 × ADDRESS_COST.
        let list = AccessList(
            (1u8..=5)
                .map(|n| AccessListItem {
                    address: addr(n),
                    storage_keys: vec![],
                })
                .collect(),
        );
        assert_eq!(access_list_gas_cost(&list), 5 * ACCESS_LIST_ADDRESS_COST);
    }

    #[test]
    fn test_single_address_many_slots() {
        // One address with 10 slots.
        let list = AccessList(vec![AccessListItem {
            address: addr(1),
            storage_keys: (0u8..10).map(slot).collect(),
        }]);
        assert_eq!(
            access_list_gas_cost(&list),
            ACCESS_LIST_ADDRESS_COST + 10 * ACCESS_LIST_STORAGE_KEY_COST
        );
    }

    #[test]
    fn test_gas_to_eth_large_gas_no_panic() {
        // u64::MAX gas at 1000 gwei: uses f64 arithmetic so no integer overflow.
        let result = gas_to_eth(u64::MAX, 1000);
        assert!(result.is_finite(), "expected finite result, got {}", result);
    }

    #[test]
    fn test_gas_to_eth_large_gas_price_no_panic() {
        // 21000 gas at u64::MAX gwei: uses f64 arithmetic so no integer overflow.
        let result = gas_to_eth(21_000, u64::MAX);
        assert!(result.is_finite(), "expected finite result, got {}", result);
    }

    // compute_gas_decision tests

    #[test]
    fn test_compute_gas_decision_empty_list() {
        use crate::types::OptimizedAccessList;

        let optimal = OptimizedAccessList::new(AccessList::default(), vec![]);
        let d = compute_gas_decision(&optimal);
        assert_eq!(d.net_gas_delta, 0);
        assert_eq!(d.recommendation, crate::types::Recommendation::Skip);
        assert_eq!(d.cold_addresses, 0);
        assert_eq!(d.cold_slots, 0);
    }

    #[test]
    fn test_compute_gas_decision_single_address_no_slots() {
        use crate::types::OptimizedAccessList;

        let list = AccessList(vec![AccessListItem {
            address: addr(1),
            storage_keys: vec![],
        }]);
        let optimal = OptimizedAccessList::new(list, vec![]);
        let d = compute_gas_decision(&optimal);
        assert_eq!(d.net_gas_delta, 200); // COLD_ACCOUNT - ADDRESS_COST
        assert_eq!(d.recommendation, crate::types::Recommendation::Attach);
        assert_eq!(d.cold_addresses, 1);
        assert_eq!(d.cold_slots, 0);
        assert_eq!(d.break_even_slots, 0);
    }

    #[test]
    fn test_compute_gas_decision_single_address_one_slot() {
        use crate::types::OptimizedAccessList;

        let list = AccessList(vec![AccessListItem {
            address: addr(1),
            storage_keys: vec![slot(1)],
        }]);
        let optimal = OptimizedAccessList::new(list, vec![]);
        let d = compute_gas_decision(&optimal);
        assert_eq!(d.net_gas_delta, 400); // no_list (2600+2100) - al_cost (2400+1900)
        assert_eq!(d.recommendation, crate::types::Recommendation::Attach);
        assert_eq!(d.cold_addresses, 1);
        assert_eq!(d.cold_slots, 1);
    }

    #[test]
    fn test_compute_gas_decision_multiple_addresses_slots() {
        use crate::types::OptimizedAccessList;

        let list = AccessList(vec![
            AccessListItem {
                address: addr(1),
                storage_keys: vec![slot(1), slot(2)],
            },
            AccessListItem {
                address: addr(2),
                storage_keys: vec![slot(1)],
            },
        ]);
        let optimal = OptimizedAccessList::new(list, vec![]);
        let d = compute_gas_decision(&optimal);
        let expected_no_list = 2 * COLD_ACCOUNT_ACCESS_COST + 3 * COLD_SLOAD_COST;
        let expected_al_cost = 2 * ACCESS_LIST_ADDRESS_COST + 3 * ACCESS_LIST_STORAGE_KEY_COST;
        assert_eq!(d.no_list_cost, expected_no_list);
        assert_eq!(d.access_list_cost, expected_al_cost);
        assert_eq!(d.net_gas_delta, expected_no_list as i64 - expected_al_cost as i64);
        assert_eq!(d.cold_addresses, 2);
        assert_eq!(d.cold_slots, 3);
    }

    #[test]
    fn test_compute_gas_decision_serde_roundtrip() {
        use crate::types::{GasDecision, OptimizedAccessList, Recommendation};

        let list = AccessList(vec![AccessListItem {
            address: addr(1),
            storage_keys: vec![slot(1)],
        }]);
        let optimal = OptimizedAccessList::new(list, vec![]);
        let d = compute_gas_decision(&optimal);
        let json = serde_json::to_string(&d).unwrap();
        let decoded: GasDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.net_gas_delta, d.net_gas_delta);
        assert_eq!(decoded.recommendation, Recommendation::Attach);
    }

    #[test]
    fn test_recommendation_serde_roundtrip() {
        use crate::types::Recommendation;

        let attach_json = serde_json::to_string(&Recommendation::Attach).unwrap();
        assert!(attach_json.contains("attach"));
        let skip_json = serde_json::to_string(&Recommendation::Skip).unwrap();
        assert!(skip_json.contains("skip"));
        let decoded_attach: Recommendation = serde_json::from_str(&attach_json).unwrap();
        let decoded_skip: Recommendation = serde_json::from_str(&skip_json).unwrap();
        assert_eq!(decoded_attach, Recommendation::Attach);
        assert_eq!(decoded_skip, Recommendation::Skip);
    }
}
