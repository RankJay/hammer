//! Domain types for access list validation reports.

use alloy_primitives::Address;
use alloy_rpc_types_eth::AccessList;
use serde::{Deserialize, Serialize};

/// A single diff entry in a validation report.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum DiffEntry {
    /// Address/slots accessed during execution but not in the declared list.
    Missing {
        address: Address,
        storage_keys: Vec<alloy_primitives::B256>,
        gas_waste: u64,
    },
    /// Address/slots in declared list but never accessed.
    Stale {
        address: Address,
        storage_keys: Vec<alloy_primitives::B256>,
        gas_waste: u64,
    },
    /// Address in both but declared has fewer slots than actual.
    Incomplete {
        address: Address,
        missing_slots: Vec<alloy_primitives::B256>,
        gas_waste: u64,
    },
    /// Address in declared list that is warm-by-default (tx.from, tx.to, coinbase, precompile).
    Redundant { address: Address, gas_waste: u64 },
    /// Same (address, slot) appears multiple times in declared list.
    Duplicate {
        address: Address,
        storage_key: alloy_primitives::B256,
        gas_waste: u64,
    },
}

impl DiffEntry {
    pub fn gas_waste(&self) -> u64 {
        match self {
            Self::Missing { gas_waste, .. }
            | Self::Stale { gas_waste, .. }
            | Self::Incomplete { gas_waste, .. }
            | Self::Redundant { gas_waste, .. }
            | Self::Duplicate { gas_waste, .. } => *gas_waste,
        }
    }
}

/// Gas cost summary for a validation report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasSummary {
    /// Gas cost of the declared access list.
    pub declared_list_cost: u64,
    /// Gas cost of the optimal access list.
    pub optimal_list_cost: u64,
    /// Estimated gas cost without any access list (all cold accesses).
    pub no_list_cost: u64,
    /// Waste per transaction: declared - optimal.
    pub waste_per_tx: i64,
    /// Savings vs no list: no_list - optimal.
    pub savings_vs_no_list: i64,
}

/// Optimized access list with metadata about what was removed.
#[derive(Debug, Clone)]
pub struct OptimizedAccessList {
    /// The final access list after optimization.
    pub list: AccessList,
    /// Addresses that were removed (warm-by-default).
    pub removed_addresses: Vec<Address>,
}

impl OptimizedAccessList {
    pub fn new(list: AccessList, removed_addresses: Vec<Address>) -> Self {
        Self {
            list,
            removed_addresses,
        }
    }
}

/// Full validation report comparing declared vs actual access list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    /// Individual diff entries (missing, stale, incomplete, redundant, duplicate).
    pub entries: Vec<DiffEntry>,
    /// Gas summary.
    pub gas_summary: GasSummary,
    /// The optimal access list (suggested fix).
    pub optimal_list: AccessList,
    /// Whether the declared list matches the optimal (no issues).
    pub is_valid: bool,
}

/// Raw result from the tracer before optimization.
#[derive(Debug, Clone)]
pub struct RawTraceResult {
    /// Raw access list from the inspector (before warm-address stripping).
    pub access_list: AccessList,
    /// Addresses of contracts created during execution (CREATE/CREATE2).
    pub created_contracts: Vec<Address>,
    /// Gas used during execution.
    pub gas_used: u64,
    /// Whether the transaction succeeded.
    pub success: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn test_diff_entry_gas_waste() {
        assert_eq!(
            DiffEntry::Missing {
                address: addr(1),
                storage_keys: vec![],
                gas_waste: 42
            }
            .gas_waste(),
            42
        );
        assert_eq!(
            DiffEntry::Stale {
                address: addr(1),
                storage_keys: vec![],
                gas_waste: 99
            }
            .gas_waste(),
            99
        );
        assert_eq!(
            DiffEntry::Incomplete {
                address: addr(1),
                missing_slots: vec![],
                gas_waste: 7
            }
            .gas_waste(),
            7
        );
        assert_eq!(
            DiffEntry::Redundant {
                address: addr(1),
                gas_waste: 2400
            }
            .gas_waste(),
            2400
        );
        assert_eq!(
            DiffEntry::Duplicate {
                address: addr(1),
                storage_key: slot(1),
                gas_waste: 1900
            }
            .gas_waste(),
            1900
        );
    }

