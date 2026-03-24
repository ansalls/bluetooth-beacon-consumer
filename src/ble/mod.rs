//! BLE collection and device processing module
//!
//! Handles Bluetooth LE adapter initialization, device discovery, and event processing.

use anyhow::{Context, Result};
use btleplug::api::{Central, Manager as _, Peripheral};
use btleplug::platform::Manager;
use chrono::Local;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::sensor::{parse_govee_data, is_govee_device, SensorReading};
use crate::storage::append_reading;

/// Initializes the BLE scanner:
/// - Sets up logging
/// - Creates output directory
/// - Prints banner
/// - Initializes BLE adapter
/// - Starts scanning for devices
///
/// Returns the adapter and event stream for processing
pub async fn initialize_ble_scanner(
    output_dir: &PathBuf,
) -> Result<(btleplug::platform::Adapter, Arc<Mutex<HashMap<String, i64>>>)> {
    // Initialize logger
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Debug)
        .try_init()
        .ok();

    // Create output directory
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("create output directory {}", output_dir.display()))?;

    // Print banner
    println!("╔══════════════════════════════════════════════════════╗");
    println!("║  Bluetooth Beacon Consumer (Govee Sensor Monitor)     ║");
    println!("║  Passively monitoring Govee sensor broadcasts        ║");
    println!("╚══════════════════════════════════════════════════════╝\n");

    // Initialize BLE manager
    let manager = Manager::new().await.context("Failed to create BLE Manager")?;

    // Get adapters
    let adapters = manager
        .adapters()
        .await
        .context("Failed to get BLE adapters")?;

    if adapters.is_empty() {
        anyhow::bail!("No BLE adapters found");
    }

    // Use first adapter
    let adapter = adapters.into_iter().next().unwrap();

    // Start scanning
    adapter
        .start_scan(btleplug::api::ScanFilter::default())
        .await
        .context("Failed to start BLE scan")?;

    // Initialize rate-limit tracker
    let last_logged = Arc::new(Mutex::new(HashMap::new()));

    Ok((adapter, last_logged))
}

/// Process a single device discovery event.
///
/// Validates device is a Govee sensor, parses sensor data, applies rate limiting,
/// and writes to CSV if valid.
pub async fn process_device_event(
    peripheral: &btleplug::platform::Peripheral,
    output_dir: &PathBuf,
    last_logged: &Arc<Mutex<HashMap<String, i64>>>,
) -> Result<Option<()>> {
    // Get device properties
    let properties = peripheral
        .properties()
        .await
        .context("Failed to get peripheral properties")?
        .context("No peripheral properties available")?;

    // Get local name
    let local_name = properties.local_name;

    // Validate it's a Govee device
    if !is_govee_device(&local_name) {
        return Ok(None);
    }

    // Parse sensor data from manufacturer data
    let (temp_c, humidity, battery) = match parse_govee_data(&properties.manufacturer_data) {
        Some(data) => data,
        None => return Ok(None),
    };

    // Calculate Fahrenheit
    let temp_f = (temp_c * 9.0 / 5.0) + 32.0;

    // Get device address
    let address = properties.address.to_string();
    let device_name = local_name.unwrap_or_else(|| "Unknown".to_string());

    // Rate limiting: only log once per 60 seconds per device
    let now = Local::now().timestamp();
    let mut last_logged_guard = last_logged.lock().await;

    let last_log_time = last_logged_guard.get(&address).copied().unwrap_or(0);
    if now - last_log_time < 60 {
        return Ok(None);
    }

    // Update the last logged time
    last_logged_guard.insert(address.clone(), now);
    drop(last_logged_guard);

    // Generate CSV filename
    let filename = format!(
        "{:04}_{:02}_{}.csv",
        Local::now().format("%Y"),
        Local::now().format("%m"),
        crate::storage::sanitize_for_filename(&format!("{}_{}", device_name, address))
    );

    let csv_path = output_dir.join(&filename);

    // Create sensor reading
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string();
    let reading = SensorReading {
        timestamp: timestamp.clone(),
        device_name: device_name.clone(),
        device_address: address.clone(),
        temperature_c: temp_c,
        temperature_f: temp_f,
        humidity,
        battery_pct: battery,
        rssi: properties.rssi,
    };

    // Write to CSV
    append_reading(&csv_path, output_dir, &reading)?;

    // Print to console
    println!(
        "[{}] {} ({}) - {:.1}°C ({:.1}°F) | {:.1}% humidity {} | {}",
        timestamp,
        device_name,
        address,
        temp_c,
        temp_f,
        humidity,
        battery.map_or(String::new(), |b| format!("| {}% battery", b)),
        filename
    );

    Ok(Some(()))
}
