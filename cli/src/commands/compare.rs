use alloy_eips::BlockId;
use alloy_primitives::{Address, U256};
use alloy_provider::Provider;
use alloy_rpc_types_eth::{TransactionRequest, TransactionTrait};
use clap::Args;
use eyre::{Context, Result};
use hammer_core::validate_replay;
use reqwest::Url;
use revm::context::{BlockEnv, TxEnv};
use revm::primitives::TxKind;

use super::util::{assert_not_blob, assert_not_create, assert_post_berlin};

#[derive(Args)]
pub struct CompareArgs {
    #[arg(long, default_value = "https://eth.llamarpc.com")]
    pub rpc_url: String,
    #[arg(long)]
    pub tx_hash: String,
}

/// Run the compare command.
///
/// # Test boundary
///
/// This function requires a live RPC connection and cannot be unit tested in isolation.
/// Its guard logic (`assert_not_create`, `assert_not_blob`, `assert_post_berlin`) is
/// covered by unit tests in `cli::commands::util`. The diffing and report formatting
/// delegates entirely to `validate_replay()` + `ValidationReport`, which are covered
/// exhaustively in `hammer_core::validator` tests. End-to-end behaviour is verified
/// by the CLI integration tests in `cli/tests/cli_test.rs` (error-path only, no RPC).
pub async fn run(args: CompareArgs) -> Result<()> {
    let tx_hash = args.tx_hash.parse().wrap_err("invalid tx hash")?;

    let url = Url::parse(&args.rpc_url).wrap_err("invalid RPC URL")?;
    let provider = alloy_provider::ProviderBuilder::new()
        .disable_recommended_fillers()
        .connect_http(url)
        .erased();

    // Fetch tx and receipt in parallel — both need only the tx hash.
    let (tx, receipt) = tokio::try_join!(
        async {
            provider
                .get_transaction_by_hash(tx_hash)
                .await?
                .ok_or_else(|| eyre::eyre!("Transaction not found"))
        },
        async {
            provider
                .get_transaction_receipt(tx_hash)
                .await?
                .ok_or_else(|| eyre::eyre!("Receipt not found"))
        },
    )?;

    // Guard 1: Reject contract creation transactions
    assert_not_create(tx.inner.to())?;

    // Guard 2: Reject blob transactions (EIP-4844, Type 3)
    assert_not_blob(tx.inner.blob_versioned_hashes())?;

    // Guard 4: Reject reverted transactions
    if !receipt.status() {
        eyre::bail!("transaction reverted on-chain — access list comparison is not meaningful for failed transactions");
    }

    let block_hash = tx
        .block_hash
        .ok_or_else(|| eyre::eyre!("Transaction not mined"))?;
    let block = provider
        .get_block_by_hash(block_hash)
        .await?
        .ok_or_else(|| eyre::eyre!("Block not found"))?;

    let header = &block.header;
    // Guard 3: Reject pre-Berlin blocks
    assert_post_berlin(header.number)?;
    let block_env = BlockEnv {
        number: U256::from(header.number),
        beneficiary: header.beneficiary,
        timestamp: U256::from(header.timestamp),
        gas_limit: header.gas_limit,
        basefee: header.base_fee_per_gas.unwrap_or(0),
        difficulty: header.difficulty,
        prevrandao: Some(header.mix_hash),
        blob_excess_gas_and_price: header.excess_blob_gas.map(|excess| {
            revm::context_interface::block::BlobExcessGasAndPrice::new(
                excess,
                revm::primitives::eip4844::BLOB_BASE_FEE_UPDATE_FRACTION_PRAGUE,
            )
        }),
    };

    let from = tx.inner.signer();
    let to = tx.inner.to().unwrap_or(Address::ZERO);
    let value = tx.inner.value();
    let data = tx.inner.input().clone();
    let declared = tx
        .inner
        .access_list()
        .cloned()
        .unwrap_or_else(|| alloy_rpc_types_eth::AccessList::default());

    let basefee = block_env.basefee as u128;
    let gas_price = tx.inner.max_fee_per_gas().max(basefee);
    let mut builder = TxEnv::builder()
        .caller(from)
        .nonce(tx.inner.nonce())
        .kind(TxKind::Call(to))
        .gas_limit(tx.inner.gas_limit())
        .gas_price(gas_price)
        .value(value)
        .data(data.clone());

    if let Some(priority) = tx.inner.max_priority_fee_per_gas() {
        builder = builder.gas_priority_fee(Some(priority));
    }

    let tx_env = builder.build().unwrap();

    // Build a TransactionRequest for the prefetch hint (eth_createAccessList).
    let tx_req = TransactionRequest {
        from: Some(from),
        to: Some(TxKind::Call(to)),
        value: Some(value),
        input: alloy_rpc_types_eth::TransactionInput::new(data),
        gas: Some(tx.inner.gas_limit()),
        ..Default::default()
    };

    // Pre-warm the database: fetch all storage/account state in parallel before
    // revm runs, eliminating sequential AlloyDB RPC calls during EVM execution.
    let state_block_id = BlockId::hash(block_hash);
    let db = super::prefetch::build(
        provider,
        state_block_id,
        state_block_id,
        tx_req,
        &declared,
    )
    .await
    .wrap_err("prefetch failed")?;

    let report = validate_replay(db, tx_env, block_env, declared).wrap_err("validation failed")?;

    let s = &report.gas_summary;
    let sign = if s.waste_per_tx >= 0 { "+" } else { "-" };
    println!(
        "List cost:  {} gas declared  →  {} gas optimal  ({}{}  upfront)",
        s.declared_list_cost,
        s.optimal_list_cost,
        sign,
        s.waste_per_tx.unsigned_abs(),
    );

    let execution_penalty: u64 = report
        .entries
        .iter()
        .filter(|e| {
            matches!(
                e,
                hammer_core::types::DiffEntry::Missing { .. }
                    | hammer_core::types::DiffEntry::Incomplete { .. }
            )
        })
        .map(|e| e.gas_waste())
        .sum();
    if execution_penalty > 0 {
        let missing_count = report
            .entries
            .iter()
            .filter(|e| matches!(e, hammer_core::types::DiffEntry::Missing { .. }))
            .count();
        let incomplete_count = report
            .entries
            .iter()
            .filter(|e| matches!(e, hammer_core::types::DiffEntry::Incomplete { .. }))
            .count();
        println!(
            "Execution:  {} missing / {} incomplete  →  +{} gas at runtime",
            missing_count, incomplete_count, execution_penalty,
        );
    }

    if !report.is_valid {
        println!("Issues: {} entries", report.entries.len());
        for e in &report.entries {
            println!("  {:?}", e);
        }
    }
    Ok(())
}