    #[test]
    fn test_diff_entry_serde_tag() {
        let entry = DiffEntry::Missing {
            address: addr(1),
            storage_keys: vec![],
            gas_waste: 100,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains(r#""kind":"missing""#));

        let stale = DiffEntry::Stale {
            address: addr(1),
            storage_keys: vec![],
            gas_waste: 100,
        };
        let json = serde_json::to_string(&stale).unwrap();
        assert!(json.contains(r#""kind":"stale""#));
    }

    #[test]
    fn test_diff_entry_serde_all_variants() {
        let cases: &[(&str, DiffEntry)] = &[
            (
                "missing",
                DiffEntry::Missing {
                    address: addr(1),
                    storage_keys: vec![slot(1)],
                    gas_waste: 2000,
                },
            ),
            (
                "stale",
                DiffEntry::Stale {
                    address: addr(2),
                    storage_keys: vec![slot(2)],
                    gas_waste: 1900,
                },
            ),
            (
                "incomplete",
                DiffEntry::Incomplete {
                    address: addr(3),
                    missing_slots: vec![slot(3)],
                    gas_waste: 2000,
                },
            ),
            (
                "redundant",
                DiffEntry::Redundant {
                    address: addr(4),
                    gas_waste: 2400,
                },
            ),
            (
                "duplicate",
                DiffEntry::Duplicate {
                    address: addr(5),
                    storage_key: slot(5),
                    gas_waste: 1900,
                },
            ),
        ];

        for (expected_kind, entry) in cases {
            let json = serde_json::to_string(entry).unwrap();
            let expected_tag = format!(r#""kind":"{}""#, expected_kind);
            assert!(
                json.contains(&expected_tag),
                "variant {:?}: expected tag {}, got {}",
                expected_kind,
                expected_tag,
                json
            );
            // Roundtrip: deserializing must succeed and preserve the kind tag.
            let decoded: DiffEntry = serde_json::from_str(&json).unwrap();
            let json2 = serde_json::to_string(&decoded).unwrap();
            assert_eq!(
                json, json2,
                "serde roundtrip failed for kind {:?}",
                expected_kind
            );
        }
    }

    #[test]
    fn test_validation_report_serde_roundtrip() {
        let report = ValidationReport {
            entries: vec![DiffEntry::Redundant {
                address: addr(1),
                gas_waste: 2400,
            }],
            gas_summary: GasSummary {
                declared_list_cost: 5000,
                optimal_list_cost: 2400,
                no_list_cost: 4700,
                waste_per_tx: 2600,
                savings_vs_no_list: 2300,
            },
            optimal_list: AccessList(vec![AccessListItem {
                address: addr(2),
                storage_keys: vec![slot(1)],
            }]),
            is_valid: false,
        };
        let json = serde_json::to_string(&report).unwrap();
        let decoded: ValidationReport = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.is_valid, report.is_valid);
        assert_eq!(
            decoded.gas_summary.declared_list_cost,
            report.gas_summary.declared_list_cost
        );
        assert_eq!(decoded.entries.len(), 1);
    }

    #[test]
    fn test_optimized_access_list_new() {
        let list = AccessList(vec![AccessListItem {
            address: addr(5),
            storage_keys: vec![],
        }]);
        let removed = vec![addr(1), addr(2)];
        let opt = OptimizedAccessList::new(list.clone(), removed.clone());
        assert_eq!(opt.list.0.len(), 1);
        assert_eq!(opt.removed_addresses.len(), 2);
        assert!(opt.removed_addresses.contains(&addr(1)));
    }
}
