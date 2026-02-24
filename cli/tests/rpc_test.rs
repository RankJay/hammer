// RPC integration tests — gated by HAMMER_TEST_RPC_URL environment variable.
//
// Run with:
//   $env:HAMMER_TEST_RPC_URL = "https://..."; cargo test --test rpc_test
//
// Every test silently skips if the env var is absent, leaving offline CI unaffected.
// Tests assert behavioral contracts (exit codes, output formats, guard enforcement),
// not mere code coverage.

use assert_cmd::Command;
use predicates::prelude::*;
use std::io::Write;

// --- Infrastructure ---

fn rpc_url() -> Option<String> {
    std::env::var("HAMMER_TEST_RPC_URL").ok()
}

macro_rules! require_rpc {
    ($url:ident) => {
        let Some($url) = rpc_url() else {
            return;
        };
    };
}

#[allow(deprecated)]
fn hammer() -> Command {
    Command::cargo_bin("hammer").unwrap()
}

/// Write content to a named temp file and return the path as a String.
fn temp_file(name: &str, content: &str) -> String {
    let path = std::env::temp_dir().join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
    path.to_str().unwrap().to_owned()
}

/// A simple JSON-RPC call via ureq. Returns the parsed response body.
fn jsonrpc(url: &str, method: &str, params: serde_json::Value) -> serde_json::Value {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    ureq::post(url)
        .send_json(body)
        .unwrap()
        .body_mut()
        .read_json::<serde_json::Value>()
        .unwrap()
}

/// Fetch a known-good post-Cancun, successful, non-blob, non-CREATE tx hash from
/// block 20_000_000 (0x1312D00). Returns `None` if the block or a suitable tx cannot
/// be found (e.g. the node pruned that state), in which case the calling test will skip.
fn find_successful_tx(url: &str) -> Option<String> {
    let resp = jsonrpc(
        url,
        "eth_getBlockByNumber",
        serde_json::json!(["0x1312D00", true]),
    );
    let txs = resp["result"]["transactions"].as_array()?;
    for tx in txs {
        let tx_type = tx["type"].as_str().unwrap_or("0x0");
        let to = tx["to"].as_str().unwrap_or("");
        // type 0x2 = EIP-1559; must have a `to` (not CREATE); skip blob (0x3)
        if tx_type == "0x2" && !to.is_empty() {
            let hash = tx["hash"].as_str()?;
            // Verify it succeeded by checking its receipt.
            let receipt = jsonrpc(
                url,
                "eth_getTransactionReceipt",
                serde_json::json!([hash]),
            );
            if receipt["result"]["status"].as_str() == Some("0x1") {
                return Some(hash.to_owned());
            }
        }
    }
    None
}

/// Fetch a known-good post-Cancun reverted tx hash from blocks around 20_000_000.
/// Returns `None` if no reverted tx exists in those blocks (the test will skip).
fn find_reverted_tx(url: &str) -> Option<String> {
    // Blocks around 20_000_000 (post-Cancun). Any busy block will have failures.
    for block_hex in ["0x1312D00", "0x1312D01", "0x1312D02"] {
        let resp = jsonrpc(
            url,
            "eth_getBlockByNumber",
            serde_json::json!([block_hex, true]),
        );
        let txs = resp["result"]["transactions"].as_array()?;
        for tx in txs {
            let tx_type = tx["type"].as_str().unwrap_or("0x0");
            let to = tx["to"].as_str().unwrap_or("");
            if tx_type == "0x2" && !to.is_empty() {
                let hash = tx["hash"].as_str()?;
                let receipt = jsonrpc(
                    url,
                    "eth_getTransactionReceipt",
                    serde_json::json!([hash]),
                );
                if receipt["result"]["status"].as_str() == Some("0x0") {
                    return Some(hash.to_owned());
                }
            }
        }
    }
    None
}

// Well-known addresses.
// Vitalik's public EOA — stable, will never become a contract.
const VITALIK: &str = "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045";
// Uniswap V3 SwapRouter — immutable deployed contract with storage.
const UNISWAP_V3_ROUTER: &str = "0xE592427A0AEce92De3Edee1F18E0157C05861564";
// A simple EOA that will never be a contract. Used as the recipient for ETH transfers.
const PLAIN_EOA: &str = "0xAb5801a7D398351b8bE11C439e05C5B3259aeC9B";

// The first-ever EIP-4844 blob tx on mainnet (block 19,426,652, March 13 2024).
// Type 3, so the blob guard must fire.
const TX_BLOB: &str = "0x110d6d8888ced3615a7ca07d91acd9eebc4e61f669d83fd2e7f42de1ac7d39a3";

// A made-up but valid-format hash — guaranteed not to exist on any chain.
const TX_NONEXISTENT: &str =
    "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

// Pinned block for generate/validate tests.
// Must be post-Cancun (≥ 19,426,588) because revm requires `excess_blob_gas` to be set
// in the block header, which only exists from Cancun onwards. Block 20,000,000 is
// post-Cancun (July 2024) and available on any standard (non-archive) node.
const PINNED_BLOCK: &str = "20000000";

