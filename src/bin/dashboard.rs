use anyhow::Context;
use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::get,
};
use chrono::{DateTime, NaiveDateTime};
use csv::ReaderBuilder;
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use tower_http::{cors::CorsLayer, services::ServeDir};

const SENSOR_LOGS_DIR: &str = "sensor_logs";
const UI_DIST_DIR: &str = "ui/dist";
const DEFAULT_ROW_LIMIT: usize = 10_000;
const MAX_ROW_LIMIT: usize = 50_000;
const MAX_DECOMPRESSED_BYTES_PER_FILE: u64 = 32 * 1024 * 1024;

#[derive(Clone)]
struct AppState {
    sensor_logs_dir: PathBuf,
}

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct SensorInfo {
    id: String,
    has_archives: bool,
}

#[derive(Serialize)]
struct SensorData {
    columns: Vec<String>,
    rows: Vec<Vec<Value>>,
    has_archives: bool,
    partial: bool,
    next_offset: Option<usize>,
    warnings: Vec<String>,
}

#[derive(Deserialize)]
struct DataParams {
    all: Option<bool>,
    since: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
}

// ── Filename helpers ──────────────────────────────────────────────────────────

/// Extract the sensor ID from a log filename.
/// Format: `YYYY_MM_<sensor_id>.csv` or `YYYY_MM_<sensor_id>.csv.gz`
fn sensor_id_from_filename(name: &str) -> Option<String> {
    let stem = name
        .strip_suffix(".csv.gz")
        .or_else(|| name.strip_suffix(".csv"))?;
    // stem = "YYYY_MM_<sensor_id>"
    let mut parts = stem.splitn(3, '_');
    let year: u32 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    if year == 0 || !(1..=12).contains(&month) {
        return None;
    }
    let sensor_id = parts.next()?.to_string();
    if sensor_id.is_empty() {
        return None;
    }
    Some(sensor_id)
}

fn is_archive_file(name: &str) -> bool {
    name.ends_with(".csv.gz")
}

// ── File scanning ─────────────────────────────────────────────────────────────

#[derive(Default)]
struct SensorFiles {
    current: Vec<PathBuf>,
    archived: Vec<PathBuf>,
}

fn scan_sensor_files(dir: &PathBuf) -> HashMap<String, SensorFiles> {
    let mut map: HashMap<String, SensorFiles> = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(id) = sensor_id_from_filename(&name) {
                let files = map.entry(id).or_default();
                let path = entry.path();
                if is_archive_file(&name) {
                    files.archived.push(path);
                } else {
                    files.current.push(path);
                }
            }
        }
    }
    for files in map.values_mut() {
        files.current.sort();
        files.archived.sort();
    }
    map
}

// ── CSV parsing ───────────────────────────────────────────────────────────────

fn normalize_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_ROW_LIMIT).clamp(1, MAX_ROW_LIMIT)
}

fn open_limited_reader(path: &PathBuf) -> Result<Box<dyn Read>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    if path.to_string_lossy().ends_with(".gz") {
        let decoder = GzDecoder::new(file);
        Ok(Box::new(decoder.take(MAX_DECOMPRESSED_BYTES_PER_FILE)))
    } else {
        Ok(Box::new(file.take(MAX_DECOMPRESSED_BYTES_PER_FILE)))
    }
}

fn for_each_csv_row<F>(path: &PathBuf, mut on_row: F) -> Result<Option<Vec<String>>, String>
where
    F: FnMut(Vec<String>) -> bool,
{
    let reader = open_limited_reader(path)?;
    let mut csv_reader = ReaderBuilder::new().has_headers(true).from_reader(reader);

    let header_record = csv_reader.headers().map_err(|e| e.to_string())?;
    let header = if header_record.is_empty() {
        None
    } else {
        Some(header_record.iter().map(str::to_string).collect::<Vec<_>>())
    };

    for record in csv_reader.records() {
        let record = record.map_err(|e| e.to_string())?;
        if record.iter().all(|field| field.trim().is_empty()) {
            continue;
        }
        let row = record.iter().map(str::to_string).collect();
        if !on_row(row) {
            break;
        }
    }

    Ok(header)
}

fn parse_sensor_timestamp(ts: &str) -> Option<i64> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(ts) {
        return Some(dt.timestamp_millis());
    }
    if let Ok(dt) = NaiveDateTime::parse_from_str(ts, "%Y-%m-%d %H:%M:%S%.f") {
        return Some(dt.and_utc().timestamp_millis());
    }
    if let Ok(dt) = NaiveDateTime::parse_from_str(ts, "%Y-%m-%d %H:%M:%S") {
        return Some(dt.and_utc().timestamp_millis());
    }
    None
}

