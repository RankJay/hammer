use clap::Parser;
use commands::{compare, generate, validate};
use eyre::Result;
use tracing_subscriber::EnvFilter;

mod commands;

#[derive(Parser)]
#[command(name = "hammer")]
#[command(about = "Hammer — EIP-2930 access list generation and validation")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Generate optimized access list for a transaction
    Generate(generate::GenerateArgs),
    /// Validate declared access list against execution trace
    Validate(validate::ValidateArgs),
    /// Compare mined transaction's access list to optimal
    Compare(compare::CompareArgs),
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("hammer=info".parse()?))
        .init();

    let cli = Cli::parse();
    match cli.command {
        Commands::Generate(args) => generate::run(args).await,
        Commands::Validate(args) => validate::run(args).await,
        Commands::Compare(args) => compare::run(args).await,
    }
}
