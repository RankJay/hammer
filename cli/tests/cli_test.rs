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

#[test]
fn test_compare_invalid_tx_hash() {
    cmd()
        .args([
            "compare",
            "--tx-hash",
            "not-a-hash",
            "--rpc-url",
            "http://127.0.0.1:1",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid tx hash"));
}

// Guards 1 (CREATE), 2 (blob), 3 (pre-Berlin block), and 4 (reverted) all require a live
// transaction from RPC and cannot be exercised in offline CLI tests. Their logic lives in
// pure helper functions in cli/src/commands/util.rs and is covered by unit tests there.

// --- validate additional argument/input error paths ---

#[test]
fn test_validate_missing_from_arg() {
    cmd()
        .args([
            "validate",
            "--to",
            "0x0000000000000000000000000000000000000001",
            "--access-list",
            "some_file.json",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--from"));
}

#[test]
fn test_validate_missing_to_arg() {
    cmd()
        .args([
            "validate",
            "--from",
            "0x0000000000000000000000000000000000000001",
            "--access-list",
            "some_file.json",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--to"));
}

#[test]
fn test_validate_invalid_to_address() {
    cmd()
        .args([
            "validate",
            "--from",
            "0x0000000000000000000000000000000000000001",
            "--to",
            "not-an-address",
            "--access-list",
            "some_file.json",
            "--rpc-url",
            "http://127.0.0.1:1",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid --to"));
}

#[test]
fn test_validate_missing_access_list_file() {
    // Nonexistent file → I/O error before any RPC call.
    cmd()
        .args([
            "validate",
            "--from",
            "0x0000000000000000000000000000000000000001",
            "--to",
            "0x0000000000000000000000000000000000000002",
            "--access-list",
            "nonexistent_file_12345.json",
            "--rpc-url",
            "http://127.0.0.1:1",
        ])
        .assert()
        .failure();
}

#[test]
fn test_validate_invalid_rpc_url() {
    // validate reads --access-list before parsing the RPC URL, so we need a real file
    // with valid JSON. Write a temp file with an empty access list.
    let tmp = std::env::temp_dir().join("hammer_test_empty_al.json");
    std::fs::write(&tmp, "[]").unwrap();

    cmd()
        .args([
            "validate",
            "--from",
            "0x0000000000000000000000000000000000000001",
            "--to",
            "0x0000000000000000000000000000000000000002",
            "--access-list",
            tmp.to_str().unwrap(),
            "--rpc-url",
            "not-a-url",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid RPC URL"));
}

// --- generate additional error paths ---

#[test]
fn test_generate_invalid_to_address() {
    cmd()
        .args([
            "generate",
            "--from",
            "0x0000000000000000000000000000000000000001",
            "--to",
            "not-an-address",
            "--rpc-url",
            "http://127.0.0.1:1",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid --to"));
}

#[test]
fn test_generate_invalid_value() {
    cmd()
        .args([
            "generate",
            "--from",
            "0x0000000000000000000000000000000000000001",
            "--to",
            "0x0000000000000000000000000000000000000002",
            "--value",
            "not-a-number",
            "--rpc-url",
            "http://127.0.0.1:1",
        ])
        .assert()
        .failure();
}

#[test]
fn test_generate_invalid_output_format() {
    cmd()
        .args([
            "generate",
            "--from",
            "0x0000000000000000000000000000000000000001",
            "--to",
            "0x0000000000000000000000000000000000000002",
            "--output",
            "xml",
            "--rpc-url",
            "http://127.0.0.1:1",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("output"));
}

// --- compare additional error paths ---

#[test]
fn test_compare_invalid_rpc_url() {
    cmd()
        .args([
            "compare",
            "--tx-hash",
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--rpc-url",
            "not-a-url",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid RPC URL"));
}

/// A valid tx hash against a non-responsive RPC (port 1) must fail gracefully with a
/// user-readable transport/network error — not a panic or an internal assertion failure.
/// This verifies that error propagation from the RPC layer surfaces cleanly.
#[test]
fn test_compare_rpc_network_failure_is_user_friendly() {
    cmd()
        .args([
            "compare",
            "--tx-hash",
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--rpc-url",
            "http://127.0.0.1:1",
        ])
        .assert()
        .failure()
        // Must not panic — stderr must contain some error text, not be empty.
        .stderr(predicate::str::is_empty().not());
}

// --- validate: invalid JSON in access list file ---

/// validate.rs parses the access list file before any RPC call.
/// Malformed JSON must fail with an error message containing "invalid access list".
#[test]
fn test_validate_invalid_json_in_access_list_file() {
    let tmp = std::env::temp_dir().join("hammer_test_bad_al.json");
    std::fs::write(&tmp, "{not valid json}").unwrap();

    cmd()
        .args([
            "validate",
            "--from",
            "0x0000000000000000000000000000000000000001",
            "--to",
            "0x0000000000000000000000000000000000000002",
            "--access-list",
            tmp.to_str().unwrap(),
            "--rpc-url",
            "http://127.0.0.1:1",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid access list"));
}

// --- generate and validate: RPC network failures are user-friendly ---

/// generate makes a network call (block fetch) after argument parsing.
/// A non-responsive RPC must fail gracefully with non-empty stderr.
#[test]
fn test_generate_rpc_network_failure_is_user_friendly() {
    cmd()
        .args([
            "generate",
            "--from",
            "0x0000000000000000000000000000000000000001",
            "--to",
            "0x0000000000000000000000000000000000000002",
            "--rpc-url",
            "http://127.0.0.1:1",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::is_empty().not());
}

/// validate makes a network call (block fetch) after local argument parsing.
/// A non-responsive RPC must fail gracefully with non-empty stderr.
#[test]
fn test_validate_rpc_network_failure_is_user_friendly() {
    let tmp = std::env::temp_dir().join("hammer_test_empty_al_rpc.json");
    std::fs::write(&tmp, "[]").unwrap();

    cmd()
        .args([
            "validate",
            "--from",
            "0x0000000000000000000000000000000000000001",
            "--to",
            "0x0000000000000000000000000000000000000002",
            "--access-list",
            tmp.to_str().unwrap(),
            "--rpc-url",
            "http://127.0.0.1:1",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::is_empty().not());
}
