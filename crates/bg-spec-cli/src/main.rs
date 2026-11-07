use anyhow::Context;
use bg_spec_core::{CatalogRepository, Config};
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

async fn open_snapshot(config: &Config) -> anyhow::Result<Snapshot> {
    Snapshot::open(config)
        .await
        .context("failed to open the active index snapshot")
}

async fn run(args: Args) -> anyhow::Result<Value> {
    let config = Config::load(&args.config).context("failed to load configuration")?;
    match args.command {
        Command::Index => {
            let report = bg_spec_indexer::index(&config)
                .await
                .context("failed to index corpus")?;
            Ok(serde_json::to_value(report)?)
        }
        Command::Doctor => {
            let snapshot = open_snapshot(&config).await?;
            let health = snapshot
                .doctor()
                .await
                .context("failed to verify index integrity")?;
            bg_spec_indexer::pipeline::verify_sources(&config, snapshot.catalog.as_ref())
                .await
                .context("failed to verify corpus sources")?;
            Ok(
                json!({"catalog":health,"source_hashes":"ok","search_count":snapshot.search.count()}),
            )
        }
        Command::Stats => Ok(open_snapshot(&config).await?.catalog.health().await?),
        Command::Sources => {
            let snapshot = open_snapshot(&config).await?;
            Ok(json!({"sources":snapshot.catalog.documents().await?}))
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    let output = run(Args::parse())
        .await
        .and_then(|result| Ok(serde_json::to_string_pretty(&result)?));
    match output {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error:?}");
            ExitCode::FAILURE
        }
    }
}
