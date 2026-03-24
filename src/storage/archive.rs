//! Archive management
//!
//! Handles compression and archival of old CSV files.

use anyhow::Result;
use chrono::{Datelike, Local};
use flate2::{Compression, write::GzEncoder};
use std::fs::{File, read_dir, remove_file};
use std::io::{Write, BufReader};
use std::path::PathBuf;

/// Check if a file with the given `YYYY_MM_` prefix should be archived.
///
/// Files older than 3 months from the current date should be archived.
#[allow(dead_code)]
pub fn should_archive_file_month(
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

/// Gzip-compress any CSV files in `output_dir` whose `YYYY_MM_` prefix is
/// older than 3 months from the current month. Each file is compressed
/// individually to `<name>.gz` and the original CSV is removed.
#[allow(dead_code)]
pub fn archive_old_files(output_dir: &PathBuf) -> Result<()> {
    let now = Local::now();
    let now_year = now.year();
    let now_month = now.month();

    for entry in read_dir(output_dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = match path.file_name() {
            Some(n) => match n.to_str() {
                Some(s) => s.to_string(),
                None => continue,
            },
            None => continue,
        };

        // Only process .csv files
        if !file_name.ends_with(".csv") {
            continue;
        }

        // Extract year/month from filename like "2000_01_sensor_old.csv"
        let parts: Vec<&str> = file_name.split('_').collect();
        if parts.len() < 2 {
            continue;
        }

        let file_year: i32 = match parts[0].parse() {
            Ok(y) => y,
            Err(_) => continue,
        };

        let file_month: i32 = match parts[1].parse() {
            Ok(m) => m,
            Err(_) => continue,
        };

        // Validate year and month ranges
        if file_year <= 0 || !(1..=12).contains(&file_month) {
            continue;
        }

        // Check if should be archived
        if !should_archive_file_month(file_year, file_month, now_year, now_month) {
            continue;
        }

        // Archive the file
        let input_file = File::open(&path)?;
        let reader = BufReader::new(input_file);

        let gz_path = format!("{}.gz", path.display());
        let gz_file = File::create(&gz_path)?;
        let mut encoder = GzEncoder::new(gz_file, Compression::default());

        let mut buffer = vec![0; 8192];
        let mut reader = std::io::BufReader::new(reader);
        loop {
            let n = std::io::Read::read(&mut reader, &mut buffer)?;
            if n == 0 {
                break;
            }
            encoder.write_all(&buffer[..n])?;
        }

        encoder.finish()?;

        // Remove the original CSV file
        remove_file(&path)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{create_dir_all, remove_dir_all, write};
    use std::io::Read;
    use std::time::{SystemTime, UNIX_EPOCH};
    use flate2::read::GzDecoder;

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
    fn archive_old_files_skips_non_csv_files() {
        // Test that archive_old_files ignores non-CSV files
        let dir = make_output_dir("archive_non_csv");

        // Create various non-CSV files
        write(&dir.join("readme.txt"), "not a csv").expect("write txt");
        write(&dir.join("2026_03_data.json"), "not a csv").expect("write json");
        write(&dir.join("2000_01_old.csv"), "header\ndata").expect("write old csv");

        archive_old_files(&dir).expect("archive should succeed");

        // CSV should be archived, others should remain
        assert!(!dir.join("2000_01_old.csv").exists(), "old csv should be archived");
        assert!(
            dir.join("2000_01_old.csv.gz").exists(),
            "old csv.gz should exist"
        );
        assert!(
            dir.join("readme.txt").exists(),
            "non-csv files should remain"
        );
        assert!(
            dir.join("2026_03_data.json").exists(),
            "json files should remain"
        );

        remove_dir_all(dir).expect("cleanup temp dir");
    }

    #[test]
    fn archive_old_files_skips_invalid_filename_formats() {
        // Test that archive_old_files handles invalid filename formats gracefully
        let dir = make_output_dir("archive_invalid_format");

        // Create CSV files with invalid formats
        write(&dir.join("no_underscore.csv"), "header\ndata").expect("write");
        write(&dir.join("only_year_2026.csv"), "header\ndata").expect("write");
        write(&dir.join("2026_13_invalid_month.csv"), "header\ndata").expect("write"); // month > 12
        write(&dir.join("0_01_invalid_year.csv"), "header\ndata").expect("write"); // year <= 0
        write(&dir.join("abc_def_invalid_year.csv"), "header\ndata").expect("write"); // non-numeric

        archive_old_files(&dir).expect("archive should succeed");

        // All invalid files should remain (not archived)
        assert!(
            dir.join("no_underscore.csv").exists(),
            "invalid format should remain"
        );
        assert!(
            dir.join("only_year_2026.csv").exists(),
            "incomplete format should remain"
        );
        assert!(
            dir.join("2026_13_invalid_month.csv").exists(),
            "invalid month should remain"
        );
        assert!(
            dir.join("0_01_invalid_year.csv").exists(),
            "invalid year should remain"
        );
        assert!(
            dir.join("abc_def_invalid_year.csv").exists(),
            "non-numeric year should remain"
        );

        remove_dir_all(dir).expect("cleanup temp dir");
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
    fn archives_only_when_older_than_three_months() {
        // Current month: March 2026.
        assert!(!should_archive_file_month(2026, 3, 2026, 3)); // 0 months old
        assert!(!should_archive_file_month(2026, 1, 2026, 3)); // 2 months old
        assert!(!should_archive_file_month(2025, 12, 2026, 3)); // exactly 3 months old
        assert!(should_archive_file_month(2025, 11, 2026, 3)); // 4 months old
    }
}