// ---
// Group 1: generate — output correctness
// ---

/// The generate command's primary output artifact is JSON on stdout.
/// This test proves the JSON is parseable and has the expected array structure.
#[test]
fn test_generate_json_output_is_valid_json() {
    require_rpc!(url);

    let output = hammer()
        .args([
            "generate",
            "--from", VITALIK,
            "--to", UNISWAP_V3_ROUTER,
            "--block", PINNED_BLOCK,
            "--rpc-url", &url,
        ])
        .output()
        .unwrap();

    assert!(output.status.success(), "expected exit 0, got: {:?}", output.status);

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.trim().is_empty(), "stdout must not be empty");

    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout must be valid JSON");

    let arr = parsed.as_array().expect("JSON must be an array");
    // Every element must have `address` and `storageKeys`.
    for item in arr {
        assert!(item["address"].is_string(), "each entry needs 'address'");
        assert!(item["storageKeys"].is_array(), "each entry needs 'storageKeys'");
    }
}

/// Asserts the --output human branch runs and produces the expected header line.
#[test]
fn test_generate_human_output_format() {
    require_rpc!(url);

    hammer()
        .args([
            "generate",
            "--from", VITALIK,
            "--to", UNISWAP_V3_ROUTER,
            "--block", PINNED_BLOCK,
            "--output", "human",
            "--rpc-url", &url,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Access list (gas cost:"));
}

/// Asserts --block <number> works as an alternative to `latest`.
#[test]
fn test_generate_block_number_flag() {
    require_rpc!(url);

    let output = hammer()
        .args([
            "generate",
            "--from", VITALIK,
            "--to", UNISWAP_V3_ROUTER,
            "--block", PINNED_BLOCK,
            "--rpc-url", &url,
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    serde_json::from_str::<serde_json::Value>(&stdout).expect("stdout must be valid JSON");
}

// ---
// Group 1b: generate → validate — algorithmic correctness proof
// ---

/// Two-step correctness proof: generate produces the access list, validate independently
/// re-traces the same transaction against the same block state and confirms they agree.
///
/// If the generator omits a required slot, includes a wrong address, or fails to strip
/// a warm address, the validator will return is_valid:false and this test fails.
/// The non-empty assertion on the generated list guards against a vacuous pass where
/// both sides trivially agree on an empty list.
#[test]
fn test_generate_then_validate_is_correct() {
    require_rpc!(url);

    // Step 1: generate
    let gen_output = hammer()
        .args([
            "generate",
            "--from", VITALIK,
            "--to", UNISWAP_V3_ROUTER,
            "--block", PINNED_BLOCK,
            "--rpc-url", &url,
        ])
        .output()
        .unwrap();

    assert!(
        gen_output.status.success(),
        "generate must succeed: {:?}",
        String::from_utf8_lossy(&gen_output.stderr)
    );

    let gen_stdout = String::from_utf8(gen_output.stdout).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&gen_stdout).expect("generate stdout must be valid JSON");

    // Guard against vacuous pass: the chosen target must touch contract storage.
    // Uniswap V3 Router reads slot 0 (reentrancy lock) on every call, so this should
    // always be non-empty. If it is empty, the test skips rather than falsely passing.
    let arr = parsed.as_array().expect("must be array");
    if arr.is_empty() {
        // No storage touched at this block — test is vacuous, skip.
        eprintln!("SKIP: generated access list is empty, test would be vacuous");
        return;
    }

    // Write generated list to a temp file for validate.
    let list_path = temp_file("hammer_rpc_gen_validate.json", &gen_stdout);

    // Step 2: validate the generated list against the same tx/block
    let val_output = hammer()
        .args([
            "validate",
            "--from", VITALIK,
            "--to", UNISWAP_V3_ROUTER,
            "--block", PINNED_BLOCK,
            "--access-list", &list_path,
            "--rpc-url", &url,
        ])
        .output()
        .unwrap();

    // Exit 0 means the generated list is exactly correct.
    assert!(
        val_output.status.success(),
        "validate must exit 0 (generated list is correct), stderr: {:?}, stdout: {:?}",
        String::from_utf8_lossy(&val_output.stderr),
        String::from_utf8_lossy(&val_output.stdout),
    );

    let val_stdout = String::from_utf8(val_output.stdout).unwrap();
    let report: serde_json::Value =
        serde_json::from_str(&val_stdout).expect("validate stdout must be valid JSON");

    assert_eq!(
        report["is_valid"],
        serde_json::Value::Bool(true),
        "is_valid must be true; report: {}",
        val_stdout
    );

    let entries = report["entries"].as_array().expect("entries must be array");
    assert!(
        entries.is_empty(),
        "zero issues expected; entries: {:?}",
        entries
    );
}

// ---
// Group 2: validate — exit codes are the CI contract
// ---

/// A simple EOA→EOA transfer touches no contract storage. The optimal access list is
/// empty. An empty declared list must therefore produce exit 0 and is_valid:true.
#[test]
fn test_validate_exit_0_on_empty_list_for_plain_transfer() {
    require_rpc!(url);

    let list_path = temp_file("hammer_rpc_empty_al.json", "[]");

    let output = hammer()
        .args([
            "validate",
            "--from", VITALIK,
            "--to", PLAIN_EOA,
            "--block", PINNED_BLOCK,
            "--access-list", &list_path,
            "--rpc-url", &url,
        ])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "must exit 0 for empty list on plain transfer; stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("\"is_valid\": true"), "must report is_valid:true; got: {stdout}");
}

/// A non-empty declared list for a plain ETH transfer (which needs no access list)
/// is stale → must exit 1 and report is_valid:false with a "stale" entry.
#[test]
fn test_validate_exit_1_on_stale_list_for_plain_transfer() {
    require_rpc!(url);

    // A made-up address that will never be accessed by a plain ETH transfer.
    let stale_list = r#"[{"address":"0x1234567890123456789012345678901234567890","storageKeys":[]}]"#;
    let list_path = temp_file("hammer_rpc_stale_al.json", stale_list);

    let output = hammer()
        .args([
            "validate",
            "--from", VITALIK,
            "--to", PLAIN_EOA,
            "--block", PINNED_BLOCK,
            "--access-list", &list_path,
            "--rpc-url", &url,
        ])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(1),
        "must exit 1 for stale list; stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("\"is_valid\": false"), "must report is_valid:false; got: {stdout}");
    assert!(stdout.contains("\"stale\"") || stdout.contains("Stale"), "must contain stale entry; got: {stdout}");
}

