use bg_spec_core::Config;
use bg_spec_mcp::server::SpecMcp;
use bg_spec_mcp::transport::BoundedInput;
use bg_spec_store::Snapshot;
use clap::Parser;
use rmcp::ServiceExt;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    version,
    about = "Read-only Berlin Group specification evidence MCP (stdio)"
)]
struct Args {
    #[arg(long, default_value = "config/config.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    let args = Args::parse();
    let config = Config::load(&args.config)?;
    let snapshot = Snapshot::open(&config).await?;
    let server = SpecMcp::new(&snapshot, &config)
        .serve((BoundedInput::new(tokio::io::stdin()), tokio::io::stdout()))
        .await?;
    server.waiting().await?;
    Ok(())
}
