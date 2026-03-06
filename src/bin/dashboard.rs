use axum::{
    extract::{Path, Query},
    http::StatusCode,
    response::Json,
    routing::get,
    Router,
};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use tower_http::{cors::CorsLayer, services::ServeDir};

const SENSOR_LOGS_DIR: &str = "sensor_logs";
const UI_DIST_DIR: &str = "ui/dist";

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
}

#[derive(Deserialize)]
struct DataParams {
    all: Option<bool>,
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

fn read_file_contents(path: &PathBuf) -> Result<String, String> {
    if path.to_string_lossy().ends_with(".gz") {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut decoder = GzDecoder::new(file);
        let mut s = String::new();
        decoder.read_to_string(&mut s).map_err(|e| e.to_string())?;
        Ok(s)
    } else {
        std::fs::read_to_string(path).map_err(|e| e.to_string())
    }
}

fn parse_csv(content: &str) -> (Option<Vec<String>>, Vec<Vec<String>>) {
    let mut lines = content.lines().filter(|l| !l.trim().is_empty());
    let header = lines
        .next()
        .map(|h| h.split(',').map(str::to_string).collect());
    let rows = lines
        .map(|l| l.split(',').map(str::to_string).collect())
        .collect();
    (header, rows)
}

fn coerce_value(s: &str, col_index: usize) -> Value {
    if col_index == 0 {
        // Always treat the first column (timestamp) as a string.
        Value::String(s.to_string())
    } else if let Ok(n) = s.parse::<f64>() {
        serde_json::json!(n)
    } else {
        Value::String(s.to_string())
    }
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn list_sensors() -> Json<Vec<SensorInfo>> {
    let dir = PathBuf::from(SENSOR_LOGS_DIR);
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
    Path(sensor_id): Path<String>,
    Query(params): Query<DataParams>,
) -> Result<Json<SensorData>, StatusCode> {
    let dir = PathBuf::from(SENSOR_LOGS_DIR);
    let mut all_files = scan_sensor_files(&dir);
    let files = all_files.remove(&sensor_id).ok_or(StatusCode::NOT_FOUND)?;

    let has_archives = !files.archived.is_empty();
    let load_all = params.all.unwrap_or(false);

    let mut paths = files.current;
    if load_all {
        paths.extend(files.archived);
        paths.sort();
    }

    let mut columns: Option<Vec<String>> = None;
    let mut all_rows: Vec<Vec<String>> = Vec::new();

    for path in &paths {
        match read_file_contents(path) {
            Ok(content) => {
                let (hdr, rows) = parse_csv(&content);
                if columns.is_none() {
                    columns = hdr;
                }
                all_rows.extend(rows);
            }
            Err(e) => eprintln!("Error reading {:?}: {}", path, e),
        }
    }

    // Sort rows by timestamp (first column).
    all_rows.sort_by(|a, b| a.first().cmp(&b.first()));

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
    }))
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/api/sensors", get(list_sensors))
        .route("/api/sensors/:id/data", get(get_sensor_data))
        .layer(CorsLayer::permissive())
        .fallback_service(
            ServeDir::new(UI_DIST_DIR).append_index_html_on_directories(true),
        );

    let addr = "127.0.0.1:3000";
    println!("Sensor Dashboard  →  http://{addr}");
    println!("Logs dir          :  {SENSOR_LOGS_DIR}/");
    println!("UI served from    :  {UI_DIST_DIR}/");
    println!();

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