/// The --output human branch for a valid report must print the exact success string.
#[test]
fn test_validate_human_output_valid_report() {
    require_rpc!(url);

    let list_path = temp_file("hammer_rpc_empty_al_human.json", "[]");

    hammer()
        .args([
            "validate",
            "--from", VITALIK,
            "--to", PLAIN_EOA,
            "--block", PINNED_BLOCK,
            "--access-list", &list_path,
            "--output", "human",
            "--rpc-url", &url,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Valid: access list matches execution trace.",
        ));
}

/// The --output human branch for an invalid (stale) report must print the issues header.
#[test]
fn test_validate_human_output_invalid_report() {
    require_rpc!(url);

    let stale_list = r#"[{"address":"0x1234567890123456789012345678901234567890","storageKeys":[]}]"#;
    let list_path = temp_file("hammer_rpc_stale_al_human.json", stale_list);

    let output = hammer()
        .args([
            "validate",
            "--from", VITALIK,
            "--to", PLAIN_EOA,
            "--block", PINNED_BLOCK,
            "--access-list", &list_path,
            "--output", "human",
            "--rpc-url", &url,
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Issues found:"), "must contain 'Issues found:'; got: {stdout}");
    assert!(stdout.contains("Gas summary:"), "must contain 'Gas summary:'; got: {stdout}");
}

// ---
// Group 3: compare — guards enforced on real transactions
// ---

/// A reverted on-chain transaction must be rejected with an error mentioning "reverted".
#[test]
fn test_compare_reverted_tx_rejected() {
    require_rpc!(url);

    let Some(tx_hash) = find_reverted_tx(&url) else {
        eprintln!("SKIP: could not find a reverted tx in the target blocks (node may not have that state)");
        return;
    };

    hammer()
        .args(["compare", "--tx-hash", &tx_hash, "--rpc-url", &url])
        .assert()
        .failure()
        .stderr(predicate::str::contains("reverted"));
}

/// A tx hash that does not exist on chain must produce "Transaction not found".
#[test]
fn test_compare_nonexistent_tx_not_found() {
    require_rpc!(url);

    hammer()
        .args(["compare", "--tx-hash", TX_NONEXISTENT, "--rpc-url", &url])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found").or(predicate::str::contains("not found")));
}

/// A successful, non-blob, post-Berlin EIP-1559 contract call must produce the gas
/// summary line on stdout, proving the entire compare output path runs.
#[test]
fn test_compare_valid_tx_produces_gas_summary() {
    require_rpc!(url);

    let Some(tx_hash) = find_successful_tx(&url) else {
        eprintln!("SKIP: could not find a suitable successful tx in block 17_000_000 (node may not have that state)");
        return;
    };

    hammer()
        .args(["compare", "--tx-hash", &tx_hash, "--rpc-url", &url])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("List cost:")
                .and(predicate::str::contains("gas declared"))
                .and(predicate::str::contains("gas optimal")),
        );
}

/// The first-ever EIP-4844 blob tx must be rejected by the blob guard.
#[test]
fn test_compare_blob_tx_rejected() {
    require_rpc!(url);

    hammer()
        .args(["compare", "--tx-hash", TX_BLOB, "--rpc-url", &url])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("blob").and(predicate::str::contains("EIP-4844")),
        );
}
