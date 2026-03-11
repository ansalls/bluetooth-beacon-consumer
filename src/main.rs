use anyhow::{Context, Result};
use btleplug::api::{Central, CentralEvent, Manager as _, Peripheral, ScanFilter};
use btleplug::platform::Manager;
use chrono::{Datelike, Local};
use csv::WriterBuilder;
use flate2::{Compression, write::GzEncoder};
use fs2::FileExt;
use futures::StreamExt;
use log::{debug, info, warn};
use std::collections::HashMap;
use std::fs::{OpenOptions, create_dir_all};
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
        (
            (combined / 1000) as f64 / 10.0,
            (combined % 1000) as f64 / 10.0,
        )
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
/// We try Layout B first; if the decoded values are implausible we fall back to
/// Layout A. Raw bytes are always logged at debug level.
fn parse_govee_data(manufacturer_data: &HashMap<u16, Vec<u8>>) -> Option<(f64, f64, Option<u8>)> {
    let data = manufacturer_data.get(&GOVEE_COMPANY_ID)?;

    debug!(
        "Govee raw bytes ({}): {}",
        data.len(),
        data.iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ")
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
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Gzip-compress any CSV files in `output_dir` whose `YYYY_MM_` prefix is
/// older than 3 months from the current month. Each file is compressed
/// individually to `<name>.gz` and the original CSV is removed.
fn should_archive_file_month(
    file_year: i32,
    file_month: i32,
    now_year: i32,
    now_month: u32,
) -> bool {
    let current_month_index = now_year * 12 + now_month as i32;
    let file_month_index = file_year * 12 + file_month;
    let age_in_months = current_month_index - file_month_index;
    age_in_months > 3
}

fn archive_old_files(output_dir: &PathBuf) -> Result<()> {
    let now = Local::now();

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
        let file_year: i32 = match parts[0].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let file_month: i32 = match parts[1].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        if file_year <= 0 || !(1..=12).contains(&file_month) {
            continue;
        }

        if !should_archive_file_month(file_year, file_month, now.year(), now.month()) {
            continue;
        }

        let csv_path = entry.path();
        let gz_path = PathBuf::from(format!("{}.gz", csv_path.display()));

        let csv_data =
            std::fs::read(&csv_path).with_context(|| format!("read {}", csv_path.display()))?;

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
    let mut writer = WriterBuilder::new()
        .has_headers(false)
        .from_writer(&mut file);

    if len == 0 {
        writer.write_record([
            "timestamp",
            "device_name",
            "device_address",
            "temperature_c",
            "temperature_f",
            "humidity_pct",
            "battery_pct",
            "rssi_dbm",
        ])?;
        writer.flush()?;
        if let Err(e) = archive_old_files(output_dir) {
            warn!("Archive sweep failed: {e}");
        }
    }

    let temperature_c = format!("{:.1}", r.temperature_c);
    let temperature_f = format!("{:.1}", r.temperature_f);
    let humidity = format!("{:.1}", r.humidity);
    let battery_pct = r.battery_pct.map(|b| b.to_string()).unwrap_or_default();
    let rssi_dbm = r.rssi.map(|v| v.to_string()).unwrap_or_default();

    writer.write_record([
        r.timestamp.as_str(),
        r.device_name.as_str(),
        r.device_address.as_str(),
        temperature_c.as_str(),
        temperature_f.as_str(),
        humidity.as_str(),
        battery_pct.as_str(),
        rssi_dbm.as_str(),
    ])?;
    writer.flush()?;

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
    let last_logged: Arc<Mutex<HashMap<String, i64>>> = Arc::new(Mutex::new(HashMap::new()));

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

                let (temp_c, humidity, battery) = match parse_govee_data(&props.manufacturer_data) {
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
                let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string();

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

#[cfg(test)]
mod tests {
    use super::{
        GOVEE_COMPANY_ID, SensorReading, append_reading, archive_old_files, decode_combined,
        is_govee_device, parse_govee_data, sanitize_for_filename, should_archive_file_month,
    };
    use chrono::{Datelike, Local};
    use flate2::read::GzDecoder;
    use std::{
        collections::HashMap,
        fs::{File, create_dir_all, remove_dir_all, write},
        io::Read,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn assert_approx_eq(left: f64, right: f64) {
        assert!(
            (left - right).abs() < 0.05,
            "values were not close: left={left}, right={right}"
        );
    }

    fn encode_combined(temp_tenths_c: i32, humidity_tenths_pct: u32) -> u32 {
        if temp_tenths_c < 0 {
            let magnitude = ((-temp_tenths_c) as u32) * 1000 + humidity_tenths_pct;
            0x0100_0000 - magnitude
        } else {
            (temp_tenths_c as u32) * 1000 + humidity_tenths_pct
        }
    }

    fn combined_bytes(combined: u32) -> [u8; 3] {
        [
            ((combined >> 16) & 0xFF) as u8,
            ((combined >> 8) & 0xFF) as u8,
            (combined & 0xFF) as u8,
        ]
    }

    fn make_output_dir(test_name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "bbc_collector_{test_name}_{nonce}_{}",
            std::process::id()
        ));
        create_dir_all(&dir).expect("create temp output dir");
        dir
    }

    #[test]
    fn decode_combined_handles_positive_and_negative_temperatures() {
        let (temp_pos, hum_pos) = decode_combined(encode_combined(234, 567));
        assert_approx_eq(temp_pos, 23.4);
        assert_approx_eq(hum_pos, 56.7);

        let (temp_neg, hum_neg) = decode_combined(encode_combined(-50, 600));
        assert_approx_eq(temp_neg, -5.0);
        assert_approx_eq(hum_neg, 60.0);
    }

    #[test]
    fn parse_govee_data_prefers_layout_b_when_plausible() {
        let mut manufacturer_data = HashMap::new();
        let bytes = combined_bytes(encode_combined(234, 567));
        manufacturer_data.insert(
            GOVEE_COMPANY_ID,
            vec![0x00, bytes[0], bytes[1], bytes[2], 95],
        );

        let (temp_c, humidity, battery) =
            parse_govee_data(&manufacturer_data).expect("decode layout B");
        assert_approx_eq(temp_c, 23.4);
        assert_approx_eq(humidity, 56.7);
        assert_eq!(battery, Some(95));
    }

    #[test]
    fn parse_govee_data_filters_invalid_layout_b_battery_values() {
        let mut manufacturer_data = HashMap::new();
        let bytes = combined_bytes(encode_combined(200, 500));
        manufacturer_data.insert(
            GOVEE_COMPANY_ID,
            vec![0x00, bytes[0], bytes[1], bytes[2], 150],
        );

        let (_, _, battery) = parse_govee_data(&manufacturer_data).expect("decode layout B");
        assert_eq!(battery, None);
    }

    #[test]
    fn parse_govee_data_uses_layout_a_fallback_when_layout_b_is_implausible() {
        let mut manufacturer_data = HashMap::new();
        // Layout B decodes to 100.0C (implausible), so parser should fallback to Layout A:
        // [0..2] => 0x000F42 => 0.3C / 90.6%.
        manufacturer_data.insert(GOVEE_COMPANY_ID, vec![0x00, 0x0F, 0x42, 0x40, 90]);

        let (temp_c, humidity, battery) =
            parse_govee_data(&manufacturer_data).expect("fallback to layout A");
        assert_approx_eq(temp_c, 0.3);
        assert_approx_eq(humidity, 90.6);
        assert_eq!(battery, Some(64));
    }

    #[test]
    fn parse_govee_data_accepts_three_byte_layout_a_payloads() {
        let mut manufacturer_data = HashMap::new();
        let bytes = combined_bytes(encode_combined(215, 455));
        manufacturer_data.insert(GOVEE_COMPANY_ID, vec![bytes[0], bytes[1], bytes[2]]);

        let (temp_c, humidity, battery) =
            parse_govee_data(&manufacturer_data).expect("decode layout A");
        assert_approx_eq(temp_c, 21.5);
        assert_approx_eq(humidity, 45.5);
        assert_eq!(battery, None);
    }

    #[test]
    fn parse_govee_data_rejects_missing_or_too_short_payloads() {
        let no_govee = HashMap::new();
        assert!(parse_govee_data(&no_govee).is_none());

        let mut short = HashMap::new();
        short.insert(GOVEE_COMPANY_ID, vec![0x01, 0x02]);
        assert!(parse_govee_data(&short).is_none());
    }

    #[test]
    fn govee_name_filter_and_filename_sanitizer_behave_as_expected() {
        assert!(is_govee_device(&Some("GVH5075".to_string())));
        assert!(is_govee_device(&Some("Govee H5102".to_string())));
        assert!(!is_govee_device(&Some("OtherBrand".to_string())));
        assert!(!is_govee_device(&None));

        assert_eq!(
            sanitize_for_filename("Govee H5075/A4:C1"),
            "Govee_H5075_A4_C1"
        );
        assert_eq!(sanitize_for_filename("safe-Name123"), "safe-Name123");
    }

    #[test]
    fn archive_old_files_compresses_old_files_and_leaves_current_month() {
        let dir = make_output_dir("archive_files");
        let old_csv = dir.join("2000_01_sensor_old.csv");
        let now = Local::now();
        let current_csv = dir.join(format!(
            "{:04}_{:02}_sensor_current.csv",
            now.year(),
            now.month()
        ));

        write(&old_csv, "timestamp,device_name\n2026-03-10 00:00:00,old\n").expect("write old csv");
        write(
            &current_csv,
            "timestamp,device_name\n2026-03-10 00:00:00,current\n",
        )
        .expect("write current csv");

        archive_old_files(&dir).expect("archive sweep should succeed");

        assert!(!old_csv.exists(), "old csv should be removed");
        let old_gz = PathBuf::from(format!("{}.gz", old_csv.display()));
        assert!(old_gz.exists(), "old csv should be compressed");
        assert!(
            current_csv.exists(),
            "current-month csv should not be archived"
        );

        let mut decoded = String::new();
        let file = File::open(&old_gz).expect("open archived file");
        let mut decoder = GzDecoder::new(file);
        decoder
            .read_to_string(&mut decoded)
            .expect("decompress archived file");
        assert!(decoded.contains("timestamp,device_name"));

        remove_dir_all(dir).expect("cleanup temp dir");
    }

    #[test]
    fn append_reading_writes_header_once_and_appends_rows() {
        let dir = make_output_dir("append_reading");
        let now = Local::now();
        let path = dir.join(format!(
            "{:04}_{:02}_sensor_alpha_A4.csv",
            now.year(),
            now.month()
        ));

        let reading_one = SensorReading {
            timestamp: "2026-03-10 12:00:00.000".to_string(),
            device_name: "sensor_alpha".to_string(),
            device_address: "A4".to_string(),
            temperature_c: 21.5,
            temperature_f: 70.7,
            humidity: 45.5,
            battery_pct: Some(88),
            rssi: Some(-45),
        };
        let reading_two = SensorReading {
            timestamp: "2026-03-10 12:01:00.000".to_string(),
            device_name: "sensor_alpha".to_string(),
            device_address: "A4".to_string(),
            temperature_c: 21.6,
            temperature_f: 70.9,
            humidity: 45.6,
            battery_pct: None,
            rssi: None,
        };

        append_reading(&path, &dir, &reading_one).expect("append first reading");
        append_reading(&path, &dir, &reading_two).expect("append second reading");

        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .from_path(&path)
            .expect("open csv");
        let headers = reader.headers().expect("read headers").clone();
        assert_eq!(headers.len(), 8);
        assert_eq!(&headers[0], "timestamp");
        assert_eq!(&headers[7], "rssi_dbm");

        let rows = reader
            .records()
            .map(|r| r.expect("valid row"))
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 2, "header should only be written once");
        assert_eq!(&rows[0][0], "2026-03-10 12:00:00.000");
        assert_eq!(&rows[0][6], "88");
        assert_eq!(&rows[0][7], "-45");
        assert_eq!(&rows[1][0], "2026-03-10 12:01:00.000");
        assert_eq!(&rows[1][6], "");
        assert_eq!(&rows[1][7], "");

        remove_dir_all(dir).expect("cleanup temp dir");
    }

    #[test]
    fn archives_only_when_older_than_three_months() {
        // Current month: March 2026.
        assert!(!should_archive_file_month(2026, 3, 2026, 3)); // 0 months old
        assert!(!should_archive_file_month(2026, 1, 2026, 3)); // 2 months old
        assert!(!should_archive_file_month(2025, 12, 2026, 3)); // exactly 3 months old
        assert!(should_archive_file_month(2025, 11, 2026, 3)); // 4 months old
    }
}
