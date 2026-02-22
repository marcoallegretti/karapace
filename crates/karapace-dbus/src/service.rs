use crate::interface::{KarapaceManager, DBUS_PATH};
use thiserror::Error;
use tracing::info;
use zbus::connection::Builder;

/// Default idle timeout before the service exits (for socket activation).
const IDLE_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("DBus error: {0}")]
    Dbus(#[from] zbus::Error),
}

/// Run the D-Bus service. If `idle_timeout` is Some, the service will exit
/// after that many seconds of inactivity. Use None for infinite runtime.
pub async fn run_service(store_root: String) -> Result<(), ServiceError> {
    run_service_with_timeout(store_root, Some(IDLE_TIMEOUT_SECS)).await
}

pub async fn run_service_with_timeout(
    store_root: String,
    idle_timeout: Option<u64>,
) -> Result<(), ServiceError> {
    let manager = KarapaceManager::new(store_root);

    let _conn = Builder::session()?
        .name("org.karapace.Manager1")?
        .serve_at(DBUS_PATH, manager)?
        .build()
        .await?;

    info!("karapace-dbus service started on session bus");

    match idle_timeout {
        Some(secs) => {
            info!("idle timeout: {secs}s");
            // In a socket-activated setup, the service exits after idle timeout.
            // The D-Bus broker will restart it on next method call.
            tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
            info!("idle timeout reached, shutting down");
        }
        None => {
            // Run forever (e.g., when started manually for debugging)
            std::future::pending::<()>().await;
        }
    }

    Ok(())
}
