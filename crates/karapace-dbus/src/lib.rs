//! D-Bus desktop integration service for Karapace.
//!
//! This crate exposes the Karapace engine over the `org.karapace.Manager1` D-Bus
//! interface, enabling desktop applications and system services to build, destroy,
//! enter, and query environments without invoking the CLI directly. Designed for
//! socket activation with an idle timeout.

pub mod interface;
pub mod service;

pub use interface::{KarapaceManager, API_VERSION, DBUS_INTERFACE, DBUS_PATH};
pub use service::{run_service, run_service_with_timeout, ServiceError};
