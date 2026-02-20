use alloy_eips::BlockId;
use alloy_primitives::{Address, U256};
use alloy_provider::Provider;
use alloy_rpc_types_eth::TransactionTrait;
use clap::Args;
use eyre::{Context, Result};
use hammer_core::validate_replay;
use reqwest::Url;
use revm::context::{BlockEnv, TxEnv};
use revm::primitives::TxKind;

#[derive(Args)]
pub struct CompareArgs {
    #[arg(long, default_value = "https://eth.llamarpc.com")]
    pub rpc_url: String,
    #[arg(long)]
    pub tx_hash: String,
}

pub async fn run(args: CompareArgs) -> Result<()> {
    let tx_hash = args.tx_hash.parse().wrap_err("invalid tx hash")?;

    let url = Url::parse(&args.rpc_url).wrap_err("invalid RPC URL")?;
    let provider = alloy_provider::ProviderBuilder::new()
        .disable_recommended_fillers()
        .connect_http(url)
        .erased();

    let tx = provider
        .get_transaction_by_hash(tx_hash)
        .await?
        .ok_or_else(|| eyre::eyre!("Transaction not found"))?;

    let block_hash = tx
        .block_hash
        .ok_or_else(|| eyre::eyre!("Transaction not mined"))?;
    let block = provider
        .get_block_by_hash(block_hash)
        .await?
        .ok_or_else(|| eyre::eyre!("Block not found"))?;

    let header = &block.header;
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
        .data(data);

    if let Some(priority) = tx.inner.max_priority_fee_per_gas() {
        builder = builder.gas_priority_fee(Some(priority));
    }

    let tx_env = builder.build().unwrap();

    // Use block state; nonce check is disabled for replay.
    let state_block_id = BlockId::hash(block_hash);
    let alloy_db = revm::database::AlloyDB::new(provider, state_block_id);
    let async_db = revm::database_interface::WrapDatabaseAsync::new(alloy_db)
        .ok_or_else(|| eyre::eyre!("WrapDatabaseAsync requires tokio runtime"))?;
    let db = revm::database_interface::WrapDatabaseRef::from(async_db);

    let report = validate_replay(db, tx_env, block_env, declared).wrap_err("validation failed")?;

    let total_issue_waste: u64 = report.entries.iter().map(|e| e.gas_waste()).sum();
    let pct = if report.gas_summary.optimal_list_cost > 0 || total_issue_waste > 0 {
        let opt = report.gas_summary.optimal_list_cost as f64;
        let waste = total_issue_waste as f64;
        100.0 * opt / (opt + waste)
    } else {
        100.0
    };
    println!("Access list optimality: {:.1}%", pct.clamp(0.0, 100.0));
    if !report.is_valid {
        println!("Issues: {} entries", report.entries.len());
        for e in &report.entries {
            println!("  {:?}", e);
        }
    }
    Ok(())
}
