//! Bluetooth Beacon Consumer library
//!
//! A Rust library for passively monitoring Govee sensor broadcasts via Bluetooth LE,
//! parsing sensor data, and persisting readings to CSV files with automatic archival.

pub mod sensor;
pub mod storage;
pub mod ble;

// Re-export key public APIs for convenient access
pub use sensor::{SensorReading, parse_govee_data, is_govee_device};
pub use storage::{append_reading, sanitize_for_filename, archive_old_files};
pub use ble::{initialize_ble_scanner, process_device_event};
