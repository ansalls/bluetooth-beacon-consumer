//! CSV file operations
//!
//! Handles writing sensor readings to CSV files with proper header management.

use anyhow::Result;
use fs2::FileExt;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use crate::sensor::SensorReading;

/// Sanitize a string for use as a filename, replacing invalid characters with underscores
pub fn sanitize_for_filename(s: &str) -> String {
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

/// Append a sensor reading to the CSV file for the device.
///
/// If the file doesn't exist, creates it and writes the header first.
/// If the file exists, appends the reading as a new row.
pub fn append_reading(path: &PathBuf, _output_dir: &PathBuf, r: &SensorReading) -> Result<()> {
    let file_exists = path.exists();

    // Open file in append mode
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)?;

    // Write header if file is new
    if !file_exists {
        writeln!(
            file,
            "timestamp,device_name,device_address,temperature_c,temperature_f,humidity,battery_pct,rssi_dbm"
        )?;
    }

    // Lock file during write
    file.lock_exclusive()?;

    // Append the reading
    writeln!(
        file,
        "{},{},{},{},{},{},{},{}",
        r.timestamp,
        r.device_name,
        r.device_address,
        r.temperature_c,
        r.temperature_f,
        r.humidity,
        r.battery_pct.map_or(String::new(), |b| b.to_string()),
        r.rssi.map_or(String::new(), |rssi| rssi.to_string())
    )?;

    file.unlock()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{create_dir_all, remove_dir_all};
    use std::time::{SystemTime, UNIX_EPOCH};

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
    fn append_reading_writes_header_once_and_appends_rows() {
        let dir = make_output_dir("append_reading");
        let path = dir.join("test_sensor.csv");

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
    fn sanitize_for_filename_handles_special_characters() {
        assert_eq!(
            sanitize_for_filename("Govee H5075/A4:C1"),
            "Govee_H5075_A4_C1"
        );
        assert_eq!(sanitize_for_filename("safe-Name123"), "safe-Name123");
    }
}
