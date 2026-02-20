use alloy_primitives::U256;
use alloy_provider::Provider;
use alloy_rpc_types_eth::AccessList;
use clap::Args;
use eyre::{Context, Result};
use hammer_core::validate;
use reqwest::Url;
use revm::context::{BlockEnv, TxEnv};
use revm::primitives::TxKind;
use std::path::PathBuf;

use super::util::{parse_block_id, parse_hex_bytes, parse_u256};

#[derive(Args)]
pub struct ValidateArgs {
    #[arg(long, default_value = "https://eth.llamarpc.com")]
    pub rpc_url: String,
    #[arg(long)]
    pub from: String,
    #[arg(long)]
    pub to: String,
    #[arg(long, default_value = "0x")]
    pub data: String,
    #[arg(long, default_value = "0")]
    pub value: String,
    #[arg(long)]
    pub access_list: PathBuf,
    #[arg(long, default_value = "latest")]
    pub block: String,
    #[arg(long, default_value = "json", value_parser = ["json", "human"])]
    pub output: String,
}

pub async fn run(args: ValidateArgs) -> Result<()> {
    // Validate all local arguments before any network calls.
    let from: alloy_primitives::Address = args.from.parse().wrap_err("invalid --from")?;
    let to: alloy_primitives::Address = args.to.parse().wrap_err("invalid --to")?;
    let value = parse_u256(&args.value)?;
    let data = parse_hex_bytes(&args.data)?;
    let block_id = parse_block_id(&args.block)?;
    let declared: AccessList =
        serde_json::from_str(&std::fs::read_to_string(&args.access_list)?)
            .wrap_err_with(|| format!("invalid access list in {}", args.access_list.display()))?;

    let url = Url::parse(&args.rpc_url).wrap_err("invalid RPC URL")?;
    let provider = alloy_provider::ProviderBuilder::new()
        .disable_recommended_fillers()
        .connect_http(url)
        .erased();

    let block = provider
        .get_block(block_id)
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

    let nonce = provider
        .get_transaction_count(from)
        .block_id(block_id)
        .await
        .wrap_err("failed to fetch nonce")?;

    let gas_price = block_env.basefee.max(1_000_000_000) as u128;
    let tx_env = TxEnv::builder()
        .caller(from)
        .nonce(nonce)
        .kind(TxKind::Call(to))
        .gas_limit(30_000_000)
        .gas_price(gas_price)
        .value(value)
        .data(data.into())
        .build()
        .unwrap();

    let alloy_db = revm::database::AlloyDB::new(provider, block_id);
    let async_db = revm::database_interface::WrapDatabaseAsync::new(alloy_db)
        .ok_or_else(|| eyre::eyre!("WrapDatabaseAsync requires tokio runtime"))?;
    let db = revm::database_interface::WrapDatabaseRef::from(async_db);

    let report = validate(db, tx_env, block_env, declared).wrap_err("validation failed")?;

    match args.output.as_str() {
        "json" => println!("{}", serde_json::to_string_pretty(&report)?),
        "human" => {
            if report.is_valid {
                println!("Valid: access list matches execution trace.");
            } else {
                println!("Issues found:");
                for e in &report.entries {
                    println!("  {:?}", e);
                }
                println!("Gas summary: {:?}", report.gas_summary);
            }
        }
        _ => unreachable!(),
    }
    std::process::exit(if report.is_valid { 0 } else { 1 });
}