fn coerce_value(s: &str, col_index: usize) -> Value {
    if col_index == 0 {
        // Always treat the first column (timestamp) as a string.
        Value::String(s.to_string())
    } else if let Ok(n) = s.parse::<f64>() {
        Number::from_f64(n)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(s.to_string()))
    } else {
        Value::String(s.to_string())
    }
}

fn row_is_after_or_equal_since(row: &[String], since_raw: &str, since_parsed: Option<i64>) -> bool {
    let Some(ts) = row.first() else {
        return false;
    };
    match (parse_sensor_timestamp(ts), since_parsed) {
        (Some(row_ts), Some(since_ts)) => row_ts >= since_ts,
        _ => ts.as_str() >= since_raw,
    }
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn list_sensors(State(state): State<AppState>) -> Json<Vec<SensorInfo>> {
    let dir = state.sensor_logs_dir;
    let mut sensors: Vec<SensorInfo> = scan_sensor_files(&dir)
        .into_iter()
        .map(|(id, files)| SensorInfo {
            id,
            has_archives: !files.archived.is_empty(),
        })
        .collect();
    sensors.sort_by(|a, b| a.id.cmp(&b.id));
    Json(sensors)
}

async fn get_sensor_data(
    State(state): State<AppState>,
    Path(sensor_id): Path<String>,
    Query(params): Query<DataParams>,
) -> Result<Json<SensorData>, StatusCode> {
    let dir = state.sensor_logs_dir;
    let mut all_files = scan_sensor_files(&dir);
    let files = all_files.remove(&sensor_id).ok_or(StatusCode::NOT_FOUND)?;

    let has_archives = !files.archived.is_empty();
    let load_all = params.all.unwrap_or(false);
    let since = params.since.as_deref();
    let since_parsed = since.and_then(parse_sensor_timestamp);
    let limit = normalize_limit(params.limit);
    let offset = params.offset.unwrap_or(0);

    let mut paths = files.current;
    if load_all {
        paths.extend(files.archived);
        paths.sort();
    }

    let mut columns: Option<Vec<String>> = None;
    let mut all_rows: Vec<Vec<String>> = Vec::with_capacity(limit.saturating_add(1));
    let mut warnings = Vec::new();
    let mut partial = false;
    let mut matched_count = 0usize;
    let target_count = offset.saturating_add(limit).saturating_add(1);

    'scan: for path in &paths {
        let header = for_each_csv_row(path, |row| {
            if let Some(since_raw) = since {
                if !row_is_after_or_equal_since(&row, since_raw, since_parsed) {
                    return true;
                }
            }

            matched_count = matched_count.saturating_add(1);
            if matched_count <= offset {
                return true;
            }

            all_rows.push(row);
            all_rows.len() < target_count
        });

        match header {
            Ok(hdr) => {
                if columns.is_none() {
                    columns = hdr;
                }
            }
            Err(e) => {
                partial = true;
                warnings.push(format!("Could not read {}: {e}", path.display()));
            }
        }

        if all_rows.len() >= target_count {
            break 'scan;
        }
    }

    let has_more = all_rows.len() > limit;
    if has_more {
        all_rows.truncate(limit);
        partial = true;
        warnings.push(format!(
            "Result truncated to {limit} rows; fetch next page with offset={}",
            offset.saturating_add(limit)
        ));
    }

    let columns = columns.unwrap_or_default();
    let rows: Vec<Vec<Value>> = all_rows
        .into_iter()
        .map(|row| {
            row.iter()
                .enumerate()
                .map(|(i, val)| coerce_value(val, i))
                .collect()
        })
        .collect();

    Ok(Json(SensorData {
        columns,
        rows,
        has_archives,
        partial,
        next_offset: has_more.then_some(offset.saturating_add(limit)),
        warnings,
    }))
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let app = build_app(PathBuf::from(SENSOR_LOGS_DIR));

    let addr = "127.0.0.1:3000";
    println!("Sensor Dashboard  →  http://{addr}");
    println!("Logs dir          :  {SENSOR_LOGS_DIR}/");
    println!("UI served from    :  {UI_DIST_DIR}/");
    println!();

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind dashboard on {addr}"))?;
    axum::serve(listener, app)
        .await
        .context("dashboard server exited unexpectedly")?;
    Ok(())
}

