// End-to-end CLI tests that exercise the binary's argument parsing and error paths.
// These tests do NOT make any RPC calls — they only verify that the CLI rejects
// invalid or missing arguments with the correct exit code and error message.

use assert_cmd::Command;
use predicates::prelude::*;

#[allow(deprecated)]
fn cmd() -> Command {
    Command::cargo_bin("hammer").unwrap()
}

// --- generate subcommand ---

#[test]
fn test_generate_missing_from_arg() {
    cmd()
        .args([
            "generate",
            "--to",
            "0x0000000000000000000000000000000000000001",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--from"));
}

#[test]
fn test_generate_missing_to_arg() {
    cmd()
        .args([
            "generate",
            "--from",
            "0x0000000000000000000000000000000000000001",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--to"));
}

#[test]
fn test_generate_invalid_from_address() {
    cmd()
        .args([
            "generate",
            "--from",
            "not-an-address",
            "--to",
            "0x0000000000000000000000000000000000000001",
            "--rpc-url",
            "http://127.0.0.1:1",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid --from"));
}

#[test]
fn test_generate_invalid_hex_data() {
    cmd()
        .args([
            "generate",
            "--from",
            "0x0000000000000000000000000000000000000001",
            "--to",
            "0x0000000000000000000000000000000000000002",
            "--data",
            "0xZZZZ",
            "--rpc-url",
            "http://127.0.0.1:1",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid hex data"));
}

#[test]
fn test_generate_invalid_rpc_url() {
    cmd()
        .args([
            "generate",
            "--from",
            "0x0000000000000000000000000000000000000001",
            "--to",
            "0x0000000000000000000000000000000000000002",
            "--rpc-url",
            "not-a-url",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid RPC URL"));
}

// --- validate subcommand ---

#[test]
fn test_validate_missing_access_list_arg() {
    cmd()
        .args([
            "validate",
            "--from",
            "0x0000000000000000000000000000000000000001",
            "--to",
            "0x0000000000000000000000000000000000000002",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--access-list"));
}

// --- compare subcommand ---

#[test]
fn test_compare_missing_tx_hash_arg() {
    cmd()
        .args(["compare"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--tx-hash"));
}
