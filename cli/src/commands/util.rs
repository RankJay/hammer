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

#[cfg(test)]
mod tests {
    use super::*;

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
