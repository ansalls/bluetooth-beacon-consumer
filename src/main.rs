use anyhow::{Context, Result};
use btleplug::api::{Central, CentralEvent, Manager as _, Peripheral, ScanFilter};
use btleplug::platform::Manager;
use chrono::{Datelike, Local};
use flate2::{write::GzEncoder, Compression};
use fs2::FileExt;
use futures::StreamExt;
use log::{debug, info, warn};
use std::collections::HashMap;
use std::fs::{create_dir_all, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Govee BLE company/manufacturer ID used in advertisement packets.
const GOVEE_COMPANY_ID: u16 = 0xEC88;

/// Minimum seconds between log entries for the same device.
const LOG_INTERVAL_SECS: i64 = 60;

/// Directory where CSV log files are written.
const OUTPUT_DIR: &str = "sensor_logs";

struct SensorReading {
    timestamp: String,
    device_name: String,
    device_address: String,
    temperature_c: f64,
    temperature_f: f64,
    humidity: f64,
    battery_pct: Option<u8>,
    rssi: Option<i16>,
}

/// Parse temperature (C), humidity (%), and optional battery (%) from Govee
/// manufacturer data.
///
/// Govee H5074 / H5075 / H5101 / H5102 / H5179 all use the same encoding:
///   bytes[0..3] -> 24-bit combined value
///   combined / 1000  -> temperature in tenths of a degree (C)
///   combined % 1000  -> relative humidity in tenths of a percent
///   If bit 23 is set the temperature is negative (below freezing).
///   bytes[3] (if present) -> battery percent 0-100
fn decode_combined(combined: u32) -> (f64, f64) {
    if combined & 0x0080_0000 != 0 {
        // Negative temperature -- two's complement around 24 bits.
        let val = 0x0100_0000u32 - combined;
        (-((val / 1000) as f64) / 10.0, (val % 1000) as f64 / 10.0)
    } else {
        ((combined / 1000) as f64 / 10.0, (combined % 1000) as f64 / 10.0)
    }
}

fn is_plausible(temp_c: f64, humidity: f64) -> bool {
    (-40.0..=85.0).contains(&temp_c) && (0.0..=100.0).contains(&humidity)
}

/// Parse temperature (C), humidity (%), and optional battery (%) from Govee
/// manufacturer data.
///
/// Govee devices encode sensor data in the manufacturer-specific advertisement
/// payload (company ID 0xEC88).  Two layouts are seen in the wild:
///
///   Layout A — 3-byte payload, bytes [0..2]:
///     combined = (b[0]<<16)|(b[1]<<8)|b[2]
///     temp_C   = (combined / 1000) / 10
///     humidity = (combined % 1000) / 10
///
///   Layout B — 5+ byte payload, bytes [1..3] are the encoded value, [4] = battery
///     (H5075, H5102, H5179 …).  Byte [0] is a flags/model byte.
///
/// We try Layout A first; if the decoded values are implausible we fall back to
/// Layout B.  Raw bytes are always logged at debug level.
fn parse_govee_data(manufacturer_data: &HashMap<u16, Vec<u8>>) -> Option<(f64, f64, Option<u8>)> {
    let data = manufacturer_data.get(&GOVEE_COMPANY_ID)?;

    debug!(
        "Govee raw bytes ({}): {}",
        data.len(),
        data.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
    );

    if data.len() < 3 {
        debug!("Govee manufacturer data too short ({} bytes)", data.len());
        return None;
    }

    // Layout B: flags/model byte at [0], combined value in bytes [1..3], battery at [4].
    // H5074, H5075, H5102, H5179, and most modern Govee sensors use this 5-6 byte format.
    // Try this first for payloads of 4+ bytes.
    if data.len() >= 4 {
        let combined_b = ((data[1] as u32) << 16) | ((data[2] as u32) << 8) | (data[3] as u32);
        let (temp_b, hum_b) = decode_combined(combined_b);
        if is_plausible(temp_b, hum_b) {
            let battery = data.get(4).filter(|&&b| b <= 100).copied();
            debug!("Layout B: temp={temp_b} hum={hum_b} battery={battery:?}");
            return Some((temp_b, hum_b, battery));
        }
    }

    // Layout A: compact 3-byte payload, combined value in bytes [0..2].
    // Fallback for older/simpler Govee devices.
    let combined_a = ((data[0] as u32) << 16) | ((data[1] as u32) << 8) | (data[2] as u32);
    let (temp_a, hum_a) = decode_combined(combined_a);
    if is_plausible(temp_a, hum_a) {
        let battery = data.get(3).filter(|&&b| b <= 100).copied();
        debug!("Layout A: temp={temp_a} hum={hum_a} battery={battery:?}");
        return Some((temp_a, hum_a, battery));
    }

    debug!("Could not decode plausible sensor values from Govee payload");
    None
}

fn is_govee_device(local_name: &Option<String>) -> bool {
    local_name
        .as_deref()
        .map(|n| n.starts_with("GVH") || n.starts_with("Govee"))
        .unwrap_or(false)
}

fn sanitize_for_filename(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
        .collect()
}

const CSV_HEADER: &str =
    "timestamp,device_name,device_address,temperature_c,temperature_f,humidity_pct,battery_pct,rssi_dbm";

/// Gzip-compress any CSV files in `output_dir` whose `YYYY_MM_` prefix is
/// more than 3 months before the current month.  Each file is compressed
/// individually to `<name>.gz` and the original CSV is removed.
fn archive_old_files(output_dir: &PathBuf) -> Result<()> {
    let now = Local::now();
    // Express the cutoff as a single comparable integer: year*12 + month.
    // "More than 3 months prior" means strictly less than (now - 3 months).
    let cutoff = now.year() as i32 * 12 + now.month() as i32 - 3;

    let entries = std::fs::read_dir(output_dir)
        .with_context(|| format!("read_dir {}", output_dir.display()))?;

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Only target uncompressed CSV files with the YYYY_MM_ prefix pattern.
        if !name_str.ends_with(".csv") {
            continue;
        }
        let parts: Vec<&str> = name_str.splitn(3, '_').collect();
        if parts.len() < 2 {
            continue;
        }
        let file_year: i32 = match parts[0].parse() { Ok(v) => v, Err(_) => continue };
        let file_month: i32 = match parts[1].parse() { Ok(v) => v, Err(_) => continue };
        if file_year <= 0 || !(1..=12).contains(&file_month) {
            continue;
        }

        if file_year * 12 + file_month > cutoff {
            continue; // Not old enough yet.
        }

        let csv_path = entry.path();
        let gz_path = PathBuf::from(format!("{}.gz", csv_path.display()));

        let csv_data = std::fs::read(&csv_path)
            .with_context(|| format!("read {}", csv_path.display()))?;

        let gz_file = std::fs::File::create(&gz_path)
            .with_context(|| format!("create {}", gz_path.display()))?;
        let mut encoder = GzEncoder::new(gz_file, Compression::best());
        encoder.write_all(&csv_data)?;
        encoder.finish()?;

        std::fs::remove_file(&csv_path)
            .with_context(|| format!("remove {}", csv_path.display()))?;

        info!(
            "Archived {} -> {}",
            csv_path.file_name().unwrap_or_default().to_string_lossy(),
            gz_path.file_name().unwrap_or_default().to_string_lossy()
        );
    }

    Ok(())
}