fn build_app(sensor_logs_dir: PathBuf) -> Router {
    let state = AppState { sensor_logs_dir };
    Router::new()
        .route("/api/sensors", get(list_sensors))
        .route("/api/sensors/:id/data", get(get_sensor_data))
        .layer(CorsLayer::permissive())
        .fallback_service(ServeDir::new(UI_DIST_DIR).append_index_html_on_directories(true))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::{normalize_limit, parse_sensor_timestamp, row_is_after_or_equal_since};
    use crate::build_app;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use flate2::{Compression, write::GzEncoder};
    use std::{
        fs::{File, create_dir_all, remove_dir_all, write},
        io::Write,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };
    use tower::ServiceExt;

    fn make_logs_dir(test_name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "bbc_dashboard_{test_name}_{nonce}_{}",
            std::process::id()
        ));
        create_dir_all(&dir).expect("create temp sensor_logs dir");
        dir
    }

    async fn get_json(app: axum::Router, path: &str) -> serde_json::Value {
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(path)
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("execute request");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        serde_json::from_slice(&body).expect("parse json")
    }

    async fn get_status(app: axum::Router, path: &str) -> StatusCode {
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(path)
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("execute request");
        response.status()
    }

    fn write_gz(path: PathBuf, content: &str) {
        let file = File::create(path).expect("create gz file");
        let mut encoder = GzEncoder::new(file, Compression::default());
        encoder
            .write_all(content.as_bytes())
            .expect("write gz content");
        encoder.finish().expect("finish gzip stream");
    }

    #[test]
    fn parse_sensor_timestamp_accepts_legacy_and_rfc3339() {
        assert!(parse_sensor_timestamp("2026-03-10 12:00:00").is_some());
        assert!(parse_sensor_timestamp("2026-03-10 12:00:00.123").is_some());
        assert!(parse_sensor_timestamp("2026-03-10T12:00:00Z").is_some());
    }

    #[test]
    fn since_filter_is_inclusive_to_avoid_missing_same_second_rows() {
        let older = vec!["2026-03-10 11:59:59".to_string()];
        let equal = vec!["2026-03-10 12:00:00".to_string()];
        let newer = vec!["2026-03-10 12:00:01".to_string()];
        let since = "2026-03-10 12:00:00";
        let since_parsed = parse_sensor_timestamp(since);

        assert!(!row_is_after_or_equal_since(&older, since, since_parsed));
        assert!(row_is_after_or_equal_since(&equal, since, since_parsed));
        assert!(row_is_after_or_equal_since(&newer, since, since_parsed));
    }

    #[test]
    fn limit_is_clamped_for_safety() {
        assert_eq!(normalize_limit(None), 10_000);
        assert_eq!(normalize_limit(Some(0)), 1);
        assert_eq!(normalize_limit(Some(5)), 5);
        assert_eq!(normalize_limit(Some(100_000)), 50_000);
    }

    #[tokio::test]
    async fn api_list_sensors_returns_archive_flags() {
        let logs_dir = make_logs_dir("list_sensors");
        write(
            logs_dir.join("2026_03_sensor_alpha.csv"),
            "timestamp,device_name\n2026-03-10 10:00:00,alpha\n",
        )
        .expect("write current sensor file");
        write(logs_dir.join("2025_10_sensor_alpha.csv.gz"), b"not-used")
            .expect("write archive marker");
        write(
            logs_dir.join("2026_03_sensor_beta.csv"),
            "timestamp,device_name\n2026-03-10 10:00:00,beta\n",
        )
        .expect("write second sensor file");

        let value = get_json(build_app(logs_dir.clone()), "/api/sensors").await;
        let sensors = value.as_array().expect("array response");
        assert_eq!(sensors.len(), 2);
        assert_eq!(sensors[0]["id"], "sensor_alpha");
        assert_eq!(sensors[0]["has_archives"], true);
        assert_eq!(sensors[1]["id"], "sensor_beta");
        assert_eq!(sensors[1]["has_archives"], false);

        remove_dir_all(logs_dir).expect("cleanup temp dir");
    }

    #[tokio::test]
    async fn api_sensor_data_limit_offset_and_nan_are_safe() {
        let logs_dir = make_logs_dir("paged_nan");
        write(
            logs_dir.join("2026_03_sensor_alpha.csv"),
            concat!(
                "timestamp,device_name,device_address,temperature_c,temperature_f,humidity_pct,battery_pct,rssi_dbm\n",
                "2026-03-10 12:00:00.000,alpha,A4,20.1,68.2,40.0,99,-55\n",
                "2026-03-10 12:00:01.000,alpha,A4,20.2,68.4,NaN,99,-54\n",
                "2026-03-10 12:00:02.000,alpha,A4,20.3,68.5,42.0,99,-53\n"
            ),
        )
        .expect("write sensor csv");

        let first_page = get_json(
            build_app(logs_dir.clone()),
            "/api/sensors/sensor_alpha/data?limit=2&offset=0",
        )
        .await;
        assert_eq!(first_page["rows"].as_array().expect("rows").len(), 2);
        assert_eq!(first_page["partial"], true);
        assert_eq!(first_page["next_offset"], 2);
        assert!(
            first_page["warnings"]
                .as_array()
                .expect("warnings")
                .iter()
                .any(|v| v.as_str().unwrap_or_default().contains("truncated"))
        );

        let columns = first_page["columns"]
            .as_array()
            .expect("columns")
            .iter()
            .map(|v| v.as_str().unwrap_or_default())
            .collect::<Vec<_>>();
        let humidity_idx = columns
            .iter()
            .position(|c| *c == "humidity_pct")
            .expect("humidity column");
        let second_row = first_page["rows"][1].as_array().expect("row");
        assert_eq!(second_row[humidity_idx], "NaN");

        let second_page = get_json(
            build_app(logs_dir.clone()),
            "/api/sensors/sensor_alpha/data?limit=2&offset=2",
        )
        .await;
        assert_eq!(second_page["rows"].as_array().expect("rows").len(), 1);
        assert_eq!(second_page["partial"], false);
        assert!(second_page["next_offset"].is_null());

        remove_dir_all(logs_dir).expect("cleanup temp dir");
    }

    #[tokio::test]
    async fn api_since_filter_is_inclusive_and_surfaces_partial_read_warnings() {
        let logs_dir = make_logs_dir("since_partial");
        write(
            logs_dir.join("2025_12_sensor_alpha.csv"),
            "timestamp,device_name,humidity_pct\n\"bad",
        )
        .expect("write malformed csv");
        write(
            logs_dir.join("2026_03_sensor_alpha.csv"),
            concat!(
                "timestamp,device_name,device_address,temperature_c,temperature_f,humidity_pct,battery_pct,rssi_dbm\n",
                "2026-03-10 12:00:00.000,alpha,A4,20.0,68.0,40.0,90,-60\n",
                "2026-03-10 12:00:01.000,alpha,A4,20.1,68.2,41.0,90,-59\n"
            ),
        )
        .expect("write valid csv");

        let value = get_json(
            build_app(logs_dir.clone()),
            "/api/sensors/sensor_alpha/data?all=true&since=2026-03-10%2012:00:00.000&limit=50",
        )
        .await;

        let rows = value["rows"].as_array().expect("rows");
        assert_eq!(rows.len(), 2);
        assert_eq!(value["partial"], true);
        assert!(
            value["warnings"]
                .as_array()
                .expect("warnings")
                .iter()
                .any(|v| v.as_str().unwrap_or_default().contains("Could not read"))
        );
        let first_ts = rows[0].as_array().expect("row")[0]
            .as_str()
            .expect("timestamp");
        assert_eq!(first_ts, "2026-03-10 12:00:00.000");

        remove_dir_all(logs_dir).expect("cleanup temp dir");
    }

    #[tokio::test]
    async fn api_returns_not_found_for_unknown_sensor() {
        let logs_dir = make_logs_dir("not_found");
        let status = get_status(build_app(logs_dir.clone()), "/api/sensors/missing/data").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        remove_dir_all(logs_dir).expect("cleanup temp dir");
    }

    #[tokio::test]
    async fn api_all_flag_controls_archive_inclusion() {
        let logs_dir = make_logs_dir("all_flag");
        write(
            logs_dir.join("2026_03_sensor_alpha.csv"),
            concat!(
                "timestamp,device_name,device_address,temperature_c,temperature_f,humidity_pct,battery_pct,rssi_dbm\n",
                "2026-03-10 12:00:00.000,alpha,A4,20.0,68.0,40.0,90,-60\n"
            ),
        )
        .expect("write current csv");

        write_gz(
            logs_dir.join("2025_12_sensor_alpha.csv.gz"),
            concat!(
                "timestamp,device_name,device_address,temperature_c,temperature_f,humidity_pct,battery_pct,rssi_dbm\n",
                "2025-12-10 12:00:00.000,alpha,A4,18.0,64.4,45.0,91,-61\n"
            ),
        );

        let current_only = get_json(
            build_app(logs_dir.clone()),
            "/api/sensors/sensor_alpha/data?limit=50",
        )
        .await;
        assert_eq!(current_only["rows"].as_array().expect("rows").len(), 1);
        assert_eq!(current_only["has_archives"], true);

        let all_history = get_json(
            build_app(logs_dir.clone()),
            "/api/sensors/sensor_alpha/data?all=true&limit=50",
        )
        .await;
        assert_eq!(all_history["rows"].as_array().expect("rows").len(), 2);
        assert_eq!(all_history["partial"], false);

        remove_dir_all(logs_dir).expect("cleanup temp dir");
    }
}
