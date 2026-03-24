//! Sensor data parsing and validation module
//!
//! Handles parsing of Govee BLE sensor data, decoding combined temperature/humidity values,
//! and device identification.

pub mod data;
pub mod validation;

pub use data::{SensorReading, parse_govee_data};
pub use validation::is_govee_device;
