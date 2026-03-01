use alloy_eips::BlockId;
use alloy_primitives::U256;
use alloy_provider::Provider;
use alloy_rpc_types_eth::TransactionRequest;
use clap::Args;
use eyre::{Context, Result};
use hammer_core::{access_list_gas_cost, generate};
use reqwest::Url;
use revm::context::{BlockEnv, TxEnv};
use revm::primitives::TxKind;

use super::util::{assert_post_berlin, parse_block_id, parse_hex_bytes, parse_u256};

#[derive(Args)]
pub struct GenerateArgs {
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
    #[arg(long, default_value = "latest")]
    pub block: String,
    #[arg(long, default_value = "json", value_parser = ["json", "human"])]
    pub output: String,
}

pub async fn run(args: GenerateArgs) -> Result<()> {
    // Validate all local arguments before any network calls.
    let from: alloy_primitives::Address = args.from.parse().wrap_err("invalid --from")?;
    let to: alloy_primitives::Address = args.to.parse().wrap_err("invalid --to")?;
    let value = parse_u256(&args.value)?;
    let data = parse_hex_bytes(&args.data)?;
    let block_id = parse_block_id(&args.block)?;

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
        .data(data.clone().into())
        .build()
        .unwrap();

    let tx_req = TransactionRequest {
        from: Some(from),
        to: Some(TxKind::Call(to)),
        value: Some(value),
        input: alloy_rpc_types_eth::TransactionInput::new(data.into()),
        gas: Some(30_000_000),
        ..Default::default()
    };

    let state_block_id = BlockId::hash(header.hash);

    let db = super::prefetch::build(
        provider,
        state_block_id,
        state_block_id,
        tx_req,
        &alloy_rpc_types_eth::AccessList::default(),
    )
    .await
    .wrap_err("prefetch failed")?;

    let result = generate(db, tx_env, block_env).wrap_err("access list generation failed")?;

    match args.output.as_str() {
        "json" => {
            let out = serde_json::json!({
                "access_list": result.optimized.list,
                "decision": result.decision
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        "human" => {
            let cost = access_list_gas_cost(&result.optimized.list);
            let d = &result.decision;
            match d.recommendation {
                hammer_core::Recommendation::Attach => {
                    println!(
                        "Recommendation: attach (saves {} gas | {} addresses, {} slots)",
                        d.net_gas_delta, d.cold_addresses, d.cold_slots
                    );
                }
                hammer_core::Recommendation::Skip => {
                    println!("Recommendation: skip (list would cost more than it saves)");
                }
            }
            println!("Access list (gas cost: {}):", cost);
            for item in &result.optimized.list.0 {
                println!("  {}:", item.address);
                for key in &item.storage_keys {
                    println!("    - {}", key);
                }
            }
            if !result.optimized.removed_addresses.is_empty() {
                println!("Removed (warm): {:?}", result.optimized.removed_addresses);
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}