/// Open (or create) the CSV log file, acquire an exclusive lock, write the
/// header if the file is new (and trigger old-file archiving), then append one
/// data row.  The lock is released when the file handle is dropped, so
/// concurrent processes cannot interleave partial writes.
fn append_reading(path: &PathBuf, output_dir: &PathBuf, r: &SensorReading) -> Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open {} for append", path.display()))?;

    // Block until we hold the exclusive lock for this file.
    file.lock_exclusive()
        .with_context(|| format!("lock {}", path.display()))?;

    // If the file was just created it will be empty — write the header first,
    // then trigger archiving of old monthly files.
    let len = file.seek(SeekFrom::End(0))?;
    if len == 0 {
        writeln!(file, "{CSV_HEADER}")?;
        if let Err(e) = archive_old_files(output_dir) {
            warn!("Archive sweep failed: {e}");
        }
    }

    writeln!(
        file,
        "{},{},{},{:.1},{:.1},{:.1},{},{}",
        r.timestamp,
        r.device_name,
        r.device_address,
        r.temperature_c,
        r.temperature_f,
        r.humidity,
        r.battery_pct.map(|b| b.to_string()).unwrap_or_default(),
        r.rssi.map(|v| v.to_string()).unwrap_or_default(),
    )?;

    // Lock released here when `file` is dropped.
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // RUST_LOG=debug cargo run   -- for verbose output
    env_logger::init();

    let output_dir = PathBuf::from(OUTPUT_DIR);
    create_dir_all(&output_dir).context("Failed to create sensor_logs directory")?;

    println!("=== Govee BLE Weather Monitor ===");
    println!("Logs -> {}/", output_dir.display());
    println!("Log interval: every {LOG_INTERVAL_SECS}s per device");
    println!("Press Ctrl+C to stop.\n");

    let manager = Manager::new()
        .await
        .context("Failed to initialise BLE manager -- is Bluetooth enabled?")?;

    let adapters = manager
        .adapters()
        .await
        .context("Failed to enumerate Bluetooth adapters")?;

    let adapter = adapters
        .into_iter()
        .next()
        .context("No Bluetooth adapter found")?;

    adapter
        .start_scan(ScanFilter::default())
        .await
        .context("Failed to start BLE scan")?;

    println!("Scanning for Govee devices...\n");

    let mut events = adapter
        .events()
        .await
        .context("Failed to subscribe to BLE events")?;

    // Track timestamp of last log write per device address.
    let last_logged: Arc<Mutex<HashMap<String, i64>>> =
        Arc::new(Mutex::new(HashMap::new()));

    while let Some(event) = events.next().await {
        match event {
            CentralEvent::DeviceDiscovered(id) | CentralEvent::DeviceUpdated(id) => {
                let peripheral = match adapter.peripheral(&id).await {
                    Ok(p) => p,
                    Err(e) => {
                        debug!("peripheral({id:?}): {e}");
                        continue;
                    }
                };

                let props = match peripheral.properties().await {
                    Ok(Some(p)) => p,
                    _ => continue,
                };

                // Accept devices identified by name OR by Govee manufacturer data.
                if !is_govee_device(&props.local_name)
                    && !props.manufacturer_data.contains_key(&GOVEE_COMPANY_ID)
                {
                    continue;
                }

                let (temp_c, humidity, battery) =
                    match parse_govee_data(&props.manufacturer_data) {
                        Some(d) => d,
                        None => continue,
                    };

                let address = props.address.to_string();
                let device_name = props
                    .local_name
                    .clone()
                    .unwrap_or_else(|| format!("Govee_{}", sanitize_for_filename(&address)));
                let rssi = props.rssi;

                // Rate-limit writes so we don't create a row every ~2 s.
                let now_ts = Local::now().timestamp();
                {
                    let mut seen = last_logged.lock().await;
                    let last = seen.get(&address).copied().unwrap_or(0);
                    if now_ts - last < LOG_INTERVAL_SECS {
                        continue;
                    }
                    seen.insert(address.clone(), now_ts);
                }

                let temp_f = temp_c * 9.0 / 5.0 + 32.0;
                let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

                let reading = SensorReading {
                    timestamp: timestamp.clone(),
                    device_name: device_name.clone(),
                    device_address: address.clone(),
                    temperature_c: temp_c,
                    temperature_f: temp_f,
                    humidity,
                    battery_pct: battery,
                    rssi,
                };

                // One CSV file per device per month: YYYY_MM_<DeviceName>_<Address>.csv
                let now = Local::now();
                let filename = format!(
                    "{:04}_{:02}_{}_{}.csv",
                    now.year(),
                    now.month(),
                    sanitize_for_filename(&device_name),
                    sanitize_for_filename(&address)
                );
                let path = output_dir.join(&filename);

                if let Err(e) = append_reading(&path, &output_dir, &reading) {
                    warn!("Append failed for {filename}: {e}");
                    continue;
                }

                println!(
                    "[{timestamp}] {device_name} ({address})\n  \
                     Temp: {temp_c:.1}C / {temp_f:.1}F  \
                     Humidity: {humidity:.1}%  \
                     Battery: {}  \
                     RSSI: {} dBm\n  -> {filename}",
                    battery
                        .map(|b| format!("{b}%"))
                        .unwrap_or_else(|| "N/A".to_string()),
                    rssi.map(|v| v.to_string())
                        .unwrap_or_else(|| "N/A".to_string()),
                );
            }
            _ => {}
        }
    }

    Ok(())
}
