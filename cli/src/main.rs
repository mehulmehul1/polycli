#![allow(clippy::items_after_statements)]

mod auth;
mod commands;
mod config;
mod output;

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use output::OutputFormat;
use polymarket_client_sdk::{bridge, data, gamma};

#[derive(Parser)]
#[command(name = "polymarket", about = "Polymarket CLI", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Output format: table or json
    #[arg(long, global = true, default_value = "table")]
    output: OutputFormat,

    /// Private key for wallet authentication (overrides env var and config file)
    #[arg(long, global = true)]
    private_key: Option<String>,

    /// Signature type: eoa, proxy, or gnosis-safe (default: proxy)
    #[arg(long, global = true)]
    signature_type: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Interact with markets
    Markets(commands::markets::MarketsArgs),
    /// Interact with events
    Events(commands::events::EventsArgs),
    /// Interact with tags
    Tags(commands::tags::TagsArgs),
    /// Interact with series
    Series(commands::series::SeriesArgs),
    /// Interact with comments
    Comments(commands::comments::CommentsArgs),
    /// Look up public profiles
    Profiles(commands::profiles::ProfilesArgs),
    /// Sports metadata and teams
    Sports(commands::sports::SportsArgs),
    /// Interact with the CLOB (order book, trading, balances)
    Clob(commands::clob::ClobArgs),
    /// Query on-chain data (positions, trades, leaderboards)
    Data(commands::data::DataArgs),
    /// Bridge assets from other chains to Polymarket
    Bridge(commands::bridge::BridgeArgs),
    /// Manage wallet and authentication
    Wallet(commands::wallet::WalletArgs),
    /// Check API health status
    Status,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let output = cli.output.clone();

    if let Err(e) = run(cli).await {
        match output {
            OutputFormat::Json => {
                println!("{}", serde_json::json!({"error": e.to_string()}));
            }
            OutputFormat::Table => {
                eprintln!("Error: {e}");
            }
        }
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let gamma_client = gamma::Client::default();
    let data_client = data::Client::default();
    let bridge_client = bridge::Client::default();

    match cli.command {
        Commands::Markets(args) => commands::markets::execute(&gamma_client, args, cli.output).await,
        Commands::Events(args) => commands::events::execute(&gamma_client, args, cli.output).await,
        Commands::Tags(args) => commands::tags::execute(&gamma_client, args, cli.output).await,
        Commands::Series(args) => commands::series::execute(&gamma_client, args, cli.output).await,
        Commands::Comments(args) => commands::comments::execute(&gamma_client, args, cli.output).await,
        Commands::Profiles(args) => commands::profiles::execute(&gamma_client, args, cli.output).await,
        Commands::Sports(args) => commands::sports::execute(&gamma_client, args, cli.output).await,
        Commands::Clob(args) => {
            commands::clob::execute(
                args,
                cli.output,
                cli.private_key.as_deref(),
                cli.signature_type.as_deref(),
            )
            .await
        }
        Commands::Data(args) => commands::data::execute(&data_client, args, cli.output).await,
        Commands::Bridge(args) => commands::bridge::execute(&bridge_client, args, cli.output).await,
        Commands::Wallet(args) => commands::wallet::execute(args, cli.output, cli.private_key.as_deref()),
        Commands::Status => {
            let status = gamma_client.status().await?;
            match cli.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::json!({"status": status}));
                }
                OutputFormat::Table => {
                    println!("API Status: {status}");
                }
            }
            Ok(())
        }
    }
}
