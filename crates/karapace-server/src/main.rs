use clap::Parser;
use karapace_server::Store;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

#[derive(Parser)]
#[command(name = "karapace-server", about = "Karapace remote protocol v1 server")]
struct Cli {
    /// Port to listen on.
    #[arg(long, default_value_t = 8321)]
    port: u16,

    /// Directory to store blobs and registry data.
    #[arg(long, default_value = "./karapace-remote-data")]
    data_dir: PathBuf,
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    fs::create_dir_all(&cli.data_dir).expect("failed to create data directory");

    let addr = format!("0.0.0.0:{}", cli.port);
    info!("starting karapace-server on {addr}");
    info!("data directory: {}", cli.data_dir.display());

    let store = Arc::new(Store::new(cli.data_dir));
    karapace_server::run_server(&store, &addr);
}
