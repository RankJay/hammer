//! Warm-by-default address sets per EIP-2929 and EIP-3651.

use alloy_primitives::Address;
use std::collections::BTreeSet;

/// Precompile addresses 0x01..0x0a are always warm (EIP-2929).
pub fn precompile_addresses() -> BTreeSet<Address> {
    (1..=10u8)
        .map(|i| Address::from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, i]))
        .collect()
}
