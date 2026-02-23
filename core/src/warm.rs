//! Warm-by-default address sets per EIP-2929 and EIP-3651.

use alloy_primitives::Address;
use std::collections::BTreeSet;

/// Precompile addresses 0x01..0x0a are always warm (EIP-2929).
pub fn precompile_addresses() -> BTreeSet<Address> {
    (1..=10u8)
        .map(|i| Address::from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, i]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(n: u8) -> Address {
        Address::from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, n])
    }

    #[test]
    fn test_precompile_addresses_count() {
        assert_eq!(precompile_addresses().len(), 10);
    }

    #[test]
    fn test_precompile_addresses_exact_range() {
        let set = precompile_addresses();
        for i in 1u8..=10 {
            assert!(
                set.contains(&addr(i)),
                "0x{:02x} must be in precompile set",
                i
            );
        }
        assert!(
            !set.contains(&addr(0)),
            "0x00 must not be in precompile set"
        );
        assert!(
            !set.contains(&addr(11)),
            "0x0b must not be in precompile set"
        );
    }
}
