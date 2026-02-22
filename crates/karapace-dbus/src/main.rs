use std::path::PathBuf;
use tracing::info;

fn default_store_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".local/share/karapace")
    } else {
        PathBuf::from("/tmp/karapace")
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("KARAPACE_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .without_time()
        .init();

    let store_root =
        std::env::var("KARAPACE_STORE").map_or_else(|_| default_store_path(), PathBuf::from);

    info!("karapace-dbus starting, store: {}", store_root.display());
    karapace_dbus::run_service(store_root.to_string_lossy().to_string()).await?;

    Ok(())
}
