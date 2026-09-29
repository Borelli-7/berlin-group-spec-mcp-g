use bg_spec_core::{CatalogRepository, Config, Error, Result};
use bg_spec_store::Snapshot;
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::{path::PathBuf, process::ExitCode};

#[derive(Parser)]
#[command(
    version,
    about = "Index and inspect a local Berlin Group specification corpus"
)]
struct Args {
    #[arg(long, global = true, default_value = "config/config.toml")]
    config: PathBuf,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Index,
    Doctor,
    Stats,
    Sources,
}

async fn run(args: Args) -> Result<Value> {
    let config = Config::load(&args.config)?;
    if matches!(args.command, Command::Index) {
        return Ok(serde_json::to_value(
            bg_spec_indexer::index(&config).await?,
        )?);
    }
    let snapshot = Snapshot::open(&config).await?;
    match args.command {
        Command::Doctor => {
            let health = snapshot.doctor().await?;
            bg_spec_indexer::pipeline::verify_sources(&config, snapshot.catalog.as_ref()).await?;
            Ok(
                json!({"catalog":health,"source_hashes":"ok","search_count":snapshot.search.count()}),
            )
        }
        Command::Stats => snapshot.catalog.health().await,
        Command::Sources => Ok(json!({"sources":snapshot.catalog.documents().await?})),
        Command::Index => Err(Error::Integrity("unreachable CLI dispatch".into())),
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    match run(Args::parse()).await {
        Ok(result) => match serde_json::to_string_pretty(&result) {
            Ok(text) => {
                println!("{text}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
