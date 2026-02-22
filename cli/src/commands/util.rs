use alloy_eips::BlockId;
use alloy_primitives::U256;
use eyre::{Context, Result};

pub fn parse_block_id(s: &str) -> Result<BlockId> {
    if s.eq_ignore_ascii_case("latest") {
        Ok(BlockId::latest())
    } else if s.eq_ignore_ascii_case("pending") {
        Ok(BlockId::pending())
    } else if let Ok(n) = s.parse::<u64>() {
        Ok(BlockId::number(n))
    } else {
        eyre::bail!("invalid block: expected 'latest', 'pending', or block number")
    }
}

pub fn parse_u256(s: &str) -> Result<U256> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        U256::from_str_radix(hex, 16).wrap_err("invalid hex number")
    } else {
        U256::from_str_radix(s, 10).wrap_err("invalid number")
    }
}

pub fn parse_hex_bytes(s: &str) -> Result<Vec<u8>> {
    let s = s.trim_start_matches("0x");
    if s.is_empty() {
        return Ok(vec![]);
    }
    hex::decode(s).wrap_err("invalid hex data")
}

/// Assert that the block number is post-Berlin fork (where EIP-2930 access lists exist).
///
/// Berlin fork activated at block 12,244,000 on mainnet.
pub fn assert_post_berlin(block_number: u64) -> Result<()> {
    const BERLIN_BLOCK: u64 = 12_244_000;
    if block_number < BERLIN_BLOCK {
        eyre::bail!(
            "access lists (EIP-2930) do not exist before the Berlin fork (block {}), \
             target block is {}",
            BERLIN_BLOCK,
            block_number
        );
    }
    Ok(())
}

/// Reject contract creation transactions (CREATE/CREATE2).
///
/// `to` is `None` for creation transactions; access list analysis requires a call target.
pub fn assert_not_create(to: Option<alloy_primitives::Address>) -> Result<()> {
    if to.is_none() {
        eyre::bail!(
            "contract creation transactions (CREATE/CREATE2) are not supported \
             — access list analysis requires a call target"
        );
    }
    Ok(())
}

/// Reject blob transactions (EIP-4844, Type 3).
///
/// Blob data (versioned hashes, KZG commitments/proofs) is not replayed, making
/// access list comparison meaningless for these transactions.
pub fn assert_not_blob(blob_hashes: Option<&[alloy_primitives::B256]>) -> Result<()> {
    if blob_hashes.map_or(false, |h| !h.is_empty()) {
        eyre::bail!(
            "blob transactions (EIP-4844, Type 3) are not supported \
             — blob data is not replayed"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, B256};

    // --- assert_post_berlin ---

    #[test]
    fn test_assert_post_berlin_at_berlin_block() {
        assert!(assert_post_berlin(12_244_000).is_ok());
    }

    #[test]
    fn test_assert_post_berlin_after_berlin() {
        assert!(assert_post_berlin(18_000_000).is_ok());
    }

    #[test]
    fn test_assert_post_berlin_at_zero() {
        let err = assert_post_berlin(0).unwrap_err();
        assert!(err.to_string().contains("Berlin"));
        assert!(err.to_string().contains("12244000"));
    }

    #[test]
    fn test_assert_post_berlin_one_before() {
        let err = assert_post_berlin(12_243_999).unwrap_err();
        assert!(err.to_string().contains("Berlin"));
        assert!(err.to_string().contains("12243999"));
    }

    // --- assert_not_create ---

    #[test]
    fn test_assert_not_create_with_call_target() {
        let addr = Address::from_slice(&[0u8; 20]);
        assert!(assert_not_create(Some(addr)).is_ok());
    }

    #[test]
    fn test_assert_not_create_with_none() {
        let err = assert_not_create(None).unwrap_err();
        assert!(err.to_string().contains("CREATE"));
    }

    // --- assert_not_blob ---

    #[test]
    fn test_assert_not_blob_with_empty_hashes() {
        assert!(assert_not_blob(Some(&[])).is_ok());
    }

    #[test]
    fn test_assert_not_blob_with_none() {
        assert!(assert_not_blob(None).is_ok());
    }

    #[test]
    fn test_assert_not_blob_with_hashes() {
        let hash = B256::ZERO;
        let err = assert_not_blob(Some(&[hash])).unwrap_err();
        assert!(err.to_string().contains("blob"));
        assert!(err.to_string().contains("EIP-4844"));
    }

    // --- parse_block_id ---

    #[test]
    fn test_parse_block_id_latest() {
        let id = parse_block_id("latest").unwrap();
        assert_eq!(id, BlockId::latest());
    }

    #[test]
    fn test_parse_block_id_latest_case_insensitive() {
        let id = parse_block_id("LATEST").unwrap();
        assert_eq!(id, BlockId::latest());
    }

    #[test]
    fn test_parse_block_id_pending() {
        let id = parse_block_id("pending").unwrap();
        assert_eq!(id, BlockId::pending());
    }

    #[test]
    fn test_parse_block_id_number() {
        let id = parse_block_id("12345").unwrap();
        assert_eq!(id, BlockId::number(12345));
    }

    #[test]
    fn test_parse_block_id_invalid() {
        assert!(parse_block_id("abc").is_err());
    }

    #[test]
    fn test_parse_u256_decimal() {
        let v = parse_u256("100").unwrap();
        assert_eq!(v, U256::from(100u64));
    }

    #[test]
    fn test_parse_u256_hex() {
        let v = parse_u256("0xff").unwrap();
        assert_eq!(v, U256::from(255u64));
    }

    #[test]
    fn test_parse_hex_bytes_empty() {
        assert_eq!(parse_hex_bytes("0x").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn test_parse_hex_bytes_valid() {
        assert_eq!(
            parse_hex_bytes("0xdeadbeef").unwrap(),
            vec![0xde, 0xad, 0xbe, 0xef]
        );
    }

    #[test]
    fn test_parse_hex_bytes_invalid() {
        assert!(parse_hex_bytes("0xgg").is_err());
    }
}
