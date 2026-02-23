//! EIP-2929 and EIP-2930 gas constants and calculations.

use alloy_rpc_types_eth::AccessList;

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
}
